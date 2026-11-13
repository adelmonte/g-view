//! Port of ActionManager's action library and menu builders.

use crate::app;
use crate::i18n::{mnemonic, tr};
use crate::window::MainWindow;
use gtk::prelude::*;
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq)]
pub enum Enable {
    Always,
    Disable,
    GifDisable,
    UndoDisable,
    FolderDisable,
    WindowDisable,
}

pub struct ActionDef {
    pub key: &'static str,
    pub text: &'static str,
    pub icons: &'static [&'static str],
    pub enable: Enable,
}

macro_rules! a {
    ($k:literal, $t:literal, [$($i:literal),*], $e:ident) => {
        ActionDef { key: $k, text: $t, icons: &[$($i),*], enable: Enable::$e }
    };
}

pub static ACTIONS: &[ActionDef] = &[
    a!("quit", "&Quit", ["application-exit"], Always),
    a!("newwindow", "New Window", ["window-new"], Always),
    a!("open", "&Open...", ["document-open"], Always),
    a!("reloadfile", "Re&load File", ["view-refresh"], Disable),
    a!("opencontainingfolder", "Open Containing &Folder", ["document-open"], Disable),
    a!("showfileinfo", "Show File &Info", ["document-properties"], Disable),
    a!("delete", "&Move to Trash", ["edit-delete"], Disable),
    a!("deletepermanent", "Delete Permanently", ["edit-delete"], Disable),
    a!("undo", "&Restore from Trash", ["edit-undo"], UndoDisable),
    a!("copy", "&Copy", ["edit-copy"], Disable),
    a!("paste", "&Paste", ["edit-paste"], Always),
    a!("rename", "R&ename...", ["edit-rename", "document-properties"], Disable),
    a!("zoomin", "Zoom &In", ["zoom-in"], Disable),
    a!("zoomout", "Zoom &Out", ["zoom-out"], Disable),
    a!("resetzoom", "Reset &Zoom", ["zoom-fit-best"], Disable),
    a!("originalsize", "Ori&ginal Size", ["zoom-original"], Disable),
    a!("rotateright", "Rotate &Right", ["object-rotate-right"], Disable),
    a!("rotateleft", "Rotate &Left", ["object-rotate-left"], Disable),
    a!("mirror", "&Mirror", ["object-flip-horizontal"], Disable),
    a!("flip", "&Flip", ["object-flip-vertical"], Disable),
    a!("fullscreen", "Enter F&ull Screen", ["view-fullscreen"], WindowDisable),
    a!("firstfile", "&First File", ["go-first"], FolderDisable),
    a!("previousfile", "Previous Fi&le", ["go-previous"], FolderDisable),
    a!("nextfile", "&Next File", ["go-next"], FolderDisable),
    a!("lastfile", "Las&t File", ["go-last"], FolderDisable),
    a!("randomfile", "Rando&m File", ["media-playlist-shuffle"], FolderDisable),
    a!("saveframeas", "Save Frame &As...", ["document-save-as"], GifDisable),
    a!("pause", "Pa&use", ["media-playback-pause"], GifDisable),
    a!("nextframe", "&Next Frame", ["media-skip-forward"], GifDisable),
    a!("decreasespeed", "&Decrease Speed", ["media-seek-backward"], GifDisable),
    a!("resetspeed", "&Reset Speed", ["media-playback-start"], GifDisable),
    a!("increasespeed", "&Increase Speed", ["media-skip-forward"], GifDisable),
    a!("slideshow", "Start S&lideshow", ["media-playback-start"], Disable),
    a!("options", "&Settings", ["configure", "preferences-other"], Always),
    a!("about", "&About", ["help-about"], Always),
    a!("welcome", "&Welcome", ["help-faq", "help-about"], Always),
    a!("clearrecents", "Clear &Menu", ["edit-delete"], Always),
    a!("openwithother", "Other Application...", ["system-run"], Always),
];

pub fn def(key: &str) -> Option<&'static ActionDef> {
    ACTIONS.iter().find(|a| a.key == key)
}

