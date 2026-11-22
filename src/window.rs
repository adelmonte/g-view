//! Port of MainWindow.

use crate::actions::{self, Enable};
use crate::app;
use crate::dialogs;
use crate::i18n::{mnemonic, tr};
use crate::imagecore::{ImageCore, LoaderMsg};
use crate::openwith::{self, OpenWithItem};
use crate::qtconf::{self, Geometry};
use crate::settings::{Setting, Store};
use crate::util;
use crate::view::{GoToFileMode, ViewState};
use gtk::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

pub struct DeletedPaths {
    pub path_in_trash: String,
    pub previous_path: String,
}

#[derive(Default)]
pub struct WinState {
    pub just_launched_with_image: bool,
    pub stored_maximized: bool,
    pub last_deleted_files: Vec<DeletedPaths>,
    pub slideshow: Option<glib::SourceId>,
    pub custom_bg: Option<(f64, f64, f64)>,
    pub movie_scheduled: Option<u64>,
    pub open_with_items: Vec<OpenWithItem>,
    pub open_with_timer: Option<glib::SourceId>,
    pub open_with_request: u64,
    pub fullscreen: bool,
    pub maximized: bool,
    pub normal_geometry: Option<Geometry>,
    pub temp_files: Vec<std::path::PathBuf>,
}

pub struct MainWindow {
    pub window: gtk::Window,
    pub area: gtk::DrawingArea,
    pub menubar: gtk::MenuBar,
    pub fullscreen_label: gtk::Label,
    pub view: RefCell<ViewState>,
    pub core: RefCell<ImageCore>,
    pub st: RefCell<WinState>,
    items: RefCell<HashMap<&'static str, Vec<(glib::WeakRef<gtk::MenuItem>, glib::WeakRef<gtk::AccelLabel>)>>>,
    openwith_menus: RefCell<Vec<glib::WeakRef<gtk::MenuItem>>>,
    context_menu: RefCell<Option<gtk::Menu>>,
    info: RefCell<Option<dialogs::InfoDialog>>,
    zoom_gesture: RefCell<Option<gtk::GestureZoom>>,
}

impl MainWindow {
    pub fn new() -> Rc<MainWindow> {
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_title("g-view");
        window.set_icon_name(Some("g-view"));
        let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let menubar = gtk::MenuBar::new();
        let fullscreen_label = gtk::Label::new(None);
        fullscreen_label.set_margin_top(3);
        fullscreen_label.set_margin_bottom(3);
        fullscreen_label.set_margin_start(3);
        fullscreen_label.set_margin_end(3);
        fullscreen_label.set_xalign(0.0);
        fullscreen_label.set_no_show_all(true);
        menubar.set_no_show_all(true);
        let area = gtk::DrawingArea::new();
        area.set_hexpand(true);
        area.set_vexpand(true);
        area.set_can_focus(true);
        vbox.pack_start(&menubar, false, false, 0);
        vbox.pack_start(&fullscreen_label, false, false, 0);
        vbox.pack_start(&area, true, true, 0);
        window.add(&vbox);

        let (tx, rx) = async_channel::unbounded::<LoaderMsg>();
        let win = Rc::new(MainWindow {
            window,
            area,
            menubar,
            fullscreen_label,
            view: RefCell::new(ViewState::default()),
            core: RefCell::new(ImageCore::new(tx)),
            st: RefCell::new(WinState::default()),
            items: RefCell::new(HashMap::new()),
            openwith_menus: RefCell::new(vec![]),
            context_menu: RefCell::new(None),
            info: RefCell::new(None),
            zoom_gesture: RefCell::new(None),
        });

        // Loader results
        {
            let w = Rc::downgrade(&win);
            glib::spawn_future_local(async move {
                while let Ok(msg) = rx.recv().await {
                    let Some(w) = w.upgrade() else { break };
                    w.core.borrow_mut().on_loader_msg(msg);
                    w.pump();
                }
            });
        }

        win.connect_signals();
        actions::build_menu_bar(&win, &win.menubar);
        win.disable_actions();
        win.settings_updated();

        // Load window geometry
        let geo = Store::get("General", "geometry").and_then(|s| qtconf::decode_geometry(&s));
        match geo {
            Some(g) => {
                win.window.set_default_size(g.w, g.h);
                if g.maximized {
                    win.window.maximize();
                }
                win.st.borrow_mut().normal_geometry = Some(g);
            }
            None => win.window.set_default_size(640, 480),
        }
        win
    }

    /// Things that need the window to exist on screen (first-launch welcome dialog)
    pub fn after_show(self: &Rc<Self>) {
        let first = Store::get("General", "firstlaunch").map(|s| s == "true").unwrap_or(false);
        if !first {
            let kf = Store::load();
            kf.set_string("General", "firstlaunch", "true");
            kf.set_string("General", "configversion", &crate::settings::fmt_double(app::VERSION));
            Store::write_keyfile(&kf);
            dialogs::open_welcome_dialog(Some(&self.window));
        }
    }

