//! Dialogs: options, shortcut editor, file info, rename, open URL, about, welcome, file picker.

use crate::app;
use crate::i18n::{self, mnemonic, plain, tr};
use crate::settings::{self, Store, Value};
use crate::shortcuts;
use crate::util;
use crate::window::MainWindow;
use gtk::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub fn error(parent: &gtk::Window, title: &str, text: &str) {
    let md = gtk::MessageDialog::new(
        Some(parent),
        gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT,
        gtk::MessageType::Error,
        gtk::ButtonsType::Ok,
        text,
    );
    md.set_title(title);
    md.connect_response(|d, _| d.close());
    md.show();
}

pub fn info(parent: Option<&gtk::Window>, title: &str, text: &str) {
    let md = gtk::MessageDialog::new(
        parent,
        gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT,
        gtk::MessageType::Info,
        gtk::ButtonsType::Ok,
        text,
    );
    md.set_title(title);
    md.connect_response(|d, _| d.close());
    md.show();
}

/// "Supported Images (*.png ...)" and "All Files (*)"
pub fn name_filters() -> Vec<gtk::FileFilter> {
    let (exts, _) = app::format_lists();
    let supported = gtk::FileFilter::new();
    let mut desc = String::new();
    for e in exts.iter() {
        desc.push_str(&format!("*{} ", e));
        // case-insensitive pattern
        let pat: String = e
            .chars()
            .map(|c| if c.is_ascii_alphabetic() { format!("[{}{}]", c.to_ascii_lowercase(), c.to_ascii_uppercase()) } else { c.to_string() })
            .collect();
        supported.add_pattern(&format!("*{}", pat));
    }
    supported.set_name(Some(&format!("{} ({})", tr("QVApplication", "Supported Images"), desc.trim_end())));
    let all = gtk::FileFilter::new();
    all.add_pattern("*");
    all.set_name(Some(&format!("{} (*)", tr("QVApplication", "All Files"))));
    vec![supported, all]
}

/// QVApplication::pickFile
pub fn pick_file(parent: Option<&Rc<MainWindow>>) {
    let dialog = gtk::FileChooserDialog::with_buttons(
        Some(&tr("QVApplication", "Open...")),
        parent.map(|p| &p.window),
        gtk::FileChooserAction::Open,
        &[("_Cancel", gtk::ResponseType::Cancel), ("_Open", gtk::ResponseType::Accept)],
    );
    dialog.set_modal(parent.is_some());
    dialog.set_select_multiple(true);
    let dir = Store::get("recents", "lastFileDialogDir").unwrap_or_else(|| glib::home_dir().to_string_lossy().to_string());
    dialog.set_current_folder(&dir);
    for f in name_filters() {
        dialog.add_filter(f);
    }
    let parent = parent.map(Rc::downgrade);
    dialog.connect_response(move |d, r| {
        let files: Vec<String> = d.filenames().iter().map(|p| p.to_string_lossy().to_string()).collect();
        d.close();
        if r != gtk::ResponseType::Accept || files.is_empty() {
            return;
        }
        let parent = parent.as_ref().and_then(|p| p.upgrade());
        for (i, f) in files.iter().enumerate() {
            match (&parent, i) {
                (Some(p), 0) => p.open_file(f),
                _ => app::open_file(f, true),
            }
        }
        Store::set("recents", "lastFileDialogDir", &util::parent_dir(&files[0]));
    });
    dialog.show();
}

pub fn input_dialog<F: Fn(String) + 'static>(parent: &gtk::Window, title: &str, label: &str, initial: &str, on_ok: F) -> (gtk::Dialog, gtk::Entry) {
    let d = gtk::Dialog::with_buttons(
        Some(title),
        Some(parent),
        gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT,
        &[("_Cancel", gtk::ResponseType::Cancel), ("_OK", gtk::ResponseType::Ok)],
    );
    d.set_default_size(350, -1);
    d.set_default_response(gtk::ResponseType::Ok);
    let b = d.content_area();
    b.set_spacing(6);
    b.set_margin_start(10);
    b.set_margin_end(10);
    b.set_margin_top(10);
    let l = gtk::Label::new(Some(label));
    l.set_xalign(0.0);
    let e = gtk::Entry::new();
    e.set_text(initial);
    e.set_activates_default(true);
    b.pack_start(&l, false, false, 0);
    b.pack_start(&e, false, false, 0);
    let e2 = e.clone();
    d.connect_response(move |d, r| {
        let text = e2.text().to_string();
        d.close();
        if r == gtk::ResponseType::Ok {
            on_ok(text);
        }
    });
    d.show_all();
    (d, e)
}

/// QVRenameDialog
pub fn rename_dialog<F: Fn(String) + 'static>(parent: &gtk::Window, path: &str, on_renamed: F) {
    let p = path.to_string();
    let parent2 = parent.clone();
    let name = util::file_name(path);
    let (_, entry) = input_dialog(parent, &tr("QVRenameDialog", "Rename..."), &tr("QVRenameDialog", "File name:"), &name, move |new_name| {
        let fname = util::file_name(&p);
        let dir = util::parent_dir(&p);
        let c = std::ffi::CString::new(p.clone()).unwrap_or_default();
        extern "C" {
            fn access(path: *const std::ffi::c_char, mode: i32) -> i32;
        }
        if unsafe { access(c.as_ptr(), 2) } != 0 {
            error(
                &parent2,
                &tr("QVRenameDialog", "Error"),
                &tr("QVRenameDialog", "Could not rename %1:\nNo write permission or file is read-only.").replace("%1", &fname),
            );
            return;
        }
        let new_path = util::absolute_clean(&format!("{}/{}", dir, new_name));
        if new_path != p {
            if std::fs::rename(&p, &new_path).is_ok() {
                on_renamed(new_path);
            } else {
                error(
                    &parent2,
                    &tr("QVRenameDialog", "Error"),
                    &tr("QVRenameDialog", "Could not rename %1:\n(Check that all characters are valid)").replace("%1", &fname),
                );
            }
        }
    });
    // Select the name without extension
    if let Some(dot) = name.rfind('.') {
        let chars = name[..dot].chars().count() as i32;
        let e = entry.clone();
        glib::idle_add_local_once(move || e.select_region(0, chars));
    }
}

// ---- File info (QVInfoDialog) ---------------------------------------------------------------

pub struct InfoDialog {
    pub window: gtk::Window,
    labels: HashMap<&'static str, gtk::Label>,
    frames_title: gtk::Label,
}

fn gcd(a: i32, b: i32) -> i32 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

