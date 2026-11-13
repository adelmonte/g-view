//! Port of QVGraphicsView. The scene/view math mirrors Qt's QGraphicsView with
//! `NoAnchor`, hidden scrollbars and `ScrollHandDrag`, so zoom, fit and panning behave the same.
//!
//! Model: the pixmap item sits at scene origin; screen = T * scene - scroll, where
//! T = diag(m11, m22) and scroll = (h, v) are the (integer) scrollbar values.

use crate::app;
use crate::imagecore::CoreEvent;
use crate::settings::Setting;
use crate::window::MainWindow;
use gtk::prelude::*;
use std::rc::Rc;
use std::time::Duration;

const MARGIN: f64 = -2.0;
const MAX_EXPENSIVE_SCALING_SIZE: f64 = 3.0;
const MAX_FILTERING_SIZE: f64 = 5000.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GoToFileMode {
    First,
    Previous,
    Next,
    Last,
    Random,
}

pub struct ViewState {
    /// The pixmap shown by loadedPixmapItem (None = empty pixmap)
    pub item: Option<cairo::ImageSurface>,
    pub smooth: bool,
    pub m11: f64,
    pub m22: f64,
    pub h: i64,
    pub v: i64,
    pub current_scale: f64,
    pub is_original_size: bool,
    pub last_zoom_event_pos: (i32, i32),
    pub last_zoom_rounding_error: (f64, f64),
    pub last_scroll_rounding_error: (f64, f64),
    pub absolute_transform: f64,
    pub zoom_basis: (f64, f64),
    pub zoom_basis_scale_factor: f64,
    pub expensive_timer: Option<glib::SourceId>,
    pub hand_drag: Option<(i32, i32)>,
    pub under_mouse: bool,
    pub size: (i32, i32),
    pub pinch_last: f64,
}

impl Default for ViewState {
    fn default() -> Self {
        ViewState {
            item: None,
            smooth: true,
            m11: 1.0,
            m22: 1.0,
            h: 0,
            v: 0,
            current_scale: 1.0,
            is_original_size: false,
            last_zoom_event_pos: (-1, -1),
            last_zoom_rounding_error: (0.0, 0.0),
            last_scroll_rounding_error: (0.0, 0.0),
            absolute_transform: 1.0,
            zoom_basis: (1.0, 1.0),
            zoom_basis_scale_factor: 1.0,
            expensive_timer: None,
            hand_drag: None,
            under_mouse: false,
            size: (0, 0),
            pinch_last: 1.0,
        }
    }
}

impl ViewState {
    fn scale(&mut self, sx: f64, sy: f64) {
        self.m11 *= sx;
        self.m22 *= sy;
    }
    fn set_transform(&mut self, a: f64, d: f64) {
        self.m11 = a;
        self.m22 = d;
    }
    fn item_size(&self) -> (f64, f64) {
        self.item.as_ref().map(|s| (s.width() as f64, s.height() as f64)).unwrap_or((0.0, 0.0))
    }
    pub fn map_to_scene(&self, p: (f64, f64)) -> (f64, f64) {
        ((p.0 + self.h as f64) / self.m11, (p.1 + self.v as f64) / self.m22)
    }
    pub fn map_from_scene(&self, p: (f64, f64)) -> (f64, f64) {
        (self.m11 * p.0 - self.h as f64, self.m22 * p.1 - self.v as f64)
    }
    /// QGraphicsView::centerOn (qView's override)
    fn center_on(&mut self, pos: (f64, f64)) {
        let (w, h) = (self.size.0 as f64, self.size.1 as f64);
        let vx = self.m11 * pos.0;
        let vy = self.m22 * pos.1;
        self.h = (vx - w / 2.0).trunc() as i64;
        self.v = (vy - h / 2.0).trunc() as i64;
    }
    /// centerOn(loadedPixmapItem): center of the item's scene bounding rect
    fn center_on_item(&mut self) {
        let (iw, ih) = self.item_size();
        self.center_on((iw / 2.0, ih / 2.0));
    }
}