    fn connect_signals(self: &Rc<Self>) {
        let area = &self.area;
        area.add_events(
            gdk::EventMask::BUTTON_PRESS_MASK
                | gdk::EventMask::BUTTON_RELEASE_MASK
                | gdk::EventMask::POINTER_MOTION_MASK
                | gdk::EventMask::SCROLL_MASK
                | gdk::EventMask::SMOOTH_SCROLL_MASK
                | gdk::EventMask::ENTER_NOTIFY_MASK
                | gdk::EventMask::LEAVE_NOTIFY_MASK
                | gdk::EventMask::TOUCHPAD_GESTURE_MASK,
        );
        let weak = Rc::downgrade(self);
        macro_rules! w {
            () => {{
                let weak = weak.clone();
                move || weak.upgrade()
            }};
        }

        let get = w!();
        area.connect_draw(move |_, cr| {
            if let Some(w) = get() {
                w.draw(cr);
            }
            glib::Propagation::Stop
        });
        let get = w!();
        area.connect_size_allocate(move |_, alloc| {
            if let Some(w) = get() {
                w.view_resized(alloc.width(), alloc.height());
            }
        });
        let get = w!();
        area.connect_button_press_event(move |a, ev| {
            a.grab_focus();
            if let Some(w) = get() {
                w.on_button_press(ev);
            }
            glib::Propagation::Stop
        });
        let get = w!();
        area.connect_button_release_event(move |_, ev| {
            if let Some(w) = get() {
                w.on_button_release(ev);
            }
            glib::Propagation::Stop
        });
        let get = w!();
        area.connect_motion_notify_event(move |_, ev| {
            if let Some(w) = get() {
                w.on_motion(ev);
            }
            glib::Propagation::Stop
        });
        let get = w!();
        area.connect_scroll_event(move |_, ev| {
            if let Some(w) = get() {
                w.on_scroll(ev);
            }
            glib::Propagation::Stop
        });
        let get = w!();
        area.connect_enter_notify_event(move |a, _| {
            if let Some(w) = get() {
                w.view.borrow_mut().under_mouse = true;
                if w.view.borrow().hand_drag.is_none() {
                    if let Some(gw) = a.window() {
                        gw.set_cursor(None);
                    }
                }
            }
            glib::Propagation::Proceed
        });
        let get = w!();
        area.connect_leave_notify_event(move |_, ev| {
            if let Some(w) = get() {
                if ev.detail() != gdk::NotifyType::Inferior {
                    w.view.borrow_mut().under_mouse = false;
                }
            }
            glib::Propagation::Proceed
        });
        let get = w!();
        area.connect_popup_menu(move |_| {
            if let Some(w) = get() {
                w.show_context_menu(None);
            }
            true
        });

        // Pinch zoom
        let gesture = gtk::GestureZoom::new(area);
        gesture.set_propagation_phase(gtk::PropagationPhase::Bubble);
        let get = w!();
        gesture.connect_begin(move |_, _| {
            if let Some(w) = get() {
                w.on_pinch_begin();
            }
        });
        let get = w!();
        gesture.connect_scale_changed(move |g, scale| {
            if let Some(w) = get() {
                let c = g.bounding_box_center().unwrap_or((-1.0, -1.0));
                w.on_pinch(scale, c);
            }
        });
        *self.zoom_gesture.borrow_mut() = Some(gesture);

        // Drag & drop
        let targets = [gtk::TargetEntry::new("text/uri-list", gtk::TargetFlags::OTHER_APP | gtk::TargetFlags::SAME_APP, 0)];
        area.drag_dest_set(gtk::DestDefaults::ALL, &targets, gdk::DragAction::COPY | gdk::DragAction::MOVE | gdk::DragAction::LINK);
        let get = w!();
        area.connect_drag_data_received(move |_, _, _, _, data, _, _| {
            if let Some(w) = get() {
                let uris: Vec<String> = data.uris().iter().map(|s| s.to_string()).collect();
                if !uris.is_empty() {
                    w.load_uris(&uris);
                }
            }
        });

        // Keyboard shortcuts
        let get = w!();
        self.window.connect_key_press_event(move |_, ev| {
            if let Some(w) = get() {
                return w.on_key_press(ev);
            }
            glib::Propagation::Proceed
        });

        let get = w!();
        self.window.connect_focus_in_event(move |_, _| {
            if let Some(w) = get() {
                app::add_to_last_active(&w);
                app::reload_settings_if_changed();
            }
            glib::Propagation::Proceed
        });

        let get = w!();
        self.window.connect_window_state_event(move |_, ev| {
            if let Some(w) = get() {
                let s = ev.new_window_state();
                let fs = s.contains(gdk::WindowState::FULLSCREEN);
                let max = s.contains(gdk::WindowState::MAXIMIZED);
                let changed = w.st.borrow().fullscreen != fs;
                {
                    let mut st = w.st.borrow_mut();
                    st.fullscreen = fs;
                    st.maximized = max;
                }
                if changed {
                    w.fullscreen_changed();
                }
            }
            glib::Propagation::Proceed
        });

        let get = w!();
        self.window.connect_configure_event(move |win, _| {
            if let Some(w) = get() {
                let (fs, max) = {
                    let st = w.st.borrow();
                    (st.fullscreen, st.maximized)
                };
                if !fs && !max {
                    let (x, y) = win.position();
                    let (cw, ch) = win.size();
                    w.st.borrow_mut().normal_geometry = Some(Geometry { x, y, w: cw, h: ch, maximized: false });
                }
            }
            false
        });

        let get = w!();
        self.window.connect_delete_event(move |_, _| {
            if let Some(w) = get() {
                w.close_event();
            }
            glib::Propagation::Proceed
        });
    }