impl InfoDialog {
    pub fn new(parent: &gtk::Window) -> InfoDialog {
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_title(&tr("QVInfoDialog", "File Info"));
        window.set_transient_for(Some(parent));
        window.set_type_hint(gdk::WindowTypeHint::Dialog);
        window.set_resizable(false);
        window.connect_delete_event(|w, _| {
            w.hide();
            glib::Propagation::Stop
        });
        let grid = gtk::Grid::new();
        grid.set_row_spacing(6);
        grid.set_column_spacing(12);
        grid.set_margin_start(12);
        grid.set_margin_end(12);
        grid.set_margin_top(12);
        grid.set_margin_bottom(12);
        let rows: [(&'static str, &str); 8] = [
            ("name", "Name:"),
            ("type", "Type:"),
            ("location", "Location:"),
            ("size", "Size:"),
            ("modified", "Modified:"),
            ("dimensions", "Dimensions:"),
            ("ratio", "Aspect Ratio:"),
            ("frames", "Frames:"),
        ];
        let mut labels = HashMap::new();
        let mut frames_title = gtk::Label::new(None);
        for (i, (key, title)) in rows.iter().enumerate() {
            let t = gtk::Label::new(Some(&tr("QVInfoDialog", title)));
            t.set_xalign(1.0);
            let v = gtk::Label::new(Some("error"));
            v.set_xalign(0.0);
            v.set_selectable(true);
            v.set_max_width_chars(60);
            v.set_line_wrap(true);
            v.set_line_wrap_mode(pango::WrapMode::WordChar);
            grid.attach(&t, 0, i as i32, 1, 1);
            grid.attach(&v, 1, i as i32, 1, 1);
            if *key == "frames" {
                frames_title = t.clone();
            }
            labels.insert(*key, v);
        }
        window.add(&grid);
        InfoDialog { window, labels, frames_title }
    }

    pub fn set_info(&self, path: &str, width: i32, height: i32, frame_count: i32) {
        let meta = std::fs::metadata(path).ok();
        let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        let mime = crate::imagecore::mime_for_file(path, true);
        let megapixels = (((width as f64 * height as f64) / 1_000_000.0 * 10.0 + 0.5).round()) / 10.0;
        self.labels["name"].set_text(&util::file_name(path));
        self.labels["type"].set_text(&mime);
        self.labels["location"].set_text(&util::parent_dir(path));
        self.labels["size"].set_text(
            &tr("QVInfoDialog", "%1 (%2 bytes)").replace("%1", &util::format_bytes(size)).replace("%2", &util::group_digits(size)),
        );
        let modified = meta
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|d| glib::DateTime::from_unix_local(d.as_secs() as i64).ok())
            .and_then(|dt| dt.format("%A, %B %-d, %Y %-I:%M:%S %p %Z").ok())
            .map(|s| s.to_string())
            .unwrap_or_default();
        self.labels["modified"].set_text(&modified);
        let mp = if megapixels.fract() == 0.0 { format!("{}", megapixels as i64) } else { format!("{}", megapixels) };
        self.labels["dimensions"].set_text(
            &tr("QVInfoDialog", "%1 x %2 (%3 MP)").replace("%1", &width.to_string()).replace("%2", &height.to_string()).replace("%3", &mp),
        );
        let g = gcd(width, height);
        if g != 0 {
            self.labels["ratio"].set_text(&format!("{}:{}", width / g, height / g));
        }
        if frame_count != 0 {
            self.labels["frames"].set_text(&frame_count.to_string());
            self.labels["frames"].show();
            self.frames_title.show();
        } else {
            self.labels["frames"].hide();
            self.frames_title.hide();
            self.labels["frames"].set_no_show_all(true);
            self.frames_title.set_no_show_all(true);
        }
        self.window.resize(1, 1);
    }
}

// ---- Shortcut editor (QVShortcutDialog) ------------------------------------------------------

fn shortcut_dialog(
    parent: &gtk::Dialog,
    index: usize,
    transient: Rc<RefCell<Vec<Vec<String>>>>,
    on_changed: impl Fn(usize, Vec<String>) + 'static,
) {
    let (name, readable, defaults) = {
        let a = app::get();
        let sm = a.shortcuts.borrow();
        let s = &sm.list[index];
        (s.name, s.readable_name.clone(), s.default_shortcuts.clone())
    };
    let _ = readable;
    let d = gtk::Dialog::new();
    d.set_title(&tr("QVShortcutDialog", "Modify Shortcuts"));
    d.set_transient_for(Some(parent));
    d.set_modal(true);
    d.set_default_size(320, -1);
    let defaults_btn = d.add_button(&mnemonic(&tr("QVOptionsDialog", "Restore &Defaults")), gtk::ResponseType::Other(1));
    let _ = defaults_btn;
    d.add_button("_Cancel", gtk::ResponseType::Cancel);
    d.add_button("_OK", gtk::ResponseType::Ok);
    d.set_default_response(gtk::ResponseType::Ok);

    // QKeySequenceEdit: records up to 4 key combos, finishing 1s after the last key press
    let entry = gtk::Entry::new();
    entry.set_editable(false);
    entry.set_icon_from_icon_name(gtk::EntryIconPosition::Secondary, Some("edit-clear-symbolic"));
    let current: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(transient.borrow()[index].clone()));
    let recording = Rc::new(RefCell::new(false));
    let finish_timer: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
    entry.set_text(&shortcuts::readable(&current.borrow()));
    {
        let current = current.clone();
        entry.connect_icon_press(move |e, _, _| {
            current.borrow_mut().clear();
            e.set_text("");
        });
    }
    {
        let current = current.clone();
        let recording = recording.clone();
        let finish_timer = finish_timer.clone();
        entry.connect_key_press_event(move |e, ev| {
            let Some(combo) = shortcuts::record_combo(ev) else { return glib::Propagation::Stop };
            if !*recording.borrow() {
                current.borrow_mut().clear();
                *recording.borrow_mut() = true;
            }
            {
                let mut c = current.borrow_mut();
                if c.len() < 4 {
                    c.push(combo.to_qt_string());
                }
            }
            e.set_text(&shortcuts::readable(&current.borrow()));
            if let Some(id) = finish_timer.borrow_mut().take() {
                id.remove();
            }
            let recording2 = recording.clone();
            let ft = finish_timer.clone();
            let id = glib::timeout_add_local_once(std::time::Duration::from_millis(1000), move || {
                *recording2.borrow_mut() = false;
                *ft.borrow_mut() = None;
            });
            *finish_timer.borrow_mut() = Some(id);
            if current.borrow().len() >= 4 {
                *recording.borrow_mut() = false;
            }
            glib::Propagation::Stop
        });
    }
    let b = d.content_area();
    b.set_margin_start(10);
    b.set_margin_end(10);
    b.set_margin_top(10);
    b.set_margin_bottom(6);
    b.pack_start(&entry, false, false, 0);

    let entry2 = entry.clone();
    d.connect_response(move |d, r| {
        match r {
            gtk::ResponseType::Other(1) => {
                *current.borrow_mut() = defaults.clone();
                entry2.set_text(&shortcuts::readable(&defaults));
            }
            gtk::ResponseType::Ok => {
                let chosen = current.borrow().clone();
                // shortcutAlreadyBound
                let a = app::get();
                let sm = a.shortcuts.borrow();
                for seq in &chosen {
                    let Some(kc) = shortcuts::KeyCombo::parse(seq) else { continue };
                    for (i, s) in sm.list.iter().enumerate() {
                        if s.name == name {
                            continue;
                        }
                        let t = transient.borrow();
                        if t[i].iter().filter_map(|x| shortcuts::KeyCombo::parse(x)).any(|k| k == kc) {
                            let md = gtk::MessageDialog::new(
                                Some(d),
                                gtk::DialogFlags::MODAL,
                                gtk::MessageType::Warning,
                                gtk::ButtonsType::Ok,
                                &tr("QVShortcutDialog", "\"%1\" is already bound to \"%2\"").replace("%1", seq).replace("%2", &s.readable_name),
                            );
                            md.set_title(&tr("QVShortcutDialog", "Shortcut Already Used"));
                            md.connect_response(|m, _| m.close());
                            md.show();
                            return;
                        }
                    }
                }
                drop(sm);
                on_changed(index, chosen);
                d.close();
            }
            _ => d.close(),
        }
    });
    d.show_all();
    entry.grab_focus();
}