/// Resolve a themed icon, trying fallbacks and symbolic variants.
pub fn icon_image(names: &[&str]) -> gtk::Image {
    let theme = gtk::IconTheme::default();
    if let Some(theme) = theme {
        for n in names {
            if n.starts_with('/') && std::path::Path::new(n).exists() {
                if let Ok(pb) = gdk_pixbuf::Pixbuf::from_file_at_scale(n, 16, 16, true) {
                    return gtk::Image::from_pixbuf(Some(&pb));
                }
            }
            if theme.has_icon(n) {
                return gtk::Image::from_icon_name(Some(n), gtk::IconSize::Menu);
            }
            let sym = format!("{}-symbolic", n);
            if theme.has_icon(&sym) {
                return gtk::Image::from_icon_name(Some(&sym), gtk::IconSize::Menu);
            }
        }
    }
    let img = gtk::Image::new();
    img.set_pixel_size(16);
    img.set_size_request(16, 16);
    img
}

pub fn gicon_image(icon: &gio::Icon) -> gtk::Image {
    gtk::Image::from_gicon(icon, gtk::IconSize::Menu)
}

/// A menu item with icon, mnemonic label and accelerator text (like a QAction in a QMenu)
pub fn make_item(text: &str, image: gtk::Image, accel: Option<&str>) -> (gtk::MenuItem, gtk::AccelLabel) {
    let item = gtk::MenuItem::new();
    let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let label = gtk::AccelLabel::new("");
    label.set_text_with_mnemonic(&mnemonic(text));
    label.set_use_underline(true);
    label.set_xalign(0.0);
    label.set_accel_widget(Some(&item));
    if let Some(a) = accel.and_then(crate::shortcuts::accel_for) {
        label.set_accel(a.0, a.1);
    }
    hbox.pack_start(&image, false, false, 0);
    hbox.pack_start(&label, true, true, 0);
    item.add(&hbox);
    (item, label)
}

pub fn action_text(win: &MainWindow, key: &str) -> String {
    let base = def(key).map(|d| d.text).unwrap_or("");
    let t = match key {
        "fullscreen" if win.is_fullscreen() => tr("MainWindow", "Exit F&ull Screen"),
        "pause" if win.core.borrow().is_paused() => tr("MainWindow", "Res&ume"),
        "slideshow" if win.slideshow_active() => tr("MainWindow", "Stop S&lideshow"),
        _ => tr("ActionManager", base),
    };
    t
}

pub fn action_icons(win: &MainWindow, key: &str) -> Vec<&'static str> {
    match key {
        "fullscreen" if win.is_fullscreen() => vec!["view-restore"],
        "pause" if win.core.borrow().is_paused() => vec!["media-playback-start"],
        "slideshow" if win.slideshow_active() => vec!["media-playback-stop"],
        _ => def(key).map(|d| d.icons.to_vec()).unwrap_or_default(),
    }
}

/// Add an action clone to a menu
pub fn add_action(menu: &gtk::Menu, win: &Rc<MainWindow>, key: &'static str) -> gtk::MenuItem {
    let shortcut = app::get().shortcuts.borrow().shortcuts_for(key).first().cloned();
    let (item, label) = make_item(&action_text(win, key), icon_image(&action_icons(win, key)), shortcut.as_deref());
    item.set_sensitive(win.action_enabled(key));
    let w = Rc::downgrade(win);
    item.connect_activate(move |_| {
        if let Some(w) = w.upgrade() {
            w.trigger(key);
        }
    });
    menu.append(&item);
    win.track_item(key, &item, &label);
    item
}

fn submenu_item(text: &str, icons: &[&str], add_icon: bool) -> (gtk::MenuItem, gtk::Menu) {
    let img = if add_icon { icon_image(icons) } else { gtk::Image::new() };
    let (item, _) = make_item(text, img, None);
    let menu = gtk::Menu::new();
    item.set_submenu(Some(&menu));
    (item, menu)
}

pub fn build_view_menu(win: &Rc<MainWindow>, add_icon: bool) -> gtk::MenuItem {
    let (item, m) = submenu_item(&tr("ActionManager", "&View"), &["zoom-fit-best"], add_icon);
    for k in ["zoomin", "zoomout", "resetzoom", "originalsize"] {
        add_action(&m, win, k);
    }
    m.append(&gtk::SeparatorMenuItem::new());
    add_action(&m, win, "rotateright");
    add_action(&m, win, "rotateleft");
    m.append(&gtk::SeparatorMenuItem::new());
    add_action(&m, win, "mirror");
    add_action(&m, win, "flip");
    m.append(&gtk::SeparatorMenuItem::new());
    add_action(&m, win, "fullscreen");
    item
}