    fn close_event(self: &Rc<Self>) {
        let mut g = self.st.borrow().normal_geometry.unwrap_or(Geometry { x: 0, y: 0, w: 640, h: 480, maximized: false });
        g.maximized = self.st.borrow().maximized;
        Store::set("General", "geometry", &qtconf::encode_geometry(&g));
        if let Some(i) = self.info.borrow_mut().take() {
            unsafe { i.window.destroy() };
        }
        for f in self.st.borrow_mut().temp_files.drain(..) {
            let _ = std::fs::remove_file(f);
        }
        if let Some(id) = self.st.borrow_mut().slideshow.take() {
            id.remove();
        }
        app::remove_window(self);
    }

    fn on_key_press(self: &Rc<Self>, ev: &gdk::EventKey) -> glib::Propagation {
        let action = app::get().shortcuts.borrow().action_for_event(ev);
        if let Some(key) = action {
            if self.action_enabled(key) {
                self.trigger(key);
            }
            return glib::Propagation::Stop;
        }
        let kv = ev.keyval();
        if kv == gdk::keys::constants::Escape && self.is_fullscreen() && !app::get().shortcuts.borrow().escape_is_bound() {
            self.toggle_full_screen();
            return glib::Propagation::Stop;
        }
        if kv == gdk::keys::constants::Menu
            || (kv == gdk::keys::constants::F10 && ev.state().contains(gdk::ModifierType::SHIFT_MASK))
        {
            self.show_context_menu(None);
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    }

    pub fn show_context_menu(self: &Rc<Self>, ev: Option<&gdk::EventButton>) {
        let menu = actions::build_context_menu(self);
        menu.set_attach_widget(Some(&self.area));
        {
            let w = Rc::downgrade(self);
            menu.connect_deactivate(move |m| {
                let m = m.clone();
                let w = w.clone();
                // Destroy after the activated item's handler ran
                glib::idle_add_local_once(move || {
                    if let Some(w) = w.upgrade() {
                        if w.context_menu.borrow().as_ref() == Some(&m) {
                            *w.context_menu.borrow_mut() = None;
                        }
                    }
                    unsafe { m.destroy() };
                });
            });
        }
        match ev {
            Some(e) => {
                let ev: &gdk::Event = e;
                menu.popup_at_pointer(Some(ev));
            }
            None => menu.popup_at_widget(&self.area, gdk::Gravity::Center, gdk::Gravity::NorthWest, None),
        }
        *self.context_menu.borrow_mut() = Some(menu);
    }

    // ---- action plumbing ----

    pub fn track_item(&self, key: &'static str, item: &gtk::MenuItem, label: &gtk::AccelLabel) {
        let mut items = self.items.borrow_mut();
        let v = items.entry(key).or_default();
        v.retain(|(i, _)| i.upgrade().is_some());
        v.push((item.downgrade(), label.downgrade()));
    }

    pub fn track_openwith_menu(&self, item: &gtk::MenuItem) {
        let mut v = self.openwith_menus.borrow_mut();
        v.retain(|i| i.upgrade().is_some());
        v.push(item.downgrade());
    }

    pub fn action_enabled(&self, key: &str) -> bool {
        let Some(def) = actions::def(key) else { return true };
        let core = self.core.borrow();
        let d = &core.details;
        match def.enable {
            Enable::Always | Enable::WindowDisable => true,
            Enable::Disable => d.is_pixmap_loaded,
            Enable::GifDisable => d.is_movie_loaded,
            Enable::FolderDisable => !d.folder_file_info_list.is_empty(),
            Enable::UndoDisable => {
                self.st.borrow().last_deleted_files.last().map(|l| !l.path_in_trash.is_empty()).unwrap_or(false)
            }
        }
    }

    /// MainWindow::disableActions
    pub fn disable_actions(&self) {
        let keys: Vec<&'static str> = self.items.borrow().keys().copied().collect();
        for key in keys {
            let enabled = self.action_enabled(key);
            if let Some(v) = self.items.borrow().get(key) {
                for (i, _) in v {
                    if let Some(i) = i.upgrade() {
                        i.set_sensitive(enabled);
                    }
                }
            }
        }
        let loaded = self.core.borrow().details.is_pixmap_loaded;
        for m in self.openwith_menus.borrow().iter() {
            if let Some(m) = m.upgrade() {
                m.set_sensitive(loaded);
            }
        }
    }

    /// Refresh label + icon of every visible copy of an action
    fn refresh_action(&self, key: &str) {
        let text = mnemonic(&actions::action_text(self, key));
        let icons = actions::action_icons(self, key);
        if let Some(v) = self.items.borrow().get(key) {
            for (i, l) in v {
                if let Some(l) = l.upgrade() {
                    l.set_text_with_mnemonic(&text);
                }
                if let Some(i) = i.upgrade() {
                    if let Some(b) = i.child().and_then(|c| c.downcast::<gtk::Box>().ok()) {
                        if let Some(old) = b.children().into_iter().next() {
                            b.remove(&old);
                            let img = actions::icon_image(&icons);
                            b.pack_start(&img, false, false, 0);
                            b.reorder_child(&img, 0);
                            img.show();
                        }
                    }
                }
            }
        }
    }

    /// ActionManager::actionTriggered(action, window)
    pub fn trigger(self: &Rc<Self>, key: &str) {
        match key {
            "quit" => app::quit(),
            "newwindow" => {
                app::new_window();
            }
            "open" => dialogs::pick_file(Some(self)),
            "options" => dialogs::open_options_dialog(Some(&self.window)),
            "about" => dialogs::open_about_dialog(Some(&self.window)),
            "welcome" => dialogs::open_welcome_dialog(Some(&self.window)),
            "clearrecents" => app::clear_recents_list(),
            "openwithother" => {
                let p = self.core.borrow().details.file_path.clone();
                openwith::show_open_with_dialog(&self.window, p);
            }
            "reloadfile" => self.view_reload_file(),
            "opencontainingfolder" => self.open_containing_folder(),
            "showfileinfo" => self.show_file_info(),
            "delete" => self.ask_delete_file(false),
            "deletepermanent" => self.ask_delete_file(true),
            "undo" => self.undo_delete(),
            "copy" => self.copy(),
            "paste" => self.paste(),
            "rename" => self.rename(),
            "zoomin" => self.zoom_in((-1, -1)),
            "zoomout" => self.zoom_out((-1, -1)),
            "resetzoom" => self.reset_zoom(),
            "originalsize" => self.original_size(),
            "rotateright" => self.rotate(90),
            "rotateleft" => self.rotate(-90),
            "mirror" => {
                self.mirror_view();
                self.reset_zoom();
            }
            "flip" => {
                self.flip_view();
                self.reset_zoom();
            }
            "fullscreen" => self.toggle_full_screen(),
            "firstfile" => self.go_to_file(GoToFileMode::First, 0),
            "previousfile" => self.previous_file(),
            "nextfile" => self.next_file(),
            "lastfile" => self.go_to_file(GoToFileMode::Last, 0),
            "randomfile" => self.go_to_file(GoToFileMode::Random, 0),
            "saveframeas" => self.save_frame_as(),
            "pause" => self.pause(),
            "nextframe" => self.next_frame(),
            "decreasespeed" => self.change_speed(-25),
            "resetspeed" => self.set_speed(100),
            "increasespeed" => self.change_speed(25),
            "slideshow" => self.toggle_slideshow(),
            _ => {}
        }
    }

    // ---- MainWindow slots ----

    pub fn open_file(self: &Rc<Self>, file: &str) {
        self.view_load_file(file);
        self.cancel_slideshow();
    }

    pub fn set_just_launched_with_image(&self, v: bool) {
        self.st.borrow_mut().just_launched_with_image = v;
    }

    pub fn is_fullscreen(&self) -> bool {
        self.st.borrow().fullscreen
    }

    pub fn slideshow_active(&self) -> bool {
        self.st.borrow().slideshow.is_some()
    }

    pub fn settings_updated(self: &Rc<Self>) {
        // QVImageCore and QVGraphicsView react first (connection order), then MainWindow
        self.core.borrow_mut().settings_updated();
        self.pump();
        self.view_settings_updated();

        let sm = app::settings();
        self.update_window_title();
        self.st.borrow_mut().custom_bg =
            if sm.bool(Setting::BgColorEnabled) { util::parse_color(&sm.string(Setting::BgColor)) } else { None };

        let menubar = sm.bool(Setting::MenuBarEnabled);
        self.menubar.set_visible(menubar);
        if menubar {
            for c in self.menubar.children() {
                c.show_all();
            }
        }

        if self.slideshow_active() {
            // QTimer::setInterval on an active timer restarts it
            self.stop_slideshow_timer();
            self.start_slideshow_timer();
        }
        self.fullscreen_label.set_visible(sm.bool(Setting::FullScreenDetails) && self.is_fullscreen());
        drop(sm);
        self.set_window_size();
        self.area.queue_draw();
    }

    pub fn shortcuts_updated(self: &Rc<Self>) {
        actions::build_menu_bar(self, &self.menubar);
        self.disable_actions();
        if app::settings().bool(Setting::MenuBarEnabled) {
            for c in self.menubar.children() {
                c.show_all();
            }
        }
    }

    pub fn open_recent(self: &Rc<Self>, i: usize) {
        let list = app::recents_snapshot();
        let path = list.get(i).map(|r| r.1.clone()).unwrap_or_default();
        self.view_load_file(&path);
        self.cancel_slideshow();
    }

    /// MainWindow::fileChanged
    pub fn file_changed(self: &Rc<Self>) {
        self.request_open_with_population();
        self.disable_actions();
        if self.info.borrow().as_ref().map(|i| i.window.is_visible()).unwrap_or(false) {
            self.refresh_properties();
        }
        self.update_window_title();
        self.area.queue_draw();
        // pause text resets with a newly loaded file
        self.refresh_action("pause");
    }

    fn request_open_with_population(self: &Rc<Self>) {
        if let Some(id) = self.st.borrow_mut().open_with_timer.take() {
            id.remove();
        }
        let w = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_millis(250), move || {
            let Some(w) = w.upgrade() else { return };
            w.st.borrow_mut().open_with_timer = None;
            let path = w.core.borrow().details.file_path.clone();
            let req = {
                let mut st = w.st.borrow_mut();
                st.open_with_request += 1;
                st.open_with_request
            };
            let (tx, rx) = async_channel::bounded(1);
            std::thread::spawn(move || {
                let _ = tx.send_blocking(openwith::get_open_with_items(&path));
            });
            let w2 = Rc::downgrade(&w);
            glib::spawn_future_local(async move {
                if let Ok(items) = rx.recv().await {
                    if let Some(w) = w2.upgrade() {
                        if w.st.borrow().open_with_request == req {
                            w.st.borrow_mut().open_with_items = items;
                        }
                    }
                }
            });
        });
        self.st.borrow_mut().open_with_timer = Some(id);
    }