// ---- Options (QVOptionsDialog) -----------------------------------------------------------------

enum Ctl {
    Check(gtk::CheckButton),
    Radio(Vec<gtk::RadioButton>),
    Combo(gtk::ComboBoxText),
    ComboData(gtk::ComboBoxText),
    Spin(gtk::SpinButton),
    DSpin(gtk::SpinButton),
}

struct Options {
    dialog: gtk::Dialog,
    transient: RefCell<HashMap<String, Value>>,
    transient_shortcuts: Rc<RefCell<Vec<Vec<String>>>>,
    controls: RefCell<Vec<(String, Ctl)>>,
    bg_button: gtk::Button,
    bg_swatch: gtk::DrawingArea,
    apply: gtk::Widget,
    defaults: gtk::Widget,
    shortcut_store: gtk::ListStore,
    syncing: RefCell<bool>,
    language_message_shown: RefCell<bool>,
    list: gtk::ListBox,
}

pub fn open_options_dialog(parent: Option<&gtk::Window>) {
    let a = app::get();
    if let Some(d) = a.options_dialog.borrow().as_ref() {
        d.present();
        return;
    }
    let opts = build_options(parent);
    *a.options_dialog.borrow_mut() = Some(opts.dialog.clone());
    opts.dialog.connect_destroy(|_| {
        *app::get().options_dialog.borrow_mut() = None;
    });
    opts.dialog.show_all();
}

fn tro(s: &str) -> String {
    tr("QVOptionsDialog", s)
}

fn material_icon(code: char) -> gtk::Label {
    let l = gtk::Label::new(None);
    l.set_markup(&format!("<span font_family=\"Material Icons Outlined\" size=\"18000\">{}</span>", code));
    l.set_size_request(24, 24);
    l
}

fn grid() -> gtk::Grid {
    let g = gtk::Grid::new();
    g.set_row_spacing(6);
    g.set_column_spacing(12);
    g.set_margin_start(10);
    g.set_margin_end(10);
    g.set_margin_top(8);
    g.set_margin_bottom(8);
    g
}

fn label(text: &str, tooltip: Option<&str>) -> gtk::Label {
    let l = gtk::Label::new(Some(&plain(&tro(text))));
    l.set_xalign(1.0);
    l.set_valign(gtk::Align::Center);
    if let Some(t) = tooltip {
        l.set_tooltip_text(Some(&tro(t)));
    }
    l
}

fn check(text: &str, tooltip: Option<&str>) -> gtk::CheckButton {
    let c = gtk::CheckButton::with_mnemonic(&mnemonic(&tro(text)));
    if let Some(t) = tooltip {
        c.set_tooltip_text(Some(&tro(t)));
    }
    c
}

fn combo(items: &[&str], tooltip: Option<&str>) -> gtk::ComboBoxText {
    let c = gtk::ComboBoxText::new();
    for i in items {
        c.append_text(&tro(i));
    }
    if let Some(t) = tooltip {
        c.set_tooltip_text(Some(&tro(t)));
    }
    c.set_halign(gtk::Align::Start);
    c
}

fn spin(min: f64, max: f64, step: f64, digits: u32) -> gtk::SpinButton {
    let s = gtk::SpinButton::with_range(min, max, step);
    s.set_digits(digits);
    s.set_halign(gtk::Align::Start);
    s
}

/// Label to the right of a spin box for the Qt suffix ("% of screen size", " sec")
fn with_suffix(w: &impl IsA<gtk::Widget>, suffix: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    b.pack_start(w, false, false, 0);
    b.pack_start(&gtk::Label::new(Some(suffix.trim())), false, false, 0);
    b
}

fn scrolled(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    let sw = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    sw.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    sw.add(child);
    sw
}