pub fn build_tools_menu(win: &Rc<MainWindow>, add_icon: bool) -> gtk::MenuItem {
    let (item, m) = submenu_item(&tr("ActionManager", "&Tools"), &["configure", "preferences-other"], add_icon);
    add_action(&m, win, "saveframeas");
    add_action(&m, win, "pause");
    add_action(&m, win, "nextframe");
    m.append(&gtk::SeparatorMenuItem::new());
    add_action(&m, win, "decreasespeed");
    add_action(&m, win, "resetspeed");
    add_action(&m, win, "increasespeed");
    m.append(&gtk::SeparatorMenuItem::new());
    add_action(&m, win, "slideshow");
    add_action(&m, win, "options");
    item
}

pub fn build_help_menu(win: &Rc<MainWindow>, add_icon: bool) -> gtk::MenuItem {
    let (item, m) = submenu_item(&tr("ActionManager", "&Help"), &["help-about"], add_icon);
    add_action(&m, win, "about");
    add_action(&m, win, "welcome");
    item
}

/// "Open Recent" submenu; contents are (re)filled by `fill_recents_menu`
pub fn build_recents_menu(win: &Rc<MainWindow>) -> gtk::MenuItem {
    let (item, m) = submenu_item(&tr("ActionManager", "Open &Recent"), &["document-open-recent"], true);
    fill_recents_menu(win, &m);
    item.set_visible(app::settings().bool(crate::settings::Setting::SaveRecents));
    item.set_no_show_all(!app::settings().bool(crate::settings::Setting::SaveRecents));
    item
}

pub fn fill_recents_menu(win: &Rc<MainWindow>, m: &gtk::Menu) {
    for c in m.children() {
        m.remove(&c);
    }
    app::load_recents_list();
    for (i, (name, path)) in app::recents_snapshot().into_iter().enumerate() {
        let mime = crate::imagecore::mime_for_file(&path, false);
        let ct = gio::content_type_from_mime_type(&mime).map(|s| s.to_string()).unwrap_or(mime);
        let icon = gio::content_type_get_icon(&ct);
        let label = name.replace('_', "__");
        let (item, _) = make_item(&label.replace('&', "&&"), gicon_image(&icon), None);
        let w = Rc::downgrade(win);
        item.connect_activate(move |_| {
            if let Some(w) = w.upgrade() {
                w.open_recent(i);
            }
        });
        m.append(&item);
    }
    m.append(&gtk::SeparatorMenuItem::new());
    add_action(m, win, "clearrecents");
    m.show_all();
}

pub fn build_open_with_menu(win: &Rc<MainWindow>) -> gtk::MenuItem {
    let (item, m) = submenu_item(&tr("ActionManager", "Open With"), &["system-run"], true);
    fill_open_with_menu(win, &m);
    item.set_sensitive(win.core.borrow().details.is_pixmap_loaded);
    win.track_openwith_menu(&item);
    item
}

pub fn fill_open_with_menu(win: &Rc<MainWindow>, m: &gtk::Menu) {
    for c in m.children() {
        m.remove(&c);
    }
    let items = win.open_with_items();
    for (i, it) in items.iter().take(10).enumerate() {
        let image = if it.icon_name.starts_with('/') {
            icon_image(&[it.icon_name.as_str()])
        } else if !it.icon_name.is_empty() {
            icon_image(&[it.icon_name.as_str()])
        } else {
            gtk::Image::new()
        };
        let (mi, _) = make_item(&it.name.replace('_', "__").replace('&', "&&"), image, None);
        let w = Rc::downgrade(win);
        let it2 = it.clone();
        mi.connect_activate(move |_| {
            if let Some(w) = w.upgrade() {
                w.open_with(&it2);
            }
        });
        m.append(&mi);
        if i == 0 && it.is_default {
            m.append(&gtk::SeparatorMenuItem::new());
        }
    }
    m.append(&gtk::SeparatorMenuItem::new());
    add_action(m, win, "openwithother");
    m.show_all();
}

