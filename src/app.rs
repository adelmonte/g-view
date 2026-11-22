//! Application-wide state (QVApplication + ActionManager's recents list).

use crate::decode;
use gtk::prelude::*;
use crate::settings::{SettingsManager, Store};
use crate::shortcuts::ShortcutManager;
use crate::window::MainWindow;
use std::cell::RefCell;
use std::rc::{Rc, Weak};

pub const VERSION: f64 = 7.0;

pub struct Recent {
    pub file_name: String,
    pub file_path: String,
}

pub struct App {
    pub settings: Rc<SettingsManager>,
    pub shortcuts: RefCell<ShortcutManager>,
    pub windows: RefCell<Vec<Rc<MainWindow>>>,
    pub last_active: RefCell<Vec<Weak<MainWindow>>>,
    pub extensions: Rc<Vec<String>>,
    pub mime_types: Rc<Vec<String>>,
    pub recents: RefCell<Vec<Recent>>,
    pub recents_save_pending: RefCell<Option<glib::SourceId>>,
    pub options_dialog: RefCell<Option<gtk::Dialog>>,
    pub about_dialog: RefCell<Option<crate::dialogs::AboutDialog>>,
    pub welcome_dialog: RefCell<Option<gtk::Dialog>>,
    pub quitting: RefCell<bool>,
}

thread_local! {
    static APP: RefCell<Option<Rc<App>>> = const { RefCell::new(None) };
}

pub fn init() {
    let lists = decode::format_lists();
    let settings = Rc::new(SettingsManager::new());
    let app = Rc::new(App {
        settings,
        shortcuts: RefCell::new(ShortcutManager::new()),
        windows: RefCell::new(vec![]),
        last_active: RefCell::new(vec![]),
        extensions: Rc::new(lists.extensions),
        mime_types: Rc::new(lists.mime_types),
        recents: RefCell::new(vec![]),
        recents_save_pending: RefCell::new(None),
        options_dialog: RefCell::new(None),
        about_dialog: RefCell::new(None),
        welcome_dialog: RefCell::new(None),
        quitting: RefCell::new(false),
    });
    APP.with(|a| *a.borrow_mut() = Some(app));
    load_recents_list();
}

pub fn get() -> Rc<App> {
    APP.with(|a| a.borrow().clone().expect("app not initialized"))
}

pub fn settings() -> Rc<SettingsManager> {
    get().settings.clone()
}

pub fn format_lists() -> (Rc<Vec<String>>, Rc<Vec<String>>) {
    let a = get();
    (a.extensions.clone(), a.mime_types.clone())
}

/// Called on window focus (QEvent::ApplicationStateChange -> loadSettings)
pub fn reload_settings_if_changed() {
    let a = get();
    if a.settings.reload_if_changed() {
        settings_updated();
    }
}

/// Re-read settings from disk and broadcast if changed (SettingsManager::loadSettings)
pub fn load_settings() {
    let a = get();
    if a.settings.load_settings() {
        settings_updated();
    }
}

/// Broadcast settingsUpdated to everything (same connection order as qView)
pub fn settings_updated() {
    let a = get();
    // ActionManager::settingsUpdated
    if !a.settings.bool(crate::settings::Setting::SaveRecents) {
        clear_recents_list();
    }
    let windows: Vec<Rc<MainWindow>> = a.windows.borrow().clone();
    for w in windows {
        w.settings_updated();
    }
}

pub fn shortcuts_updated() {
    let a = get();
    a.shortcuts.borrow_mut().update_shortcuts();
    let windows: Vec<Rc<MainWindow>> = a.windows.borrow().clone();
    for w in windows {
        w.shortcuts_updated();
    }
}

// ---- windows ------------------------------------------------------------------------------

pub fn new_window() -> Rc<MainWindow> {
    let w = MainWindow::new();
    get().windows.borrow_mut().push(w.clone());
    w.window.show_all();
    w.after_show();
    w.window.present();
    w
}