fn build_options(parent: Option<&gtk::Window>) -> Rc<Options> {
    let dialog = gtk::Dialog::new();
    dialog.set_title(&tro("Settings"));
    if let Some(p) = parent {
        dialog.set_transient_for(Some(p));
        dialog.set_modal(true);
    }
    dialog.set_default_size(640, 530);
    let defaults = dialog.add_button(&mnemonic(&tro("Restore &Defaults")), gtk::ResponseType::Other(1));
    let apply = dialog.add_button("_Apply", gtk::ResponseType::Apply);
    dialog.add_button("_Cancel", gtk::ResponseType::Cancel);
    dialog.add_button("_OK", gtk::ResponseType::Ok);

    let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Single);
    let stack = gtk::Stack::new();
    stack.set_hexpand(true);
    stack.set_vexpand(true);
    for (code, text) in [('\u{e069}', "Window"), ('\u{e3f4}', "Image"), ('\u{e429}', "Miscellaneous"), ('\u{e312}', "Shortcuts")] {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.set_margin_start(6);
        row.set_margin_end(9);
        row.set_margin_top(4);
        row.set_margin_bottom(4);
        row.pack_start(&material_icon(code), false, false, 0);
        let l = gtk::Label::new(Some(&tro(text)));
        l.set_xalign(0.0);
        row.pack_start(&l, true, true, 0);
        list.add(&row);
    }
    let list_frame = gtk::Frame::new(None);
    list_frame.add(&list);
    hbox.pack_start(&list_frame, false, false, 0);
    hbox.pack_start(&stack, true, true, 0);
    hbox.set_margin_start(8);
    hbox.set_margin_end(8);
    hbox.set_margin_top(8);
    dialog.content_area().pack_start(&hbox, true, true, 0);

    let mut controls: Vec<(String, Ctl)> = vec![];

    // ---- Window page ----
    let g = grid();
    let bg_check = check("Back&ground color:", None);
    let bg_button = gtk::Button::new();
    let bg_swatch = gtk::DrawingArea::new();
    bg_swatch.set_size_request(16, 16);
    let bg_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let bg_label = gtk::Label::new(None);
    bg_box.pack_start(&bg_swatch, false, false, 0);
    bg_box.pack_start(&bg_label, false, false, 0);
    bg_button.add(&bg_box);
    bg_button.set_halign(gtk::Align::Start);
    g.attach(&bg_check, 0, 0, 1, 1);
    g.attach(&bg_button, 1, 0, 1, 1);
    controls.push(("bgcolorenabled".into(), Ctl::Check(bg_check.clone())));

    g.attach(&label("Titlebar text:", Some("Changes the amount of information displayed in the titlebar")), 0, 1, 1, 1);
    let r0 = gtk::RadioButton::with_mnemonic(&mnemonic(&tro("&Basic")));
    let r1 = gtk::RadioButton::with_mnemonic_from_widget(&r0, &mnemonic(&tro("&Minimal")));
    let r2 = gtk::RadioButton::with_mnemonic_from_widget(&r0, &mnemonic(&tro("&Practical")));
    let r3 = gtk::RadioButton::with_mnemonic_from_widget(&r0, &mnemonic(&tro("&Verbose")));
    for (i, r) in [&r0, &r1, &r2, &r3].iter().enumerate() {
        g.attach(*r, 1, 1 + i as i32, 1, 1);
    }
    controls.push(("titlebarmode".into(), Ctl::Radio(vec![r0, r1, r2, r3])));

    let resize_tip = "Control when the window should resize to fit the image's actual size";
    g.attach(&label("Window matches image size:", Some(resize_tip)), 0, 6, 1, 1);
    let resize_combo = combo(&["Never", "When launching", "When opening images"], Some(resize_tip));
    g.attach(&resize_combo, 1, 6, 1, 1);
    controls.push(("windowresizemode".into(), Ctl::Combo(resize_combo.clone())));
    let min_label = label("Minimum size:", None);
    let min_spin = spin(0.0, 100.0, 5.0, 0);
    min_spin.set_tooltip_text(Some(&tro("Control the minimum size that the window should reach when matching the image's actual size")));
    g.attach(&min_label, 0, 7, 1, 1);
    g.attach(&with_suffix(&min_spin, &tro("% of screen size")), 1, 7, 1, 1);
    controls.push(("minwindowresizedpercentage".into(), Ctl::Spin(min_spin.clone())));
    let max_label = label("Maximum size:", Some("Control the maximum size that the window should reach when matching the image's actual size"));
    let max_spin = spin(0.0, 100.0, 5.0, 0);
    max_spin.set_tooltip_text(Some(&tro("Control the maximum size that the window should reach when matching the image's actual size")));
    g.attach(&max_label, 0, 8, 1, 1);
    g.attach(&with_suffix(&max_spin, &tro("% of screen size")), 1, 8, 1, 1);
    controls.push(("maxwindowresizedpercentage".into(), Ctl::Spin(max_spin.clone())));

    let menubar = check("Show menubar", None);
    g.attach(&menubar, 1, 10, 1, 1);
    controls.push(("menubarenabled".into(), Ctl::Check(menubar)));
    // hidetitlebar / forcedarkmode / quitonlastwindow are macOS-only and hidden on Linux,
    // but their values are still kept in the config.
    let details = check("Show titlebar text in fullscreen", Some("Choose whether or not to display the titlebar text while in fullscreen"));
    g.attach(&details, 1, 13, 1, 1);
    controls.push(("fullscreendetails".into(), Ctl::Check(details)));
    let ctrl_drag = gtk::CheckButton::with_mnemonic(&mnemonic(&tro("%1 + drag to move window").replace("%1", "Ctrl")));
    ctrl_drag.set_tooltip_text(Some(&tro("Enable %1 + drag to move the window").replace("%1", "Ctrl")));
    g.attach(&ctrl_drag, 1, 15, 1, 1);
    controls.push(("ctrldragwindow".into(), Ctl::Check(ctrl_drag)));
    stack.add_named(&scrolled(&g), "window");

    // ---- Image page ----
    let g = grid();
    g.attach(&label("Scaling:", None), 0, 0, 1, 1);
    let filtering = check("&Bilinear filtering", Some("Turn this off to see individual pixels"));
    g.attach(&filtering, 1, 0, 1, 1);
    controls.push(("filteringenabled".into(), Ctl::Check(filtering)));
    let scaling = check("&Image scaling", Some("Images appear aliased (having jagged edges) without this, but it is faster"));
    g.attach(&scaling, 1, 1, 1, 1);
    controls.push(("scalingenabled".into(), Ctl::Check(scaling.clone())));
    let scaling_two = check(
        "&Scaling above window size",
        Some("Choose whether or not the image continues to be scaled when zooming above the window size (can be laggier with large images)"),
    );
    g.attach(&scaling_two, 1, 2, 1, 1);
    controls.push(("scalingtwoenabled".into(), Ctl::Check(scaling_two.clone())));
    g.attach(&label("Zoom amount:", Some("The amount to zoom every scroll wheel click")), 0, 4, 1, 1);
    let scale_factor = spin(1.0, 200.0, 5.0, 0);
    scale_factor.set_tooltip_text(Some(&tro("The amount to zoom every scroll wheel click")));
    g.attach(&with_suffix(&scale_factor, "%"), 1, 4, 1, 1);
    controls.push(("scalefactor".into(), Ctl::Spin(scale_factor)));
    g.attach(&label("Scrolling behavior:", Some("When enabled, the scrolling action zooms the image")), 0, 5, 1, 1);
    let scroll_combo = combo(&["Zoom", "Auto", "Scroll"], None);
    scroll_combo.set_tooltip_text(Some(&tro("Auto")));
    g.attach(&scroll_combo, 1, 5, 1, 1);
    controls.push(("scrollzoom".into(), Ctl::Combo(scroll_combo.clone())));
    let cursor_zoom = check("Zoom &towards cursor", Some("Choose whether or not zooming in and out above 100% zoom will zoom towards the cursor"));
    g.attach(&cursor_zoom, 1, 6, 1, 1);
    controls.push(("cursorzoom".into(), Ctl::Check(cursor_zoom)));
    let fractional = check(
        "Fractional zooming",
        Some("Use hardware fractional scroll wheel steps when zooming, or always zoom with distinct steps"),
    );
    g.attach(&fractional, 1, 7, 1, 1);
    controls.push(("fractionalzoom".into(), Ctl::Check(fractional.clone())));
    let crop_tip = "Ignores select sides of an image when fitting to window (some sides will extend beyond the window boundaries)";
    g.attach(&label("On window resize:", Some(crop_tip)), 0, 9, 1, 1);
    let crop = combo(&["Fit whole image", "Fit height", "Fit width"], Some(crop_tip));
    g.attach(&crop, 1, 9, 1, 1);
    controls.push(("cropmode".into(), Ctl::Combo(crop)));
    let past = check(
        "Image resizes &past actual size",
        Some("Stop the image from going past its actual size when resizing the window - you can still zoom past it though"),
    );
    g.attach(&past, 1, 10, 1, 1);
    controls.push(("pastactualsizeenabled".into(), Ctl::Check(past)));
    stack.add_named(&scrolled(&g), "image");

    // ---- Miscellaneous page ----
    let g = grid();
    g.attach(&label("Language:", None), 0, 0, 1, 1);
    let lang = gtk::ComboBoxText::new();
    lang.set_halign(gtk::Align::Start);
    lang.append(Some("system"), &tro("System Language"));
    lang.append(Some("en"), "English (en)");
    for (code, _) in i18n::TS_FILES {
        lang.append(Some(code), &format!("{} ({})", i18n::native_name(code), code));
    }
    g.attach(&lang, 1, 0, 1, 1);
    controls.push(("language".into(), Ctl::ComboData(lang.clone())));
    g.attach(&label("Sort files by:", None), 0, 1, 1, 1);
    let sort = combo(&["Name", "Date Modified", "Date Created", "Size", "Type", "Random"], None);
    g.attach(&sort, 1, 1, 1, 1);
    controls.push(("sortmode".into(), Ctl::Combo(sort)));
    let asc = gtk::RadioButton::with_mnemonic(&mnemonic(&tro("A&scending")));
    let desc = gtk::RadioButton::with_mnemonic_from_widget(&asc, &mnemonic(&tro("D&escending")));
    g.attach(&asc, 1, 2, 1, 1);
    g.attach(&desc, 1, 3, 1, 1);
    controls.push(("sortdescending".into(), Ctl::Radio(vec![asc, desc])));
    g.attach(&label("Preloading:", Some("Controls the amount of images preloaded")), 0, 5, 1, 1);
    let preload = combo(&["Disabled", "Adjacent", "Extended"], Some("Controls the amount of images preloaded"));
    g.attach(&preload, 1, 5, 1, 1);
    controls.push(("preloadingmode".into(), Ctl::Combo(preload)));
    let loop_ = check(
        "&Loop through folders",
        Some("Controls whether or not qView should go back to the first item after reaching the end of a folder"),
    );
    g.attach(&loop_, 1, 6, 1, 1);
    controls.push(("loopfoldersenabled".into(), Ctl::Check(loop_)));
    g.attach(&label("Slideshow direction:", None), 0, 8, 1, 1);
    let ss_dir = combo(&["Forward", "Backward"], None);
    g.attach(&ss_dir, 1, 8, 1, 1);
    controls.push(("slideshowreversed".into(), Ctl::Combo(ss_dir)));
    g.attach(&label("Slideshow timer:", None), 0, 9, 1, 1);
    let ss_timer = spin(0.1, 999.9, 0.1, 1);
    g.attach(&with_suffix(&ss_timer, &tro(" sec")), 1, 9, 1, 1);
    controls.push(("slideshowtimer".into(), Ctl::DSpin(ss_timer)));
    g.attach(&label("After deletion:", None), 0, 11, 1, 1);
    let after = combo(&["Move Back", "Do Nothing", "Move Forward"], None);
    g.attach(&after, 1, 11, 1, 1);
    controls.push(("afterdelete".into(), Ctl::Combo(after)));
    let ask = check("&Ask before deleting files", None);
    g.attach(&ask, 1, 12, 1, 1);
    controls.push(("askdelete".into(), Ctl::Check(ask)));
    let mime = check(
        "Allow &MIME content detection",
        Some("Detect supported files in folder even if extension isn't recognized (may be slow with larger/network folders)"),
    );
    g.attach(&mime, 1, 14, 1, 1);
    controls.push(("allowmimecontentdetection".into(), Ctl::Check(mime)));
    let recents = check("Save &recent files", None);
    g.attach(&recents, 1, 15, 1, 1);
    controls.push(("saverecents".into(), Ctl::Check(recents)));
    let skip = check("Skip hidden files", Some("Skip hidden files when browsing to the next/previous file"));
    g.attach(&skip, 1, 17, 1, 1);
    controls.push(("skiphidden".into(), Ctl::Check(skip)));
    stack.add_named(&scrolled(&g), "misc");

    // ---- Shortcuts page ----
    let store = gtk::ListStore::new(&[String::static_type(), String::static_type()]);
    let tree = gtk::TreeView::with_model(&store);
    for (i, title) in ["Action", "Shortcuts"].iter().enumerate() {
        let col = gtk::TreeViewColumn::new();
        col.set_title(&tro(title));
        let cell = gtk::CellRendererText::new();
        gtk::prelude::CellLayoutExt::pack_start(&col, &cell, true);
        gtk::prelude::CellLayoutExt::add_attribute(&col, &cell, "text", i as i32);
        if i == 0 {
            col.set_fixed_width(150);
            col.set_resizable(true);
        }
        tree.append_column(&col);
    }
    let sw = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    sw.add(&tree);
    sw.set_margin_start(10);
    stack.add_named(&sw, "shortcuts");

    let opts = Rc::new(Options {
        dialog: dialog.clone(),
        transient: RefCell::new(HashMap::new()),
        transient_shortcuts: Rc::new(RefCell::new(vec![])),
        controls: RefCell::new(controls),
        bg_button: bg_button.clone(),
        bg_swatch: bg_swatch.clone(),
        apply,
        defaults,
        shortcut_store: store,
        syncing: RefCell::new(false),
        language_message_shown: RefCell::new(false),
        list: list.clone(),
    });

    // bg color swatch
    {
        let o = Rc::downgrade(&opts);
        bg_swatch.connect_draw(move |w, cr| {
            if let Some(o) = o.upgrade() {
                let c = o.transient.borrow().get("bgcolor").map(|v| v.to_str()).unwrap_or_default();
                if let Some((r, g, b)) = util::parse_color(&c) {
                    cr.set_source_rgb(r, g, b);
                    cr.rectangle(0.0, 0.0, w.allocated_width() as f64, w.allocated_height() as f64);
                    let _ = cr.fill();
                }
            }
            glib::Propagation::Stop
        });
        let o = Rc::downgrade(&opts);
        let bl = bg_label.clone();
        bg_button.connect_clicked(move |b| {
            let Some(o) = o.upgrade() else { return };
            let cur = o.transient.borrow().get("bgcolor").map(|v| v.to_str()).unwrap_or_default();
            let cd = gtk::ColorChooserDialog::new(None, b.toplevel().and_then(|t| t.downcast::<gtk::Window>().ok()).as_ref());
            cd.set_use_alpha(false);
            if let Ok(rgba) = gdk::RGBA::parse(&cur) {
                cd.set_rgba(&rgba);
            }
            let o2 = Rc::downgrade(&o);
            let bl = bl.clone();
            cd.connect_response(move |cd, r| {
                if r == gtk::ResponseType::Ok {
                    if let Some(o) = o2.upgrade() {
                        let c = cd.rgba();
                        let name = format!(
                            "#{:02x}{:02x}{:02x}",
                            (c.red() * 255.0).round() as u8,
                            (c.green() * 255.0).round() as u8,
                            (c.blue() * 255.0).round() as u8
                        );
                        bl.set_text(&name);
                        o.modify_setting("bgcolor", Value::Str(name));
                        o.bg_swatch.queue_draw();
                    }
                }
                cd.close();
            });
            cd.show();
        });
        let bl = bg_label.clone();
        let o = Rc::downgrade(&opts);
        // keep the label in sync with transient value after sync_settings
        opts.dialog.connect_show(move |_| {
            if let Some(o) = o.upgrade() {
                bl.set_text(&o.transient.borrow().get("bgcolor").map(|v| v.to_str()).unwrap_or_default());
            }
        });
        let _ = bl;
    }
    *opts.transient.borrow_mut() = HashMap::new();

    // Category list
    {
        let st = stack.clone();
        list.connect_row_selected(move |_, row| {
            if let Some(r) = row {
                let name = ["window", "image", "misc", "shortcuts"][r.index().clamp(0, 3) as usize];
                st.set_visible_child_name(name);
            }
        });
        let tab = Store::get("General", "optionstab").and_then(|s| s.parse::<i32>().ok()).unwrap_or(1);
        if let Some(r) = list.row_at_index(tab.clamp(0, 3)) {
            list.select_row(Some(&r));
        }
        stack.set_visible_child_name(["window", "image", "misc", "shortcuts"][tab.clamp(0, 3) as usize]);
    }

    opts.sync_settings(false, true);

    // Dependent enable states
    {
        let (bg_check2, bg_button2) = (bg_check.clone(), bg_button.clone());
        bg_check.connect_toggled(move |c| bg_button2.set_sensitive(c.is_active()));
        let _ = bg_check2;
        let st2 = scaling_two.clone();
        scaling.connect_toggled(move |c| st2.set_sensitive(c.is_active()));
        let (ml, ms, xl, xs) = (min_label.clone(), min_spin.clone(), max_label.clone(), max_spin.clone());
        let update = move |idx: Option<u32>| {
            let en = idx != Some(0);
            ml.set_sensitive(en);
            ms.set_sensitive(en);
            xl.set_sensitive(en);
            xs.set_sensitive(en);
        };
        update(resize_combo.active());
        resize_combo.connect_changed(move |c| update(c.active()));
        let fz = fractional.clone();
        fz.set_sensitive(scroll_combo.active() != Some(2));
        scroll_combo.connect_changed(move |c| fz.set_sensitive(c.active() != Some(2)));
        let o = Rc::downgrade(&opts);
        lang.connect_changed(move |_| {
            let Some(o) = o.upgrade() else { return };
            if *o.syncing.borrow() || *o.language_message_shown.borrow() {
                return;
            }
            *o.language_message_shown.borrow_mut() = true;
            info(Some(o.dialog.upcast_ref()), &tro("Restart Required"), &tro("You must restart qView to change the language."));
        });
    }

    opts.sync_shortcuts(false);
    {
        let o = Rc::downgrade(&opts);
        tree.connect_row_activated(move |_, path, _| {
            let Some(o) = o.upgrade() else { return };
            let idx = path.indices().first().copied().unwrap_or(0) as usize;
            let o2 = Rc::downgrade(&o);
            shortcut_dialog(&o.dialog, idx, o.transient_shortcuts.clone(), move |i, list| {
                if let Some(o) = o2.upgrade() {
                    o.transient_shortcuts.borrow_mut()[i] = list;
                    o.update_shortcuts_table();
                }
            });
        });
    }
    opts.update_button_box();

    {
        let o = Rc::downgrade(&opts);
        let keep = opts.clone();
        dialog.connect_response(move |d, r| {
            let _ = &keep;
            let Some(o) = o.upgrade() else { return };
            match r {
                gtk::ResponseType::Ok => {
                    o.save_settings();
                    o.done();
                    d.close();
                }
                gtk::ResponseType::Apply => {
                    o.save_settings();
                    o.update_button_box();
                }
                gtk::ResponseType::Other(1) => {
                    o.sync_settings(true, false);
                    o.sync_shortcuts(true);
                }
                _ => {
                    o.done();
                    d.close();
                }
            }
        });
        let o = Rc::downgrade(&opts);
        dialog.connect_delete_event(move |_, _| {
            if let Some(o) = o.upgrade() {
                o.done();
            }
            glib::Propagation::Proceed
        });
    }
    opts
}