    pub fn open_with_items(&self) -> Vec<OpenWithItem> {
        self.st.borrow().open_with_items.clone()
    }

    pub fn open_with(&self, item: &OpenWithItem) {
        let p = self.core.borrow().details.file_path.clone();
        openwith::open_with(&p, item);
    }

    pub fn refresh_properties(self: &Rc<Self>) {
        let (path, w, h, frames) = {
            let core = self.core.borrow();
            let d = &core.details;
            let frames = if d.is_movie_loaded { core.movie.as_ref().map(|m| m.frame_count()).unwrap_or(0) } else { 0 };
            (d.file_path.clone(), d.base_image_size.0, d.base_image_size.1, frames)
        };
        let mut info = self.info.borrow_mut();
        let info = info.get_or_insert_with(|| dialogs::InfoDialog::new(&self.window));
        info.set_info(&path, w, h, frames as i32);
    }

    pub fn update_window_title(&self) {
        let core = self.core.borrow();
        let d = &core.details;
        let mut s = "g-view".to_string();
        if d.is_file() {
            let idx = d.loaded_index_in_folder + 1;
            let count = d.folder_file_info_list.len();
            match app::settings().int(Setting::TitleBarMode) {
                1 => s = d.file_name(),
                2 => s = format!("{}/{} - {}", idx, count, d.file_name()),
                3 => {
                    s = format!("{}/{} - {}", idx, count, d.file_name());
                    if !d.error_data.has_error {
                        s += &format!(
                            " - {}x{} - {}",
                            d.base_image_size.0,
                            d.base_image_size.1,
                            util::format_bytes(d.file_size())
                        );
                    }
                    s += " - g-view";
                }
                _ => {}
            }
        }
        self.window.set_title(&s);
        self.fullscreen_label.set_text(&s);
    }