pub fn remove_window(w: &MainWindow) {
    let a = get();
    a.windows.borrow_mut().retain(|x| !std::ptr::eq(x.as_ref(), w));
    a.last_active.borrow_mut().retain(|x| x.upgrade().map(|x| !std::ptr::eq(x.as_ref(), w)).unwrap_or(false));
    if a.windows.borrow().is_empty() {
        gtk::main_quit();
    }
}

pub fn add_to_last_active(w: &Rc<MainWindow>) {
    let a = get();
    let mut l = a.last_active.borrow_mut();
    if let Some(first) = l.first().and_then(|x| x.upgrade()) {
        if Rc::ptr_eq(&first, w) {
            return;
        }
    }
    l.insert(0, Rc::downgrade(w));
    l.truncate(5);
}

/// QVApplication::getMainWindow
pub fn get_main_window(should_be_empty: bool) -> Rc<MainWindow> {
    let a = get();
    let candidates: Vec<Rc<MainWindow>> = a
        .last_active
        .borrow()
        .iter()
        .filter_map(|w| w.upgrade())
        .chain(a.windows.borrow().iter().cloned())
        .collect();
    for w in candidates {
        if !should_be_empty || !w.core.borrow().details.is_load_requested {
            return w;
        }
    }
    new_window()
}

pub fn open_file_in(window: &Rc<MainWindow>, file: &str, resize: bool) {
    window.set_just_launched_with_image(resize);
    window.open_file(file);
}

pub fn open_file(file: &str, resize: bool) {
    let w = get_main_window(true);
    open_file_in(&w, file, resize);
}

pub fn quit() {
    let a = get();
    *a.quitting.borrow_mut() = true;
    let windows: Vec<Rc<MainWindow>> = a.windows.borrow().clone();
    for w in windows {
        w.window.close();
    }
    if a.windows.borrow().is_empty() {
        gtk::main_quit();
    }
}

// ---- recents (ActionManager) ----------------------------------------------------------------

const RECENTS_MAX: usize = 10;

pub fn load_recents_list() {
    let a = get();
    // Prevents weird bugs when opening the recent menu while the save timer is still running
    if a.recents_save_pending.borrow().is_some() {
        return;
    }
    let list = Store::get_list("recents", "recentFiles").unwrap_or_default();
    *a.recents.borrow_mut() = list
        .into_iter()
        .map(|p| Recent { file_name: crate::util::file_name(&p), file_path: p })
        .collect();
    audit_recents_list();
}

fn save_recents_list() {
    audit_recents_list();
    let a = get();
    let paths: Vec<String> = a.recents.borrow().iter().map(|r| r.file_path.clone()).collect();
    Store::set_list("recents", "recentFiles", &paths);
}

pub fn add_file_to_recents_list(path: &str) {
    let a = get();
    a.recents.borrow_mut().insert(0, Recent { file_name: crate::util::file_name(path), file_path: path.to_string() });
    audit_recents_list();
    if let Some(id) = a.recents_save_pending.borrow_mut().take() {
        id.remove();
    }
    let id = glib::timeout_add_local_once(std::time::Duration::from_millis(500), || {
        let a = get();
        *a.recents_save_pending.borrow_mut() = None;
        save_recents_list();
    });
    *a.recents_save_pending.borrow_mut() = Some(id);
}

fn audit_recents_list() {
    let a = get();
    let enabled = a.settings.bool(crate::settings::Setting::SaveRecents);
    let mut l = a.recents.borrow_mut();
    if !enabled {
        l.clear();
    }
    let mut out: Vec<Recent> = vec![];
    for r in l.drain(..) {
        if r.file_path.is_empty() || !std::path::Path::new(&r.file_path).exists() {
            continue;
        }
        if out.iter().any(|o| o.file_path == r.file_path && o.file_name == r.file_name) {
            continue;
        }
        out.push(r);
    }
    out.truncate(RECENTS_MAX);
    *l = out;
}

pub fn clear_recents_list() {
    let a = get();
    a.recents.borrow_mut().clear();
    if let Some(id) = a.recents_save_pending.borrow_mut().take() {
        id.remove();
    }
    save_recents_list();
}

pub fn recents_snapshot() -> Vec<(String, String)> {
    get().recents.borrow().iter().map(|r| (r.file_name.clone(), r.file_path.clone())).collect()
}