impl Options {
    fn done(&self) {
        let row = self.list.selected_row().map(|r| r.index()).unwrap_or(1);
        Store::set("General", "optionstab", &row.to_string());
    }

    fn modify_setting(&self, key: &str, value: Value) {
        if *self.syncing.borrow() {
            return;
        }
        self.transient.borrow_mut().insert(key.to_string(), value);
        self.update_button_box();
    }

    fn save_settings(&self) {
        let values: Vec<(String, String)> = self.transient.borrow().iter().map(|(k, v)| (k.clone(), v.to_str())).collect();
        Store::set_many("options", &values);
        {
            let a = app::get();
            let sm = a.shortcuts.borrow();
            let kf = Store::load();
            for (i, s) in sm.list.iter().enumerate() {
                if let Some(list) = self.transient_shortcuts.borrow().get(i) {
                    let refs: Vec<&str> = list.iter().map(|s| s.as_str()).collect();
                    crate::settings::kf_set_list(&kf, "shortcuts", s.name, &refs);
                }
            }
            Store::write_keyfile(&kf);
        }
        app::shortcuts_updated();
        app::load_settings();
        self.update_button_box();
    }

    fn sync_settings(self: &Rc<Self>, defaults: bool, make_connections: bool) {
        *self.syncing.borrow_mut() = true;
        let sm = app::settings();
        sm.load_settings();
        let get = |k: &str| sm.get_by_key(k, defaults);
        for (key, ctl) in self.controls.borrow().iter() {
            let val = get(key);
            let tv = match ctl {
                Ctl::Check(c) => {
                    c.set_active(val.to_bool());
                    Value::Bool(val.to_bool())
                }
                Ctl::Radio(rs) => {
                    let i = val.to_int().clamp(0, rs.len() as i32 - 1);
                    rs[i as usize].set_active(true);
                    Value::Int(val.to_int())
                }
                Ctl::Combo(c) => {
                    c.set_active(Some(val.to_int().max(0) as u32));
                    Value::Int(val.to_int())
                }
                Ctl::ComboData(c) => {
                    c.set_active_id(Some(&val.to_str()));
                    Value::Str(val.to_str())
                }
                Ctl::Spin(s) => {
                    s.set_value(val.to_int() as f64);
                    Value::Int(val.to_int())
                }
                Ctl::DSpin(s) => {
                    s.set_value(val.to_double());
                    Value::Double(val.to_double())
                }
            };
            self.transient.borrow_mut().insert(key.clone(), tv);
        }
        // bgcolor
        let bg = get("bgcolor").to_str();
        self.transient.borrow_mut().insert("bgcolor".into(), Value::Str(bg.clone()));
        if let Some(l) = self
            .bg_button
            .child()
            .and_then(|c| c.downcast::<gtk::Box>().ok())
            .and_then(|b| b.children().into_iter().find_map(|c| c.downcast::<gtk::Label>().ok()))
        {
            l.set_text(&bg);
        }
        self.bg_button.set_sensitive(get("bgcolorenabled").to_bool());
        self.bg_swatch.queue_draw();
        // hidden macOS-only settings keep their saved values
        for k in ["hidetitlebar", "forcedarkmode", "quitonlastwindow"] {
            self.transient.borrow_mut().insert(k.into(), get(k));
        }
        drop(sm);

        if make_connections {
            for (key, ctl) in self.controls.borrow().iter() {
                let o = Rc::downgrade(self);
                let k = key.clone();
                match ctl {
                    Ctl::Check(c) => {
                        c.connect_toggled(move |c| {
                            if let Some(o) = o.upgrade() {
                                o.modify_setting(&k, Value::Bool(c.is_active()));
                            }
                        });
                    }
                    Ctl::Radio(rs) => {
                        for (i, r) in rs.iter().enumerate() {
                            let o = o.clone();
                            let k = k.clone();
                            r.connect_toggled(move |r| {
                                if r.is_active() {
                                    if let Some(o) = o.upgrade() {
                                        let v = if k == "sortdescending" { Value::Bool(i == 1) } else { Value::Int(i as i32) };
                                        o.modify_setting(&k, v);
                                    }
                                }
                            });
                        }
                    }
                    Ctl::Combo(c) => {
                        c.connect_changed(move |c| {
                            if let Some(o) = o.upgrade() {
                                let i = c.active().unwrap_or(0) as i32;
                                let v = if k == "slideshowreversed" { Value::Bool(i == 1) } else { Value::Int(i) };
                                o.modify_setting(&k, v);
                            }
                        });
                    }
                    Ctl::ComboData(c) => {
                        c.connect_changed(move |c| {
                            if let Some(o) = o.upgrade() {
                                o.modify_setting(&k, Value::Str(c.active_id().map(|s| s.to_string()).unwrap_or_default()));
                            }
                        });
                    }
                    Ctl::Spin(s) => {
                        s.connect_value_changed(move |s| {
                            if let Some(o) = o.upgrade() {
                                o.modify_setting(&k, Value::Int(s.value_as_int()));
                            }
                        });
                    }
                    Ctl::DSpin(s) => {
                        s.connect_value_changed(move |s| {
                            if let Some(o) = o.upgrade() {
                                o.modify_setting(&k, Value::Double((s.value() * 10.0).round() / 10.0));
                            }
                        });
                    }
                }
            }
        }
        *self.syncing.borrow_mut() = false;
        self.update_button_box();
    }