    /// MainWindow::setWindowSize
    pub fn set_window_size(&self) {
        let (loaded, size) = {
            let core = self.core.borrow();
            (core.details.is_pixmap_loaded, core.details.loaded_pixmap_size)
        };
        if !loaded {
            return;
        }
        let sm = app::settings();
        let mode = sm.int(Setting::WindowResizeMode);
        let just = self.st.borrow().just_launched_with_image;
        if !(mode == 2 || (mode == 1 && just)) {
            return;
        }
        self.st.borrow_mut().just_launched_with_image = false;
        {
            let st = self.st.borrow();
            if st.maximized || st.fullscreen {
                return;
            }
        }
        let min_pct = sm.int(Setting::MinWindowResizedPercentage) as f64 / 100.0;
        let max_pct = sm.int(Setting::MaxWindowResizedPercentage) as f64 / 100.0;
        let mut img = (size.0 as i64 - 4, size.1 as i64 - 4);

        let Some(display) = gdk::Display::default() else { return };
        let monitor = self
            .window
            .window()
            .and_then(|gw| display.monitor_at_window(&gw))
            .or_else(|| display.primary_monitor())
            .or_else(|| display.monitor(0));
        let Some(monitor) = monitor else { return };
        let geo = monitor.geometry();
        let work = monitor.workarea();

        let mut extra_h = 0i64;
        if self.menubar.is_visible() {
            let h = self.menubar.allocated_height();
            extra_h += if h > 1 { h as i64 } else { self.menubar.preferred_height().1 as i64 };
        }
        let (cw, ch) = self.window.size();
        let frame = self.window.window().map(|gw| gw.frame_extents()).map(|r| (r.width() - cw, r.height() - ch)).unwrap_or((0, 0));
        let frame = (frame.0.clamp(0, 200) as i64, frame.1.clamp(0, 200) as i64);
        let hard = (work.width() as i64 - frame.0, work.height() as i64 - frame.1 - extra_h);
        let screen = (geo.width() as f64, geo.height() as f64);
        let bounded = |p: f64| -> (i64, i64) { (((screen.0 * p).round() as i64).min(hard.0), ((screen.1 * p).round() as i64).min(hard.1)) };
        let min_size = bounded(min_pct);
        let max_size = bounded(max_pct);

        let scale_keep = |s: (i64, i64), to: (i64, i64)| -> (i64, i64) {
            if s.0 <= 0 || s.1 <= 0 {
                return to;
            }
            let rw = to.1 * s.0 / s.1;
            if rw <= to.0 {
                (rw, to.1)
            } else {
                (to.0, to.0 * s.1 / s.0)
            }
        };
        if img.0 < min_size.0 && img.1 < min_size.1 {
            img = scale_keep(img, min_size);
        } else if img.0 > max_size.0 || img.1 > max_size.1 {
            img = scale_keep(img, max_size);
        }
        let new_w = img.0.max(1) as i32;
        let new_h = (img.1 + extra_h).max(1) as i32;

        // Wayland: windows can't position themselves; the compositor places them
        self.window.resize(new_w, new_h);
    }

