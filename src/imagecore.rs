//! Port of QVImageCore: file loading, folder listing & sorting, preloading cache, animation.
//!
//! Methods never call back into the window directly; they queue `CoreEvent`s which the window
//! drains afterwards. That keeps RefCell borrows short and mirrors Qt's signal emission order.

use crate::app;
use crate::decode::{self, ErrorData, Frame, RawImage, ReadData, TargetColorSpace};
use crate::settings::Setting;
use crate::util;
use gdk::prelude::*;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CoreEvent {
    FileChanged,
    UpdateLoadedPixmapItem,
    AnimatedFrameChanged,
}

pub enum LoaderMsg {
    Loaded(ReadData),
    Preloaded(ReadData),
}

#[derive(Clone, Debug)]
pub struct CompatibleFile {
    pub absolute_file_path: String,
    pub file_name: String,
    pub last_modified: i64,
    pub last_created: i64,
    pub size: i64,
    pub mime_type: String,
}

#[derive(Clone, Default)]
pub struct FileDetails {
    /// absolute file path of the loaded/requested file ("" = none)
    pub file_path: String,
    pub folder_file_info_list: Vec<CompatibleFile>,
    pub loaded_index_in_folder: i32,
    pub is_load_requested: bool,
    pub is_pixmap_loaded: bool,
    pub is_movie_loaded: bool,
    pub base_image_size: (i32, i32),
    pub loaded_pixmap_size: (i32, i32),
    pub time_since_loaded: Option<Instant>,
    pub error_data: ErrorData,
}

impl FileDetails {
    pub fn update_loaded_index_in_folder(&mut self) {
        self.loaded_index_in_folder = -1;
        if self.file_path.is_empty() {
            return;
        }
        for (i, f) in self.folder_file_info_list.iter().enumerate() {
            if f.absolute_file_path == self.file_path {
                self.loaded_index_in_folder = i as i32;
                return;
            }
        }
    }
    pub fn is_file(&self) -> bool {
        !self.file_path.is_empty() && std::path::Path::new(&self.file_path).is_file()
    }
    pub fn file_name(&self) -> String {
        util::file_name(&self.file_path)
    }
    pub fn file_size(&self) -> u64 {
        std::fs::metadata(&self.file_path).map(|m| m.len()).unwrap_or(0)
    }
}

#[derive(PartialEq, Clone, Default)]
struct DirInfo {
    dir_path: String,
    file_count: usize,
    sort_mode: i32,
    sort_descending: bool,
}

static GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
fn next_gen() -> u64 {
    GEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

pub struct Movie {
    pub frames: Arc<Vec<Frame>>,
    pub current: usize,
    pub speed: i32,
    pub paused: bool,
    pub running: bool,
    pub generation: u64,
}

impl Movie {
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }
    pub fn current_image(&self) -> &RawImage {
        &self.frames[self.current].image
    }
}

// ---- process-wide image cache (QCache<QString, ReadData>) -----------------------------------

struct Cache {
    entries: VecDeque<(String, ReadData, usize)>, // front = most recently used
    max_cost: usize,
    total: usize,
}

impl Cache {
    fn trim(&mut self, limit: usize) {
        while self.total > limit {
            match self.entries.pop_back() {
                Some((_, _, c)) => self.total -= c,
                None => break,
            }
        }
    }
    fn set_max_cost(&mut self, c: usize) {
        self.max_cost = c;
        self.trim(c);
    }
    fn take(&mut self, key: &str) -> Option<ReadData> {
        let pos = self.entries.iter().position(|(k, _, _)| k == key)?;
        let (_, rd, c) = self.entries.remove(pos)?;
        self.total -= c;
        Some(rd)
    }
    fn contains(&self, key: &str) -> bool {
        self.entries.iter().any(|(k, _, _)| k == key)
    }
    fn insert(&mut self, key: String, rd: ReadData, cost: usize) {
        if let Some(pos) = self.entries.iter().position(|(k, _, _)| *k == key) {
            let (_, _, c) = self.entries.remove(pos).unwrap();
            self.total -= c;
        }
        if cost > self.max_cost {
            return;
        }
        self.trim(self.max_cost - cost);
        self.total += cost;
        self.entries.push_front((key, rd, cost));
    }
    fn clear(&mut self) {
        self.entries.clear();
        self.total = 0;
    }
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::new(Cache { entries: VecDeque::new(), max_cost: 100 * 1024, total: 0 });
}