    fn sync_shortcuts(&self, defaults: bool) {
        app::get().shortcuts.borrow_mut().update_shortcuts();
        let a = app::get();
        let sm = a.shortcuts.borrow();
        let mut t = self.transient_shortcuts.borrow_mut();
        t.clear();
        self.shortcut_store.clear();
        for s in &sm.list {
            let list = if defaults { s.default_shortcuts.clone() } else { s.shortcuts.clone() };
            self.shortcut_store.insert_with_values(None, &[(0, &s.readable_name), (1, &shortcuts::readable(&list))]);
            t.push(list);
        }
        drop(t);
        drop(sm);
        self.update_button_box();
    }

    fn update_shortcuts_table(&self) {
        let t = self.transient_shortcuts.borrow();
        let mut i = 0;
        if let Some(iter) = self.shortcut_store.iter_first() {
            loop {
                if let Some(l) = t.get(i) {
                    self.shortcut_store.set_value(&iter, 1, &shortcuts::readable(l).to_value());
                }
                i += 1;
                if !self.shortcut_store.iter_next(&iter) {
                    break;
                }
            }
        }
        drop(t);
        self.update_button_box();
    }

    fn update_button_box(&self) {
        let sm = app::settings();
        let mut apply = false;
        let mut defaults = false;
        for (k, v) in self.transient.borrow().iter() {
            let Some(def) = settings::def_for_key(k) else { continue };
            if !v.same(&sm.get(def.setting, false)) {
                apply = true;
            }
            if !v.same(&sm.get(def.setting, true)) {
                defaults = true;
            }
        }
        let a = app::get();
        let shortcut_mgr = a.shortcuts.borrow();
        for (i, t) in self.transient_shortcuts.borrow().iter().enumerate() {
            if let Some(s) = shortcut_mgr.list.get(i) {
                if *t != s.shortcuts {
                    apply = true;
                }
                if *t != s.default_shortcuts {
                    defaults = true;
                }
            }
        }
        self.apply.set_sensitive(apply);
        self.defaults.set_sensitive(defaults);
    }
}