    fn fullscreen_changed(self: &Rc<Self>) {
        self.refresh_action("fullscreen");
        self.fullscreen_label.set_visible(self.is_fullscreen() && app::settings().bool(Setting::FullScreenDetails));
    }

    pub fn toggle_full_screen(self: &Rc<Self>) {
        if self.is_fullscreen() {
            self.window.unfullscreen();
            if self.st.borrow().stored_maximized {
                self.window.maximize();
            }
        } else {
            let max = self.st.borrow().maximized;
            self.st.borrow_mut().stored_maximized = max;
            self.window.fullscreen();
        }
    }

    pub fn reset_zoom(self: &Rc<Self>) {
        self.reset_scale();
    }

    fn rotate(self: &Rc<Self>, deg: i32) {
        self.core.borrow_mut().rotate_image(deg);
        self.pump();
        self.reset_zoom();
    }

    pub fn previous_file(self: &Rc<Self>) {
        self.go_to_file(GoToFileMode::Previous, 0);
    }

    pub fn next_file(self: &Rc<Self>) {
        self.go_to_file(GoToFileMode::Next, 0);
    }

    // ---- slideshow ----

    fn start_slideshow_timer(self: &Rc<Self>) {
        let secs = app::settings().double(Setting::SlideshowTimer);
        let ms = ((secs * 1000.0) as i64).max(1) as u64;
        let w = Rc::downgrade(self);
        let id = glib::timeout_add_local(Duration::from_millis(ms), move || {
            if let Some(w) = w.upgrade() {
                if app::settings().bool(Setting::SlideshowReversed) {
                    w.previous_file();
                } else {
                    w.next_file();
                }
                glib::ControlFlow::Continue
            } else {
                glib::ControlFlow::Break
            }
        });
        self.st.borrow_mut().slideshow = Some(id);
    }

    fn stop_slideshow_timer(&self) {
        if let Some(id) = self.st.borrow_mut().slideshow.take() {
            id.remove();
        }
    }

    pub fn toggle_slideshow(self: &Rc<Self>) {
        if self.slideshow_active() {
            self.stop_slideshow_timer();
        } else {
            self.start_slideshow_timer();
        }
        self.refresh_action("slideshow");
    }

    pub fn cancel_slideshow(self: &Rc<Self>) {
        if self.slideshow_active() {
            self.toggle_slideshow();
        }
    }

    // ---- animation ----