pub fn cache_key(path: &str, size: u64, target: TargetColorSpace) -> String {
    format!("{}\n{}\n{:?}", path, size, target)
}

pub struct ImageCore {
    pub loaded_pixmap: Option<cairo::ImageSurface>,
    /// Pixels behind loaded_pixmap (shared, zero-copy)
    pub loaded_raw: Option<RawImage>,
    pub movie: Option<Movie>,
    pub details: FileDetails,
    pub current_rotation: i32,
    last_dir_info: DirInfo,
    random_sort_seed: u64,
    last_files_preloaded: Vec<String>,
    preload_files_in_progress: Vec<String>,
    waiting_on_preload_file: String,
    largest_dimension: i32,
    waiting_on_load: bool,
    pub pending: Vec<CoreEvent>,
    sender: async_channel::Sender<LoaderMsg>,
}

/// Wraps shared pixel data so Cairo can use it without copying. Cairo only ever reads
/// these surfaces (they are used as paint sources), so handing out `&mut` is never acted on.
struct SharedBuf(Arc<Vec<u8>>);
impl AsMut<[u8]> for SharedBuf {
    fn as_mut(&mut self) -> &mut [u8] {
        let v: &Vec<u8> = &self.0;
        unsafe { std::slice::from_raw_parts_mut(v.as_ptr() as *mut u8, v.len()) }
    }
}

/// Zero-copy Cairo surface over a RawImage
pub fn surface_from_raw(img: &RawImage) -> Option<cairo::ImageSurface> {
    cairo::ImageSurface::create_for_data(SharedBuf(img.data.clone()), cairo::Format::ARgb32, img.width, img.height, img.width * 4).ok()
}

impl ImageCore {
    pub fn new(sender: async_channel::Sender<LoaderMsg>) -> Self {
        let mut largest = 0;
        if let Some(display) = gdk::Display::default() {
            for i in 0..display.n_monitors() {
                if let Some(m) = display.monitor(i) {
                    let g = m.geometry();
                    let s = m.scale_factor();
                    largest = largest.max(g.width().max(g.height()) * s);
                }
            }
        }
        if largest == 0 {
            largest = 1920;
        }
        let mut core = ImageCore {
            loaded_pixmap: None,
            loaded_raw: None,
            movie: None,
            details: FileDetails { loaded_index_in_folder: -1, ..Default::default() },
            current_rotation: 0,
            last_dir_info: DirInfo::default(),
            random_sort_seed: 0,
            last_files_preloaded: vec![],
            preload_files_in_progress: vec![],
            waiting_on_preload_file: String::new(),
            largest_dimension: largest,
            waiting_on_load: false,
            pending: vec![],
            sender,
        };
        core.settings_updated();
        core
    }

    fn emit(&mut self, e: CoreEvent) {
        self.pending.push(e);
    }