fn dpr(win: &MainWindow) -> f64 {
    win.area.scale_factor().max(1) as f64
}

impl MainWindow {
    pub fn view_queue_draw(&self) {
        self.area.queue_draw();
    }

    /// Process queued core events (Qt signal delivery)
    pub fn pump(self: &Rc<Self>) {
        loop {
            let events: Vec<CoreEvent> = std::mem::take(&mut self.core.borrow_mut().pending);
            if events.is_empty() {
                break;
            }
            for e in events {
                match e {
                    CoreEvent::FileChanged => self.post_load(),
                    CoreEvent::UpdateLoadedPixmapItem => self.update_loaded_pixmap_item(),
                    CoreEvent::AnimatedFrameChanged => self.animated_frame_changed(),
                }
            }
        }
        self.schedule_movie();
    }

    fn schedule_movie(self: &Rc<Self>) {
        let next = self.core.borrow().next_frame_delay();
        let Some((delay, gen)) = next else { return };
        if self.st.borrow().movie_scheduled == Some(gen) {
            return;
        }
        self.st.borrow_mut().movie_scheduled = Some(gen);
        let w = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(delay), move || {
            if let Some(w) = w.upgrade() {
                let advanced = w.core.borrow_mut().movie_tick(gen);
                if advanced {
                    w.pump();
                }
            }
        });
    }

    // ---- loading ----

    pub fn view_load_file(self: &Rc<Self>, file: &str) {
        self.core.borrow_mut().load_file(file, false);
        self.pump();
    }

    pub fn view_reload_file(self: &Rc<Self>) {
        if !self.core.borrow().details.is_pixmap_loaded {
            return;
        }
        let p = self.core.borrow().details.file_path.clone();
        self.core.borrow_mut().load_file(&p, true);
        self.pump();
    }

    fn post_load(self: &Rc<Self>) {
        self.update_loaded_pixmap_item();
        let path = self.core.borrow().details.file_path.clone();
        app::add_file_to_recents_list(&path);
        self.file_changed();
    }

    fn update_loaded_pixmap_item(self: &Rc<Self>) {
        {
            let core = self.core.borrow();
            self.view.borrow_mut().item = core.loaded_pixmap.clone();
        }
        self.reset_scale();
        self.set_window_size();
        // Do the high-quality scale right away instead of after the 50ms timer, so the first
        // frame of a new image is already the crisp, small version (avoids painting the
        // full-resolution image once).
        if app::settings().bool(Setting::ScalingEnabled) && self.core.borrow().details.is_pixmap_loaded {
            if let Some(id) = self.view.borrow_mut().expensive_timer.take() {
                id.remove();
            }
            self.scale_expensively();
        }
        self.view_queue_draw();
    }

    fn animated_frame_changed(self: &Rc<Self>) {
        if app::settings().bool(Setting::ScalingEnabled) {
            self.scale_expensively();
        } else {
            let s = self.core.borrow().current_movie_surface();
            self.view.borrow_mut().item = s;
        }
        self.view_queue_draw();
    }

    // ---- zoom ----

    pub fn zoom_in(self: &Rc<Self>, pos: (i32, i32)) {
        let f = app::settings().int(Setting::ScaleFactor) as f64 / 100.0 + 1.0;
        self.zoom(f, pos);
    }

    pub fn zoom_out(self: &Rc<Self>, pos: (i32, i32)) {
        let f = (app::settings().int(Setting::ScaleFactor) as f64 / 100.0 + 1.0).powi(-1);
        self.zoom(f, pos);
    }

    pub fn zoom(self: &Rc<Self>, scale_factor: f64, pos: (i32, i32)) {
        let sm = app::settings();
        {
            let mut v = self.view.borrow_mut();
            // don't zoom too far out, dude
            v.current_scale *= scale_factor;
            if v.current_scale >= 500.0 || v.current_scale <= 0.01 {
                v.current_scale *= scale_factor.powi(-1);
                return;
            }
        }
        self.update_filtering_mode();
        let mut v = self.view.borrow_mut();

        if pos != v.last_zoom_event_pos {
            v.last_zoom_event_pos = pos;
            v.last_zoom_rounding_error = (0.0, 0.0);
        }
        let sp = v.map_to_scene((pos.0 as f64, pos.1 as f64));
        let scene_pos = (sp.0 - v.last_zoom_rounding_error.0, sp.1 - v.last_zoom_rounding_error.1);

        v.zoom_basis_scale_factor *= scale_factor;
        let (zb, zf) = (v.zoom_basis, v.zoom_basis_scale_factor);
        v.set_transform(zb.0 * zf, zb.1 * zf);
        v.absolute_transform *= scale_factor;

        if v.current_scale > 1.00001 && pos != (-1, -1) && v.under_mouse && sm.bool(Setting::CursorZoom) {
            let p1 = v.map_from_scene(scene_pos);
            let mv = (p1.0 - pos.0 as f64, p1.1 - pos.1 as f64);
            v.h = (v.h as f64 + mv.0).trunc() as i64;
            v.v = (v.v as f64 + mv.1).trunc() as i64;
            let np = v.map_to_scene((pos.0 as f64, pos.1 as f64));
            v.last_zoom_rounding_error = (np.0 - scene_pos.0, np.1 - scene_pos.1);
        } else {
            v.center_on_item();
        }
        let restart = sm.bool(Setting::ScalingEnabled) && !v.is_original_size;
        drop(v);
        if restart {
            self.start_expensive_timer();
        }
        self.view_queue_draw();
    }

    fn start_expensive_timer(self: &Rc<Self>) {
        if let Some(id) = self.view.borrow_mut().expensive_timer.take() {
            id.remove();
        }
        let w = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_millis(50), move || {
            if let Some(w) = w.upgrade() {
                w.view.borrow_mut().expensive_timer = None;
                w.scale_expensively();
                w.view_queue_draw();
            }
        });
        self.view.borrow_mut().expensive_timer = Some(id);
    }

    pub fn scale_expensively(self: &Rc<Self>) {
        let _t = crate::util::Trace::new("scale_expensively");
        let sm = app::settings();
        let (mirrored, flipped, current_scale) = {
            let v = self.view.borrow();
            (v.m11 < 0.0, v.m22 < 0.0, v.current_scale)
        };
        if current_scale >= MAX_EXPENSIVE_SCALING_SIZE || (!sm.bool(Setting::ScalingTwoEnabled) && current_scale > 1.00001) {
            self.make_unscaled();
            return;
        }
        let dpr = dpr(self);
        let (lw, lh) = self.core.borrow().details.loaded_pixmap_size;
        let at = self.view.borrow().absolute_transform;
        let (dw, dh) = (lw as f64 * at * dpr, lh as f64 * at * dpr);
        let scaled = self.core.borrow().scale_expensively(dw, dh);
        let mut v = self.view.borrow_mut();
        v.item = scaled;
        v.set_transform(1.0 / dpr, 1.0 / dpr);
        if mirrored {
            v.scale(-1.0, 1.0);
        }
        if flipped {
            v.scale(1.0, -1.0);
        }
        v.zoom_basis = (v.m11, v.m22);
        v.zoom_basis_scale_factor = 1.0;
    }

    pub fn make_unscaled(self: &Rc<Self>) {
        let item = {
            let core = self.core.borrow();
            if core.details.is_movie_loaded {
                core.current_movie_surface()
            } else {
                core.loaded_pixmap.clone()
            }
        };
        let mut v = self.view.borrow_mut();
        let (mirrored, flipped) = (v.m11 < 0.0, v.m22 < 0.0);
        v.item = item;
        let at = v.absolute_transform;
        v.set_transform(at, at);
        if mirrored {
            v.scale(-1.0, 1.0);
        }
        if flipped {
            v.scale(1.0, -1.0);
        }
        v.zoom_basis = (v.m11, v.m22);
        v.zoom_basis_scale_factor = 1.0;
    }

    fn update_filtering_mode(&self) {
        let filtering = app::settings().bool(Setting::FilteringEnabled);
        let mut v = self.view.borrow_mut();
        v.smooth = v.current_scale < MAX_FILTERING_SIZE && filtering;
    }

    pub fn reset_scale(self: &Rc<Self>) {
        if !self.core.borrow().details.is_pixmap_loaded {
            return;
        }
        self.fit_in_view_marginless();
        if app::settings().bool(Setting::ScalingEnabled) {
            self.start_expensive_timer();
        }
        self.view_queue_draw();
    }

    pub fn original_size(self: &Rc<Self>) {
        {
            let v = self.view.borrow();
            if v.is_original_size && v.m11 == 1.0 && v.m22 == 1.0 {
                drop(v);
                self.reset_scale();
                return;
            }
        }
        self.make_unscaled();
        let mut v = self.view.borrow_mut();
        v.set_transform(1.0, 1.0);
        v.center_on_item();
        v.zoom_basis = (1.0, 1.0);
        v.zoom_basis_scale_factor = 1.0;
        v.absolute_transform = 1.0;
        v.is_original_size = true;
        drop(v);
        self.view_queue_draw();
    }

    /// QVGraphicsView::fitInViewMarginless(loadedPixmapItem)
    fn fit_in_view_marginless(self: &Rc<Self>) {
        let sm = app::settings();
        let crop = sm.int(Setting::CropMode);
        let past_actual = sm.bool(Setting::PastActualSizeEnabled);
        let loaded = self.core.borrow().details.loaded_pixmap_size;
        let mut v = self.view.borrow_mut();
        let (iw, ih) = v.item_size();
        let rect_center = (iw / 2.0, ih / 2.0);
        let mut adj_img = (loaded.0 as f64, loaded.1 as f64);
        let mut adj_rect = (iw, ih);
        match crop {
            1 => {
                adj_img.0 = 1.0;
                adj_rect.0 = 1.0;
            }
            2 => {
                adj_img.1 = 1.0;
                adj_rect.1 = 1.0;
            }
            _ => {}
        }
        if adj_rect.0 <= 0.0 || adj_rect.1 <= 0.0 {
            return;
        }
        // Reset the view scale to 1:1 (keeps mirror/flip signs)
        let (ux, uy) = (v.m11.abs(), v.m22.abs());
        if ux == 0.0 || uy == 0.0 {
            return;
        }
        v.scale(1.0 / ux, 1.0 / uy);

        let (w, h) = (v.size.0 as f64, v.size.1 as f64);
        let adj_w = w - MARGIN;
        let adj_h = h - MARGIN;
        let view_rect: (f64, f64) = if past_actual || adj_img.0 >= adj_w || adj_img.1 >= adj_h {
            (w - 2.0 * MARGIN, h - 2.0 * MARGIN)
        } else {
            (loaded.0 as f64, loaded.1 as f64)
        };
        if view_rect.0 <= 0.0 || view_rect.1 <= 0.0 {
            return;
        }
        let ratio = (view_rect.0 / adj_rect.0).min(view_rect.1 / adj_rect.1);
        let absolute_ratio = (view_rect.0 / adj_img.0).min(view_rect.1 / adj_img.1);
        v.absolute_transform = absolute_ratio;
        v.scale(ratio, ratio);
        v.center_on(rect_center);
        v.zoom_basis = (v.m11, v.m22);
        v.is_original_size = false;
        v.current_scale = 1.0;
        v.zoom_basis_scale_factor = 1.0;
        drop(v);
        self.update_filtering_mode();
    }

    pub fn view_resized(self: &Rc<Self>, w: i32, h: i32) {
        {
            let mut v = self.view.borrow_mut();
            if v.size == (w, h) {
                return;
            }
            v.size = (w, h);
        }
        let orig = self.view.borrow().is_original_size;
        if !orig {
            self.reset_scale();
        } else {
            self.view.borrow_mut().center_on_item();
            self.view_queue_draw();
        }
    }

    pub fn view_settings_updated(self: &Rc<Self>) {
        if self.core.borrow().details.is_pixmap_loaded {
            self.reset_scale();
        }
    }

    pub fn mirror_view(self: &Rc<Self>) {
        self.view.borrow_mut().scale(-1.0, 1.0);
    }
    pub fn flip_view(self: &Rc<Self>) {
        self.view.borrow_mut().scale(1.0, -1.0);
    }

    // ---- navigation ----

    pub fn go_to_file(self: &Rc<Self>, mode: GoToFileMode, _index: i32) {
        let mut should_retry_folder_info_update = false;
        {
            let mut core = self.core.borrow_mut();
            let expired = core.details.time_since_loaded.map(|t| t.elapsed() > Duration::from_millis(3000)).unwrap_or(true);
            if expired {
                if std::path::Path::new(&core.details.file_path).exists() {
                    core.update_folder_info(None);
                } else {
                    should_retry_folder_info_update = true;
                }
            }
        }
        let (next_path, cur_path) = {
            let core = self.core.borrow();
            let list = &core.details.folder_file_info_list;
            if list.is_empty() {
                return;
            }
            let len = list.len() as i32;
            let mut new_index = core.details.loaded_index_in_folder;
            let search;
            let loop_folders = app::settings().bool(Setting::LoopFoldersEnabled);
            let mut cancel = false;
            match mode {
                GoToFileMode::First => {
                    new_index = 0;
                    search = 1;
                }
                GoToFileMode::Previous => {
                    if new_index == 0 {
                        if loop_folders {
                            new_index = len - 1;
                        } else {
                            cancel = true;
                        }
                    } else {
                        new_index -= 1;
                    }
                    search = -1;
                }
                GoToFileMode::Next => {
                    if len - 1 == new_index {
                        if loop_folders {
                            new_index = 0;
                        } else {
                            cancel = true;
                        }
                    } else {
                        new_index += 1;
                    }
                    search = 1;
                }
                GoToFileMode::Last => {
                    new_index = len - 1;
                    search = -1;
                }
                GoToFileMode::Random => {
                    if len > 1 {
                        use rand::RngExt;
                        let mut rng = rand::rng();
                        loop {
                            let r = rng.random_range(0..len);
                            if r != new_index {
                                new_index = r;
                                break;
                            }
                        }
                    }
                    search = 1;
                }
            }
            if cancel {
                drop(core);
                self.cancel_slideshow();
                return;
            }
            let exists = |i: i32| list.get(i as usize).map(|f| std::path::Path::new(&f.absolute_file_path).exists()).unwrap_or(false);
            if search != 0 {
                while search == 1 && new_index < len - 1 && !exists(new_index) {
                    new_index += 1;
                }
                while search == -1 && new_index > 0 && !exists(new_index) {
                    new_index -= 1;
                }
            }
            let next = list.get(new_index.max(0) as usize).map(|f| f.absolute_file_path.clone()).unwrap_or_default();
            (next, core.details.file_path.clone())
        };
        if next_path.is_empty() || !std::path::Path::new(&next_path).exists() || next_path == cur_path {
            return;
        }
        if should_retry_folder_info_update {
            self.core.borrow_mut().update_folder_info(Some(crate::util::parent_dir(&next_path)));
        }
        self.view_load_file(&next_path);
    }

    // ---- drawing ----

    pub fn draw(&self, cr: &cairo::Context) {
        let _t = crate::util::Trace::new("draw");
        let (w, h) = (self.area.allocated_width() as f64, self.area.allocated_height() as f64);
        let bg = self.st.borrow().custom_bg;
        let bg_rgb = match bg {
            Some(c) => c,
            None => {
                let ctx = self.window.style_context();
                #[allow(deprecated)]
                let c = ctx.style_property_for_state("background-color", gtk::StateFlags::NORMAL);
                let _ = c;
                gtk::render_background(&ctx, cr, 0.0, 0.0, w, h);
                (-1.0, 0.0, 0.0)
            }
        };
        if bg_rgb.0 >= 0.0 {
            cr.set_source_rgb(bg_rgb.0, bg_rgb.1, bg_rgb.2);
            let _ = cr.paint();
        }

        let v = self.view.borrow();
        if let Some(item) = v.item.as_ref() {
            let _ = cr.save();
            cr.translate(-(v.h as f64), -(v.v as f64));
            cr.scale(v.m11, v.m22);
            if cr.set_source_surface(item, 0.0, 0.0).is_ok() {
                let src = cr.source();
                let exact = (v.m11.abs() * dpr(self) - 1.0).abs() < 1e-9 && (v.m22.abs() * dpr(self) - 1.0).abs() < 1e-9;
                src.set_filter(if v.smooth && !exact { cairo::Filter::Bilinear } else { cairo::Filter::Nearest });
                cr.rectangle(0.0, 0.0, item.width() as f64, item.height() as f64);
                let _ = cr.fill();
            }
            let _ = cr.restore();
        }
        drop(v);

        // Error message
        let core = self.core.borrow();
        if core.details.error_data.has_error {
            let e = &core.details.error_data;
            let msg = crate::i18n::tr("MainWindow", "Error occurred opening\n%3\n%2 (Error %1)")
                .replace("%1", &e.error_num.to_string())
                .replace("%2", &e.error_string)
                .replace("%3", &crate::util::file_name(&core.details.file_path));
            let (r, g, b) = match bg {
                Some(c) => c,
                None => {
                    let c = self.window.style_context().color(gtk::StateFlags::NORMAL);
                    // inverse of text color approximates the window background
                    (1.0 - c.red(), 1.0 - c.green(), 1.0 - c.blue())
                }
            };
            let light = crate::util::perceived_brightness(r, g, b) > 0.5;
            if light {
                cr.set_source_rgb(0.0, 0.0, 0.0);
            } else {
                cr.set_source_rgb(1.0, 1.0, 1.0);
            }
            let layout = self.area.create_pango_layout(Some(&msg));
            layout.set_alignment(pango::Alignment::Center);
            let (lw, lh) = layout.pixel_size();
            cr.move_to(((w - lw as f64) / 2.0).round(), ((h - lh as f64) / 2.0).round());
            pangocairo_show(cr, &layout);
        }
    }

    // ---- input ----

    pub fn on_scroll(self: &Rc<Self>, ev: &gdk::EventScroll) {
        let (ax, ay): (f64, f64) = match ev.direction() {
            gdk::ScrollDirection::Up => (0.0, 120.0),
            gdk::ScrollDirection::Down => (0.0, -120.0),
            gdk::ScrollDirection::Left => (120.0, 0.0),
            gdk::ScrollDirection::Right => (-120.0, 0.0),
            _ => {
                let (dx, dy) = ev.delta();
                (-dx * 120.0, -dy * 120.0)
            }
        };
        let pos = ev.position();
        let event_pos = (pos.0 as i32, pos.1 as i32);
        let sm = app::settings();
        let ctrl = ev.state().contains(gdk::ModifierType::CONTROL_MASK);
        let scroll_zoom = sm.int(Setting::ScrollZoom);
        let mut dont_zoom = scroll_zoom == 2;
        if ctrl {
            dont_zoom = !dont_zoom;
        }
        let touch = ev
            .source_device()
            .map(|d| matches!(d.source(), gdk::InputSource::Touchpad | gdk::InputSource::Touchscreen))
            .unwrap_or(false);
        if touch && scroll_zoom == 1 {
            dont_zoom = !ctrl;
        }

        if dont_zoom {
            let divisor = 2.0;
            let mut sx = ax * -1.0 / divisor;
            let mut sy = ay * -1.0 / divisor;
            if ev.state().contains(gdk::ModifierType::SHIFT_MASK) {
                std::mem::swap(&mut sx, &mut sy);
            }
            let mut v = self.view.borrow_mut();
            let target = (sx - v.last_scroll_rounding_error.0, sy - v.last_scroll_rounding_error.1);
            let rounded = (target.0.round(), target.1.round());
            v.h += rounded.0 as i64;
            v.v += rounded.1 as i64;
            v.last_scroll_rounding_error = (rounded.0 - target.0, rounded.1 - target.1);
            drop(v);
            self.view_queue_draw();
            return;
        }

        let y_delta = ay;
        if y_delta == 0.0 {
            return;
        }
        let mut zoom_factor = sm.int(Setting::ScaleFactor) as f64 / 100.0;
        if sm.bool(Setting::FractionalZoom) || touch {
            zoom_factor *= y_delta.abs() / 120.0;
        }
        zoom_factor += 1.0;
        if y_delta < 0.0 {
            zoom_factor = zoom_factor.powi(-1);
        }
        drop(sm);
        self.zoom(zoom_factor, event_pos);
    }

    pub fn on_button_press(self: &Rc<Self>, ev: &gdk::EventButton) -> bool {
        let (x, y) = ev.position();
        let button = ev.button();
        let state = self.window.window().map(|w| w.state()).unwrap_or(gdk::WindowState::empty());
        let fs_or_max = state.intersects(gdk::WindowState::FULLSCREEN | gdk::WindowState::MAXIMIZED);

        if ev.event_type() == gdk::EventType::DoubleButtonPress {
            if button == 1 {
                self.toggle_full_screen();
            }
            return true;
        }
        if ev.event_type() != gdk::EventType::ButtonPress {
            return true;
        }
        match button {
            1 => {
                if ev.state().contains(gdk::ModifierType::CONTROL_MASK) && app::settings().bool(Setting::CtrlDragWindow) && !fs_or_max {
                    let (rx, ry) = ev.root();
                    self.window.begin_move_drag(1, rx as i32, ry as i32, ev.time());
                    return true;
                }
                self.view.borrow_mut().hand_drag = Some((x as i32, y as i32));
                if let Some(gw) = self.area.window() {
                    let cursor = gdk::Cursor::from_name(&gw.display(), "grabbing");
                    gw.set_cursor(cursor.as_ref());
                }
            }
            2 => self.reset_zoom(),
            3 => self.show_context_menu(Some(ev)),
            8 => self.previous_file(),
            9 => self.next_file(),
            _ => {}
        }
        true
    }

    pub fn on_button_release(self: &Rc<Self>, _ev: &gdk::EventButton) {
        self.view.borrow_mut().hand_drag = None;
        if let Some(gw) = self.area.window() {
            gw.set_cursor(None);
        }
    }

    pub fn on_motion(self: &Rc<Self>, ev: &gdk::EventMotion) {
        let (x, y) = ev.position();
        let (x, y) = (x as i32, y as i32);
        let mut v = self.view.borrow_mut();
        if let Some((lx, ly)) = v.hand_drag {
            if !ev.state().contains(gdk::ModifierType::BUTTON1_MASK) {
                v.hand_drag = None;
                return;
            }
            let (dx, dy) = (x - lx, y - ly);
            if dx != 0 || dy != 0 {
                v.h -= dx as i64;
                v.v -= dy as i64;
                v.hand_drag = Some((x, y));
                drop(v);
                self.view_queue_draw();
            }
        }
    }

    pub fn on_pinch_begin(&self) {
        self.view.borrow_mut().pinch_last = 1.0;
    }

    pub fn on_pinch(self: &Rc<Self>, scale: f64, center: (f64, f64)) {
        let last = self.view.borrow().pinch_last;
        if last <= 0.0 || scale <= 0.0 {
            return;
        }
        self.view.borrow_mut().pinch_last = scale;
        self.zoom(scale / last, (center.0 as i32, center.1 as i32));
    }

    // ---- drag & drop ----

    pub fn load_uris(self: &Rc<Self>, uris: &[String]) {
        let mut first = true;
        for u in uris {
            if first {
                self.view_load_file(u);
                self.cancel_slideshow();
                first = false;
                continue;
            }
            app::open_file(u, true);
        }
    }
}

fn pangocairo_show(cr: &cairo::Context, layout: &pango::Layout) {
    extern "C" {
        fn pango_cairo_show_layout(cr: *mut cairo::ffi::cairo_t, layout: *mut pango::ffi::PangoLayout);
    }
    use glib::translate::ToGlibPtr;
    unsafe {
        pango_cairo_show_layout(cr.to_raw_none(), layout.to_glib_none().0);
    }
}