    fn pause(self: &Rc<Self>) {
        if !self.core.borrow().details.is_movie_loaded {
            return;
        }
        let paused = self.core.borrow().is_paused();
        self.core.borrow_mut().set_paused(!paused);
        self.refresh_action("pause");
        self.pump();
    }

    fn next_frame(self: &Rc<Self>) {
        if !self.core.borrow().details.is_movie_loaded {
            return;
        }
        self.core.borrow_mut().jump_to_next_frame();
        self.pump();
    }

    fn set_speed(self: &Rc<Self>, s: i32) {
        if !self.core.borrow().details.is_movie_loaded {
            return;
        }
        self.core.borrow_mut().set_speed(s);
        self.pump();
    }

    fn change_speed(self: &Rc<Self>, delta: i32) {
        let s = self.core.borrow().speed();
        self.set_speed(s + delta);
    }

    fn save_frame_as(self: &Rc<Self>) {
        if !self.core.borrow().details.is_movie_loaded {
            return;
        }
        if !self.core.borrow().is_paused() {
            self.pause();
        }
        let (path, frame_no) = {
            let core = self.core.borrow();
            (core.details.file_path.clone(), core.movie.as_ref().map(|m| m.current).unwrap_or(0))
        };
        let dialog = gtk::FileChooserDialog::with_buttons(
            Some(&tr("MainWindow", "Save Frame As...")),
            Some(&self.window),
            gtk::FileChooserAction::Save,
            &[("_Cancel", gtk::ResponseType::Cancel), ("_Save", gtk::ResponseType::Accept)],
        );
        dialog.set_do_overwrite_confirmation(true);
        let dir = Store::get("recents", "lastFileDialogDir").unwrap_or_else(|| glib::home_dir().to_string_lossy().to_string());
        dialog.set_current_folder(&dir);
        dialog.set_current_name(&format!("{}-{}.png", util::base_name(&path), frame_no));
        for f in dialogs::name_filters() {
            dialog.add_filter(f);
        }
        let w = Rc::downgrade(self);
        dialog.connect_response(move |d, r| {
            if r == gtk::ResponseType::Accept {
                if let (Some(w), Some(file)) = (w.upgrade(), d.filename()) {
                    let mut file = file;
                    if file.extension().is_none() {
                        file.set_extension("png");
                    }
                    let frame = w.core.borrow().movie.as_ref().map(|m| m.current_image().clone());
                    if let Some(img) = frame {
                        let rgba = crate::decode::cairo_to_rgba(&img.data);
                        if let Some(buf) = image::RgbaImage::from_raw(img.width as u32, img.height as u32, rgba) {
                            if let Err(e) = buf.save(&file) {
                                dialogs::error(&w.window, &tr("MainWindow", "Error"), &e.to_string());
                            }
                        }
                    }
                    w.reset_scale();
                }
            }
            d.close();
        });
        dialog.show();
    }

    // ---- file operations ----