    pub fn load_file(&mut self, file_name: &str, is_reloading: bool) {
        if self.waiting_on_load {
            return;
        }
        let sanitary = util::url_to_local(file_name).unwrap_or_else(|| file_name.to_string());
        let path = util::absolute_clean(&sanitary);

        if std::path::Path::new(&path).is_dir() {
            self.update_folder_info(Some(path));
            if self.details.folder_file_info_list.is_empty() {
                self.close_image();
            } else {
                let first = self.details.folder_file_info_list[0].absolute_file_path.clone();
                self.load_file(&first, false);
            }
            return;
        }

        // Pause playing movie because it feels better that way
        self.set_paused(true);

        self.details.is_load_requested = true;
        self.waiting_on_load = true;

        let target = self.get_target_color_space();
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let key = cache_key(&path, size, target);

        let cached = if is_reloading { None } else { CACHE.with(|c| c.borrow_mut().take(&key)) };
        if let Some(rd) = cached {
            self.load_pixmap(rd);
        } else if self.preload_files_in_progress.contains(&path) {
            self.waiting_on_preload_file = path;
        } else {
            self.spawn_read(path, target, false);
        }
    }

    fn spawn_read(&self, path: String, target: TargetColorSpace, preload: bool) {
        let sender = self.sender.clone();
        let largest = self.largest_dimension;
        std::thread::spawn(move || {
            let ctx = decode::ColorContext { target };
            let rd = decode::read_file(&path, &ctx, largest);
            let msg = if preload { LoaderMsg::Preloaded(rd) } else { LoaderMsg::Loaded(rd) };
            let _ = sender.send_blocking(msg);
        });
    }

    pub fn on_loader_msg(&mut self, msg: LoaderMsg) {
        match msg {
            LoaderMsg::Loaded(rd) => self.load_pixmap(rd),
            LoaderMsg::Preloaded(rd) => {
                let path = rd.absolute_file_path.clone();
                if self.waiting_on_preload_file == path {
                    self.waiting_on_preload_file.clear();
                    self.load_pixmap(rd);
                } else {
                    Self::add_to_cache(rd);
                }
                self.preload_files_in_progress.retain(|p| *p != path);
            }
        }
    }

    pub fn load_pixmap(&mut self, rd: ReadData) {
        let _t = crate::util::Trace::new("load_pixmap");
        if rd.error_data.has_error {
            self.details = self.get_empty_file_details();
            self.details.error_data = rd.error_data.clone();
        } else {
            self.details.error_data = ErrorData::default();
        }

        // Do this first so we can keep folder info even when loading errored files
        self.details.file_path = rd.absolute_file_path.clone();
        self.details.update_loaded_index_in_folder();
        if self.details.loaded_index_in_folder == -1 {
            self.update_folder_info(None);
        }

        // Reset mechanism to avoid stalling while loading
        self.waiting_on_load = false;

        if self.details.error_data.has_error {
            self.load_empty_pixmap();
            return;
        }

        let Some(image) = rd.image.as_ref() else {
            self.load_empty_pixmap();
            return;
        };
        let raw = if self.current_rotation != 0 { image.rotated(self.current_rotation) } else { image.clone() };
        let Some(surf) = surface_from_raw(&raw) else {
            self.details.error_data = ErrorData { has_error: true, error_num: 0, error_string: "Unknown error".into() };
            self.load_empty_pixmap();
            return;
        };

        self.details.is_pixmap_loaded = true;
        self.details.base_image_size = rd.image_size;
        self.details.loaded_pixmap_size = (surf.width(), surf.height());
        if self.details.base_image_size == (-1, -1) {
            self.details.base_image_size = self.details.loaded_pixmap_size;
        }
        self.loaded_pixmap = Some(surf);
        self.loaded_raw = Some(raw);

        // Animation detection
        self.stop_movie();
        if let Some(frames) = rd.frames.clone() {
            self.movie = Some(Movie { frames, current: 0, speed: 100, paused: false, running: true, generation: next_gen() });
        }
        self.details.is_movie_loaded = self.movie.as_ref().map(|m| m.running).unwrap_or(false);

        Self::add_to_cache(rd);

        self.details.time_since_loaded = Some(Instant::now());

        self.emit(CoreEvent::FileChanged);

        self.request_caching();
    }

    pub fn close_image(&mut self) {
        self.details = self.get_empty_file_details();
        self.load_empty_pixmap();
    }