pub fn build_context_menu(win: &Rc<MainWindow>) -> gtk::Menu {
    let menu = gtk::Menu::new();
    add_action(&menu, win, "open");
    menu.append(&build_recents_menu(win));
    menu.append(&build_open_with_menu(win));
    add_action(&menu, win, "opencontainingfolder");
    add_action(&menu, win, "showfileinfo");
    menu.append(&gtk::SeparatorMenuItem::new());
    add_action(&menu, win, "rename");
    add_action(&menu, win, "delete");
    menu.append(&gtk::SeparatorMenuItem::new());
    add_action(&menu, win, "nextfile");
    add_action(&menu, win, "previousfile");
    menu.append(&gtk::SeparatorMenuItem::new());
    menu.append(&build_view_menu(win, true));
    menu.append(&build_tools_menu(win, true));
    menu.append(&build_help_menu(win, true));
    menu.show_all();
    menu
}

pub fn build_menu_bar(win: &Rc<MainWindow>, bar: &gtk::MenuBar) {
    for c in bar.children() {
        bar.remove(&c);
    }
    let top = |text: &str| -> (gtk::MenuItem, gtk::Menu) {
        let item = gtk::MenuItem::with_mnemonic(&mnemonic(text));
        let menu = gtk::Menu::new();
        item.set_submenu(Some(&menu));
        (item, menu)
    };

    // File
    let (file_item, file) = top(&tr("ActionManager", "&File"));
    add_action(&file, win, "open");
    let recents = build_recents_menu(win);
    file.append(&recents);
    add_action(&file, win, "reloadfile");
    file.append(&gtk::SeparatorMenuItem::new());
    let openwith = build_open_with_menu(win);
    file.append(&openwith);
    add_action(&file, win, "opencontainingfolder");
    add_action(&file, win, "showfileinfo");
    file.append(&gtk::SeparatorMenuItem::new());
    add_action(&file, win, "quit");
    {
        // Refresh the dynamic submenus right before the File menu opens (aboutToShow)
        let w = Rc::downgrade(win);
        let recents = recents.clone();
        let openwith = openwith.clone();
        file_item.connect_select(move |_| {
            if let Some(w) = w.upgrade() {
                if let Some(m) = recents.submenu().and_then(|m| m.downcast::<gtk::Menu>().ok()) {
                    fill_recents_menu(&w, &m);
                }
                if let Some(m) = openwith.submenu().and_then(|m| m.downcast::<gtk::Menu>().ok()) {
                    fill_open_with_menu(&w, &m);
                }
            }
        });
    }
    bar.append(&file_item);

    // Edit
    let (edit_item, edit) = top(&tr("ActionManager", "&Edit"));
    add_action(&edit, win, "undo");
    edit.append(&gtk::SeparatorMenuItem::new());
    add_action(&edit, win, "copy");
    add_action(&edit, win, "paste");
    add_action(&edit, win, "rename");
    edit.append(&gtk::SeparatorMenuItem::new());
    add_action(&edit, win, "delete");
    add_action(&edit, win, "deletepermanent");
    bar.append(&edit_item);

    // View
    let view = build_view_menu(win, false);
    if let Some(l) = view.child().and_then(|b| b.downcast::<gtk::Box>().ok()).and_then(|b| b.children().into_iter().nth(1)) {
        let _ = l;
    }
    bar.append(&plain_top_item(view));

    // Go
    let (go_item, go) = top(&tr("ActionManager", "&Go"));
    for k in ["firstfile", "previousfile", "nextfile", "lastfile", "randomfile"] {
        add_action(&go, win, k);
    }
    bar.append(&go_item);

    bar.append(&plain_top_item(build_tools_menu(win, false)));
    bar.append(&plain_top_item(build_help_menu(win, false)));
    for c in bar.children() {
        c.show_all();
    }
}

/// Menubar entries shouldn't carry the icon column the context-menu versions have
fn plain_top_item(item: gtk::MenuItem) -> gtk::MenuItem {
    let label_text = item
        .child()
        .and_then(|c| c.downcast::<gtk::Box>().ok())
        .and_then(|b| b.children().into_iter().find_map(|c| c.downcast::<gtk::AccelLabel>().ok()))
        .map(|l| l.label().to_string())
        .unwrap_or_default();
    let top = gtk::MenuItem::with_mnemonic(&label_text);
    if let Some(sub) = item.submenu() {
        item.set_submenu(None::<&gtk::Menu>);
        if let Ok(sub) = sub.downcast::<gtk::Menu>() {
            top.set_submenu(Some(&sub));
        }
    }
    top
}