    fn open_containing_folder(&self) {
        if !self.core.borrow().details.is_pixmap_loaded {
            return;
        }
        let dir = util::parent_dir(&self.core.borrow().details.file_path);
        let uri = glib::filename_to_uri(&dir, None).map(|s| s.to_string()).unwrap_or_default();
        if let Err(e) = gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>) {
            eprintln!("{}", e);
        }
    }

    fn show_file_info(self: &Rc<Self>) {
        self.refresh_properties();
        if let Some(i) = self.info.borrow().as_ref() {
            i.window.show_all();
            i.window.present();
        }
    }

    fn ask_delete_file(self: &Rc<Self>, permanent: bool) {
        if !permanent && !app::settings().bool(Setting::AskDelete) {
            self.delete_file(permanent);
            return;
        }
        let path = self.core.borrow().details.file_path.clone();
        let file_name = util::file_name(&path);
        if !is_writable(&path) {
            dialogs::error(
                &self.window,
                &tr("MainWindow", "Error"),
                &tr("MainWindow", "Can't delete %1:\nNo write permission or file is read-only.").replace("%1", &file_name),
            );
            return;
        }
        let text = if permanent {
            tr("MainWindow", "Are you sure you want to delete %1 permanently? This can't be undone.").replace("%1", &file_name)
        } else {
            tr("MainWindow", "Are you sure you want to move %1 to the Trash?").replace("%1", &file_name)
        };
        let md = gtk::MessageDialog::new(
            Some(&self.window),
            gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT,
            gtk::MessageType::Question,
            gtk::ButtonsType::YesNo,
            &text,
        );
        md.set_title(&tr("MainWindow", "Delete"));
        md.set_default_response(gtk::ResponseType::Yes);
        let check = gtk::CheckButton::with_mnemonic(&mnemonic(&tr("MainWindow", "Do not ask again")));
        if !permanent {
            if let Ok(b) = md.message_area().downcast::<gtk::Box>() {
                b.pack_end(&check, false, false, 0);
            }
            check.show();
        }
        let w = Rc::downgrade(self);
        md.connect_response(move |d, r| {
            d.close();
            if r != gtk::ResponseType::Yes {
                return;
            }
            let Some(w) = w.upgrade() else { return };
            if !permanent {
                Store::set("options", "askdelete", if check.is_active() { "false" } else { "true" });
                app::load_settings();
            }
            w.delete_file(permanent);
        });
        md.show();
    }

    fn delete_file(self: &Rc<Self>, permanent: bool) {
        let path = self.core.borrow().details.file_path.clone();
        let file_name = util::file_name(&path);
        self.core.borrow_mut().close_image();
        self.pump();

        let mut trash_path = String::new();
        let success = if permanent {
            std::fs::remove_file(&path).is_ok()
        } else {
            match crate::trash::move_to_trash(&path) {
                Some(t) => {
                    trash_path = t;
                    true
                }
                None => false,
            }
        };
        if !success || std::path::Path::new(&path).exists() {
            self.open_file(&path);
            dialogs::error(&self.window, &tr("MainWindow", "Error"), &tr("MainWindow", "Can't delete %1.").replace("%1", &file_name));
            return;
        }
        let after = app::settings().int(Setting::AfterDelete);
        if after > 1 {
            self.next_file();
        } else if after < 1 {
            self.previous_file();
        }
        if !trash_path.is_empty() {
            self.st.borrow_mut().last_deleted_files.push(DeletedPaths { path_in_trash: trash_path, previous_path: path });
        }
        self.disable_actions();
    }

    fn undo_delete(self: &Rc<Self>) {
        let Some(last) = self.st.borrow_mut().last_deleted_files.pop() else { return };
        if last.path_in_trash.is_empty() || last.previous_path.is_empty() {
            return;
        }
        let name = util::file_name(&last.path_in_trash);
        if !is_writable(&last.path_in_trash) {
            dialogs::error(
                &self.window,
                &tr("MainWindow", "Error"),
                &tr("MainWindow", "Can't undo deletion of %1:\nNo write permission or file is read-only.").replace("%1", &name),
            );
            return;
        }
        if !crate::trash::restore(&last.path_in_trash, &last.previous_path) {
            dialogs::error(&self.window, &tr("MainWindow", "Error"), &tr("MainWindow", "Failed undoing deletion of %1.").replace("%1", &name));
        }
        self.open_file(&last.previous_path);
        self.disable_actions();
    }

    fn copy(&self) {
        let (path, pixmap) = {
            let core = self.core.borrow();
            if !core.details.is_pixmap_loaded {
                return;
            }
            (core.details.file_path.clone(), core.loaded_pixmap.clone())
        };
        let Some(surf) = pixmap else { return };
        let Some(pb) = gdk::pixbuf_get_from_surface(&surf, 0, 0, surf.width(), surf.height()) else { return };
        let uri = glib::filename_to_uri(&path, None).map(|s| s.to_string()).unwrap_or_default();
        let clipboard = gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD);
        let targets = [
            gtk::TargetEntry::new("image/png", gtk::TargetFlags::empty(), 1),
            gtk::TargetEntry::new("image/bmp", gtk::TargetFlags::empty(), 1),
            gtk::TargetEntry::new("image/jpeg", gtk::TargetFlags::empty(), 1),
            gtk::TargetEntry::new("image/tiff", gtk::TargetFlags::empty(), 1),
            gtk::TargetEntry::new("text/uri-list", gtk::TargetFlags::empty(), 2),
            gtk::TargetEntry::new("x-special/gnome-copied-files", gtk::TargetFlags::empty(), 3),
        ];
        let _ = clipboard.set_with_data(&targets, move |_, sel, info| match info {
            1 => {
                sel.set_pixbuf(&pb);
            }
            2 => {
                sel.set_uris(&[uri.as_str()]);
            }
            _ => {
                let data = format!("copy\n{}", uri);
                sel.set(&sel.target(), 8, data.as_bytes());
            }
        });
    }

    fn paste(self: &Rc<Self>) {
        let clipboard = gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD);
        let uris: Vec<String> = clipboard.wait_for_uris().iter().map(|s| s.to_string()).collect();
        if !uris.is_empty() {
            self.load_uris(&uris);
        }
    }

    fn rename(self: &Rc<Self>) {
        if !self.core.borrow().details.is_pixmap_loaded {
            return;
        }
        let path = self.core.borrow().details.file_path.clone();
        let w = Rc::downgrade(self);
        dialogs::rename_dialog(&self.window, &path, move |new_path| {
            if let Some(w) = w.upgrade() {
                w.open_file(&new_path);
            }
        });
    }

}

fn is_writable(path: &str) -> bool {
    let c = std::ffi::CString::new(path).unwrap_or_default();
    extern "C" {
        fn access(path: *const std::ffi::c_char, mode: i32) -> i32;
    }
    unsafe { access(c.as_ptr(), 2) == 0 }
}