    fn load_empty_pixmap(&mut self) {
        self.loaded_pixmap = None;
        self.loaded_raw = None;
        self.stop_movie();
        self.emit(CoreEvent::FileChanged);
    }

    fn stop_movie(&mut self) {
        self.movie = None;
    }

    fn get_empty_file_details(&self) -> FileDetails {
        FileDetails {
            file_path: String::new(),
            folder_file_info_list: self.details.folder_file_info_list.clone(),
            loaded_index_in_folder: self.details.loaded_index_in_folder,
            ..Default::default()
        }
    }

    pub fn get_compatible_files(dir_path: &str) -> Vec<CompatibleFile> {
        let sm = app::settings();
        let sort_mode = sm.int(Setting::SortMode);
        let allow_content = sm.bool(Setting::AllowMimeContentDetection);
        let skip_hidden = sm.bool(Setting::SkipHidden);
        drop(sm);
        let (extensions, mime_types) = app::format_lists();

        let mut list = vec![];
        let Ok(rd) = std::fs::read_dir(dir_path) else { return list };
        for entry in rd.flatten() {
            let file_name = entry.file_name().to_string_lossy().to_string();
            if skip_hidden && file_name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            // QDir::Files follows symlinks to files
            let Ok(meta) = std::fs::metadata(&path) else { continue };
            if !meta.is_file() {
                continue;
            }
            let abs = path.to_string_lossy().to_string();
            let lower = file_name.to_lowercase();
            let mut matched = extensions.iter().any(|e| lower.ends_with(e.as_str()));
            let mut mime = String::new();
            if !matched || sort_mode == 4 {
                mime = mime_for_file(&abs, allow_content);
                matched |= mime_types.iter().any(|m| *m == mime);
            }
            // ignore macOS ._ metadata files
            if file_name.starts_with("._") {
                matched = false;
            }
            if matched {
                let ms = |t: std::io::Result<SystemTime>| {
                    t.ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64).unwrap_or(0)
                };
                list.push(CompatibleFile {
                    absolute_file_path: abs,
                    file_name,
                    last_modified: if sort_mode == 1 { ms(meta.modified()) } else { 0 },
                    last_created: if sort_mode == 2 { ms(meta.created()) } else { 0 },
                    size: if sort_mode == 3 { meta.len() as i64 } else { 0 },
                    mime_type: if sort_mode == 4 { mime } else { String::new() },
                });
            }
        }
        list
    }

    pub fn update_folder_info(&mut self, dir_path: Option<String>) {
        let dir_path = match dir_path {
            Some(d) => d,
            None => {
                if self.details.file_path.is_empty() {
                    return;
                }
                util::parent_dir(&self.details.file_path)
            }
        };

        let mut list = Self::get_compatible_files(&dir_path);
        let (sort_mode, desc) = {
            let sm = app::settings();
            (sm.int(Setting::SortMode), sm.bool(Setting::SortDescending))
        };
        let dir_info = DirInfo { dir_path, file_count: list.len(), sort_mode, sort_descending: desc };
        if self.last_dir_info != dir_info {
            self.random_sort_seed = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
        }
        self.last_dir_info = dir_info;

        use std::cmp::Ordering;
        let rev = |o: Ordering| if desc { o.reverse() } else { o };
        match sort_mode {
            0 => list.sort_by(|a, b| rev(util::natural_cmp(&a.file_name, &b.file_name))),
            // Note: qView sorts dates and sizes newest/largest first when "ascending"
            1 => list.sort_by(|a, b| rev(b.last_modified.cmp(&a.last_modified))),
            2 => list.sort_by(|a, b| rev(b.last_created.cmp(&a.last_created))),
            3 => list.sort_by(|a, b| rev(b.size.cmp(&a.size))),
            4 => list.sort_by(|a, b| rev(util::natural_cmp(&a.mime_type, &b.mime_type))),
            5 => {
                use rand::seq::SliceRandom;
                use rand::SeedableRng;
                let mut rng = rand::rngs::StdRng::seed_from_u64(self.random_sort_seed);
                list.shuffle(&mut rng);
            }
            _ => {}
        }
        self.details.folder_file_info_list = list;
        self.details.update_loaded_index_in_folder();
    }

    fn request_caching(&mut self) {
        let (mode, loop_folders) = {
            let sm = app::settings();
            (sm.int(Setting::PreloadingMode), sm.bool(Setting::LoopFoldersEnabled))
        };
        if mode == 0 {
            CACHE.with(|c| c.borrow_mut().clear());
            return;
        }
        let target = self.get_target_color_space();
        let distance: i32 = if mode > 1 { 4 } else { 1 };
        let cur = self.details.loaded_index_in_folder;
        let len = self.details.folder_file_info_list.len() as i32;
        let mut files_to_preload = vec![];
        for i in (cur - distance)..=(cur + distance) {
            let mut index = i;
            if index == cur {
                continue;
            }
            if loop_folders {
                if index > len - 1 {
                    index -= len;
                } else if index < 0 {
                    index += len;
                }
            }
            if index > len - 1 || index < 0 || len == 0 {
                continue;
            }
            let path = self.details.folder_file_info_list[index as usize].absolute_file_path.clone();
            files_to_preload.push(path.clone());
            self.request_caching_file(path, target);
        }
        self.last_files_preloaded = files_to_preload;
    }

    fn request_caching_file(&mut self, path: String, target: TargetColorSpace) {
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let key = cache_key(&path, size, target);
        if CACHE.with(|c| c.borrow().contains(&key)) || self.last_files_preloaded.contains(&path) {
            return;
        }
        let max = CACHE.with(|c| c.borrow().max_cost);
        if (size / 1024) as usize > max / 2 {
            return;
        }
        if self.preload_files_in_progress.contains(&path) {
            return;
        }
        self.preload_files_in_progress.push(path.clone());
        self.spawn_read(path, target, true);
    }

    fn add_to_cache(rd: ReadData) {
        if rd.image.is_none() {
            return;
        }
        let key = cache_key(&rd.absolute_file_path, rd.file_size, rd.target_color_space);
        let cost = rd.cost_kib();
        CACHE.with(|c| c.borrow_mut().insert(key, rd, cost));
    }

    pub fn get_target_color_space(&self) -> TargetColorSpace {
        TargetColorSpace::Srgb
    }

    pub fn settings_updated(&mut self) {
        let mode = app::settings().int(Setting::PreloadingMode);
        // Cost is in KiB
        let cost = match mode {
            0 => 0,
            2 => 2_048_000,
            _ => 256_000,
        };
        CACHE.with(|c| c.borrow_mut().set_max_cost(cost));

        // update folder info to reflect new settings (e.g. sort order)
        self.update_folder_info(None);

    }

    // ---- movie controls ----

    pub fn jump_to_next_frame(&mut self) {
        if !self.details.is_movie_loaded {
            return;
        }
        if let Some(m) = self.movie.as_mut() {
            m.current = (m.current + 1) % m.frames.len();
        }
        self.emit(CoreEvent::AnimatedFrameChanged);
    }

    pub fn set_paused(&mut self, paused: bool) {
        if !self.details.is_movie_loaded {
            return;
        }
        if let Some(m) = self.movie.as_mut() {
            if m.paused != paused {
                m.paused = paused;
                m.generation = next_gen();
            }
        }
    }

    pub fn is_paused(&self) -> bool {
        self.movie.as_ref().map(|m| m.paused).unwrap_or(false)
    }

    pub fn set_speed(&mut self, speed: i32) {
        let speed = speed.clamp(0, 1000);
        if !self.details.is_movie_loaded {
            return;
        }
        if let Some(m) = self.movie.as_mut() {
            m.speed = speed;
            m.generation = next_gen();
        }
    }

    pub fn speed(&self) -> i32 {
        self.movie.as_ref().map(|m| m.speed).unwrap_or(100)
    }

    /// Delay before the next frame should be shown, or None if not playing.
    pub fn next_frame_delay(&self) -> Option<(u64, u64)> {
        let m = self.movie.as_ref()?;
        if !m.running || m.paused || m.speed == 0 {
            return None;
        }
        let d = m.frames[m.current].delay_ms.max(10) as u64;
        Some((d * 100 / m.speed as u64, m.generation))
    }

    pub fn movie_tick(&mut self, generation: u64) -> bool {
        let Some(m) = self.movie.as_mut() else { return false };
        if m.generation != generation || m.paused || !m.running {
            return false;
        }
        m.current = (m.current + 1) % m.frames.len();
        m.generation = next_gen();
        self.emit(CoreEvent::AnimatedFrameChanged);
        true
    }

    /// Current movie frame with the current rotation applied
    pub fn current_movie_raw(&self) -> Option<RawImage> {
        let m = self.movie.as_ref()?;
        Some(if self.current_rotation != 0 { m.current_image().rotated(self.current_rotation) } else { m.current_image().clone() })
    }

    pub fn current_movie_surface(&self) -> Option<cairo::ImageSurface> {
        surface_from_raw(&self.current_movie_raw()?)
    }

    pub fn rotate_image(&mut self, rotation: i32) {
        self.current_rotation = ((self.current_rotation + rotation) % 360 + 360) % 360;
        let new_raw = if self.details.is_movie_loaded {
            self.current_movie_raw()
        } else {
            self.loaded_raw.as_ref().map(|r| r.rotated(rotation))
        };
        if let Some(r) = new_raw {
            self.loaded_pixmap = surface_from_raw(&r);
            self.details.loaded_pixmap_size = (r.width, r.height);
            self.loaded_raw = Some(r);
        }
        self.emit(CoreEvent::UpdateLoadedPixmapItem);
    }

    /// QVImageCore::scaleExpensively
    pub fn scale_expensively(&self, desired_w: f64, desired_h: f64) -> Option<cairo::ImageSurface> {
        if !self.details.is_pixmap_loaded {
            return None;
        }
        let raw = self.loaded_raw.as_ref()?;
        let (pw, ph) = (raw.width as i64, raw.height as i64);
        let (dw, dh) = (desired_w.round() as i64, desired_h.round() as i64);
        // QSize::scale(KeepAspectRatio)
        let (sw, sh) = if ph == 0 || pw == 0 {
            (dw, dh)
        } else {
            let rw = dh * pw / ph;
            if rw <= dw {
                (rw, dh)
            } else {
                (dw, dw * ph / pw)
            }
        };
        let relevant = if self.details.is_movie_loaded { self.current_movie_raw()? } else { raw.clone() };
        if (desired_w - relevant.width as f64).abs() < 1.0 && (desired_h - relevant.height as f64).abs() < 1.0 {
            return surface_from_raw(&relevant);
        }
        if sw <= 0 || sh <= 0 {
            return surface_from_raw(&relevant);
        }
        surface_from_raw(&decode::scale_smooth(&relevant, sw as i32, sh as i32))
    }
}

pub fn mime_for_file(path: &str, content: bool) -> String {
    let name = util::file_name(path);
    let data: Vec<u8> = if content {
        use std::io::Read;
        let mut buf = vec![0u8; 4096];
        match std::fs::File::open(path).and_then(|mut f| f.read(&mut buf)) {
            Ok(n) => {
                buf.truncate(n);
                buf
            }
            Err(_) => vec![],
        }
    } else {
        vec![]
    };
    let (ct, _uncertain) = gio::content_type_guess(Some(name.as_str()), &data);
    gio::content_type_get_mime_type(&ct).map(|m| m.to_string()).unwrap_or_else(|| ct.to_string())
}