// ---- About / Welcome --------------------------------------------------------------------------

fn dark_dialog(title: &str, parent: Option<&gtk::Window>) -> gtk::Dialog {
    let d = gtk::Dialog::new();
    d.set_title(title);
    if let Some(p) = parent {
        d.set_transient_for(Some(p));
        d.set_modal(true);
    }
    d.set_resizable(false);
    let css = gtk::CssProvider::new();
    let _ = css.load_from_data(
        b"dialog, dialog box, dialog .dialog-action-area { background-color: #212121; color: #ffffff; } \
          dialog label { color: #ffffff; } dialog checkbutton { color: #ffffff; }",
    );
    d.style_context().add_provider(&css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    fn apply(w: &gtk::Widget, css: &gtk::CssProvider) {
        w.style_context().add_provider(css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        if let Some(c) = w.downcast_ref::<gtk::Container>() {
            for ch in c.children() {
                apply(&ch, css);
            }
        }
    }
    let d2 = d.clone();
    d.connect_show(move |_| apply(d2.upcast_ref(), &css));
    d
}

fn lato_label(text: &str, size_pt: i32, light: bool) -> gtk::Label {
    let l = gtk::Label::new(None);
    let weight = if light { "light" } else { "normal" };
    l.set_markup(&format!(
        "<span font_family=\"Lato\" weight=\"{}\" size=\"{}\">{}</span>",
        weight,
        size_pt * 1024,
        text
    ));
    l.set_justify(gtk::Justification::Center);
    l
}

pub struct AboutDialog {
    pub dialog: gtk::Dialog,
}

fn about_update_text() -> String {
    format!(
        "<span font_family=\"Lato\" size=\"{}\"><a href=\"https://interversehq.com/qview/\"><span color=\"#03A9F4\" underline=\"none\">interversehq.com/qview</span></a></span>",
        10 * 1024
    )
}

pub fn open_about_dialog(parent: Option<&gtk::Window>) {
    let a = app::get();
    if let Some(d) = a.about_dialog.borrow().as_ref() {
        d.dialog.present();
        return;
    }
    let d = dark_dialog(&tr("QVAboutDialog", "About qView"), parent);
    let b = d.content_area();
    b.set_spacing(4);
    b.set_margin_start(30);
    b.set_margin_end(30);
    b.set_margin_top(20);
    b.set_margin_bottom(20);
    b.pack_start(&lato_label("g-view", 72, true), false, false, 0);
    b.pack_start(
        &lato_label(&glib::markup_escape_text(&tr("QVAboutDialog", "version %1").replace("%1", &format!("{:.1}", app::VERSION))), 18, false),
        false,
        false,
        0,
    );
    let update_label = gtk::Label::new(None);
    update_label.set_markup(&about_update_text());
    update_label.set_justify(gtk::Justification::Center);
    b.pack_start(&update_label, false, false, 6);
    let info = gtk::Label::new(None);
    info.set_markup(&format!(
        "<span font_family=\"Lato\" size=\"{}\">Built with GTK {}.{}.{} ({})\nGTK port of qView. Original source code available under GPLv3 on <a href=\"https://github.com/jurplel/qView\"><span color=\"#03A9F4\" underline=\"none\">GitHub</span></a>\nIcon glyph created by Guilhem from the Noun Project\nCopyright © 2018-2025 jurplel and qView contributors</span>",
        8 * 1024,
        gtk::major_version(),
        gtk::minor_version(),
        gtk::micro_version(),
        std::env::consts::ARCH
    ));
    info.set_justify(gtk::Justification::Center);
    b.pack_start(&info, false, false, 0);
    let ad = AboutDialog { dialog: d.clone() };
    *a.about_dialog.borrow_mut() = Some(ad);
    d.connect_destroy(|_| {
        *app::get().about_dialog.borrow_mut() = None;
    });
    d.connect_response(|d, _| d.close());
    d.show_all();
}

pub fn open_welcome_dialog(parent: Option<&gtk::Window>) {
    let a = app::get();
    if let Some(d) = a.welcome_dialog.borrow().as_ref() {
        d.present();
        return;
    }
    let d = dark_dialog(&tr("QVWelcomeDialog", "Welcome"), parent);
    let b = d.content_area();
    b.set_spacing(8);
    b.set_margin_start(30);
    b.set_margin_end(30);
    b.set_margin_top(20);
    b.set_margin_bottom(20);
    b.pack_start(&lato_label(&glib::markup_escape_text(&tr("QVWelcomeDialog", "Welcome")), 54, true), false, false, 0);
    let subtitle = tr("QVWelcomeDialog", "Thank you for downloading qView.<br>Here's a few tips to get you started:").replace("<br>", "\n");
    b.pack_start(&lato_label(&glib::markup_escape_text(&subtitle), 14, false), false, false, 0);
    let tips_html = tr(
        "QVWelcomeDialog",
        "<ul><li>Right click to access the main menu</li><li>Drag the image to reposition it</li><li>Scroll to zoom in and out</li><li>Use arrow keys to switch files</li></ul>",
    );
    let tips: Vec<String> = tips_html
        .split("<li>")
        .skip(1)
        .map(|s| format!("•  {}", s.split("</li>").next().unwrap_or("")))
        .collect();
    let tl = lato_label(&glib::markup_escape_text(&tips.join("\n")), 12, false);
    tl.set_justify(gtk::Justification::Left);
    tl.set_halign(gtk::Align::Center);
    b.pack_start(&tl, false, false, 6);
    *a.welcome_dialog.borrow_mut() = Some(d.clone());
    d.connect_destroy(|_| {
        *app::get().welcome_dialog.borrow_mut() = None;
    });
    d.connect_response(|d, _| d.close());
    d.show_all();
}
