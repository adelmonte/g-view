//! Port of OpenWith (Linux .desktop file implementation) and QVOpenWithDialog.

use crate::i18n::tr;
use crate::util;
use gtk::prelude::*;
use std::path::PathBuf;

#[derive(Clone, Debug, Default)]
pub struct OpenWithItem {
    pub icon_name: String,
    pub name: String,
    pub exec: String,
    pub args: Vec<String>,
    pub categories: Vec<String>,
    pub is_default: bool,
}

fn application_locations() -> Vec<PathBuf> {
    let mut v = vec![glib::user_data_dir().join("applications")];
    for d in glib::system_data_dirs() {
        v.push(d.join("applications"));
    }
    v
}

/// OpenWith::getOpenWithItems (empty path = all applications)
pub fn get_open_with_items(file_path: &str) -> Vec<OpenWithItem> {
    if !file_path.is_empty() && !std::path::Path::new(file_path).exists() {
        return vec![];
    }
    let mut items = get_items_from_desktop_files(file_path);
    items.sort_by(|a, b| util::natural_cmp(&a.name, &b.name));
    if let Some(pos) = items.iter().position(|i| i.is_default) {
        let d = items.remove(pos);
        items.insert(0, d);
    }
    items
}

fn strip_prefix_ci<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    if line.len() >= prefix.len() && line[..prefix.len()].eq_ignore_ascii_case(prefix) {
        Some(&line[prefix.len()..])
    } else {
        None
    }
}

fn get_items_from_desktop_files(file_path: &str) -> Vec<OpenWithItem> {
    let mime_name = if file_path.is_empty() { String::new() } else { crate::imagecore::mime_for_file(file_path, true) };
    let default_app = if mime_name.is_empty() {
        String::new()
    } else {
        let ct = gio::content_type_from_mime_type(&mime_name).map(|s| s.to_string()).unwrap_or(mime_name.clone());
        gio::AppInfo::default_for_type(&ct, false).and_then(|a| a.id()).map(|s| s.to_string()).unwrap_or_default()
    };

    let mut seen = std::collections::HashSet::new();
    let mut out = vec![];
    for loc in application_locations() {
        let Ok(rd) = std::fs::read_dir(&loc) else { continue };
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let fname = entry.file_name().to_string_lossy().to_string();
            if fname == "qView.desktop" || fname == "com.interversehq.qView.desktop" || fname.starts_with("g-view") {
                continue;
            }
            if !fname.ends_with(".desktop") || !seen.insert(fname.clone()) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(entry.path()) else { continue };
            let mut item = OpenWithItem::default();
            let mut mime_types = String::new();
            let mut no_display = false;
            for line in text.lines() {
                if let Some(v) = strip_prefix_ci(line, "Name=") {
                    if item.name.is_empty() {
                        item.name = v.to_string();
                    }
                } else if let Some(v) = strip_prefix_ci(line, "Icon=") {
                    if item.icon_name.is_empty() {
                        item.icon_name = v.to_string();
                    }
                } else if let Some(v) = strip_prefix_ci(line, "Categories=") {
                    if item.categories.is_empty() {
                        item.categories = v.split(';').map(|s| s.to_string()).collect();
                    }
                } else if let Some(v) = strip_prefix_ci(line, "Exec=") {
                    if item.exec.is_empty() {
                        let v = match v.find('%') {
                            Some(p) => &v[..p],
                            None => v,
                        };
                        item.exec = v.to_string();
                    }
                } else if let Some(v) = strip_prefix_ci(line, "MimeType=") {
                    if mime_types.is_empty() {
                        mime_types = v.to_string();
                    }
                } else if let Some(v) = strip_prefix_ci(line, "NoDisplay=").or_else(|| strip_prefix_ci(line, "Hidden=")) {
                    if v.eq_ignore_ascii_case("true") {
                        no_display = true;
                    }
                }
            }
            if (mime_name.is_empty() || mime_types.to_lowercase().contains(&mime_name.to_lowercase())) && !no_display {
                item.is_default = fname == default_app;
                out.push(item);
            }
        }
    }
    out
}

/// OpenWith::openWith
pub fn open_with(file_path: &str, item: &OpenWithItem) {
    let exec = item.exec.trim();
    if exec.is_empty() {
        return;
    }
    let mut argv: Vec<String> = glib::shell_parse_argv(exec)
        .map(|v| v.iter().map(|s| s.to_string_lossy().to_string()).collect())
        .unwrap_or_else(|_| vec![exec.to_string()]);
    argv.extend(item.args.iter().cloned());
    argv.push(file_path.to_string());
    let refs: Vec<&std::ffi::OsStr> = argv.iter().map(|s| std::ffi::OsStr::new(s.as_str())).collect();
    if let Err(e) = gio::Subprocess::newv(&refs, gio::SubprocessFlags::NONE) {
        eprintln!("Failed to launch {}: {}", exec, e);
    }
}

struct Category {
    name: &'static str,
    readable: &'static str,
    icon: &'static str,
}

const CATEGORIES: &[Category] = &[
    Category { name: "Development", readable: "Development", icon: "applications-development" },
    Category { name: "Education", readable: "Education", icon: "applications-education" },
    Category { name: "Game", readable: "Games", icon: "applications-games" },
    Category { name: "Graphics", readable: "Graphics", icon: "applications-graphics" },
    Category { name: "Network", readable: "Internet", icon: "applications-internet" },
    Category { name: "AudioVideo", readable: "Multimedia", icon: "applications-multimedia" },
    Category { name: "Office", readable: "Office", icon: "applications-office" },
    Category { name: "Science", readable: "Science", icon: "applications-science" },
    Category { name: "Settings", readable: "Settings", icon: "preferences-system" },
    Category { name: "System", readable: "System", icon: "applications-system" },
    Category { name: "Utility", readable: "Utilities", icon: "applications-utilities" },
    Category { name: "", readable: "Other", icon: "applications-other" },
];

fn themed_pixbuf(name: &str) -> Option<gdk_pixbuf::Pixbuf> {
    if name.starts_with('/') {
        return gdk_pixbuf::Pixbuf::from_file_at_scale(name, 16, 16, true).ok();
    }
    let theme = gtk::IconTheme::default()?;
    theme.load_icon(name, 16, gtk::IconLookupFlags::FORCE_SIZE).ok().flatten().or_else(|| {
        theme.load_icon(&format!("{}-symbolic", name), 16, gtk::IconLookupFlags::FORCE_SIZE).ok().flatten()
    })
}

/// QVOpenWithDialog ("Other Application...")
pub fn show_open_with_dialog(parent: &gtk::Window, file_path: String) {
    let dialog = gtk::Dialog::with_buttons(
        Some(&tr("QVOpenWithDialog", "Choose Application")),
        Some(parent),
        gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT,
        &[("_Cancel", gtk::ResponseType::Cancel), ("_OK", gtk::ResponseType::Ok)],
    );
    dialog.set_default_size(400, 400);
    let store = gtk::TreeStore::new(&[gdk_pixbuf::Pixbuf::static_type(), String::static_type(), String::static_type()]);
    let mut all = get_open_with_items("");
    for cat in CATEGORIES {
        let mut children = vec![];
        all.retain(|app| {
            if cat.name.is_empty() || app.categories.iter().any(|c| c.eq_ignore_ascii_case(cat.name)) {
                children.push(app.clone());
                false
            } else {
                true
            }
        });
        if children.is_empty() {
            continue;
        }
        let pb = themed_pixbuf(cat.icon).or_else(|| themed_pixbuf("applications-other"));
        let parent_iter = store.insert_with_values(None, None, &[(0, &pb), (1, &tr("QVOpenWithDialog", cat.readable)), (2, &"")]);
        for app in children {
            let pb = themed_pixbuf(&app.icon_name);
            store.insert_with_values(Some(&parent_iter), None, &[(0, &pb), (1, &app.name), (2, &app.exec)]);
        }
    }
    let tree = gtk::TreeView::with_model(&store);
    tree.set_headers_visible(false);
    let col = gtk::TreeViewColumn::new();
    let pr = gtk::CellRendererPixbuf::new();
    let tr_ = gtk::CellRendererText::new();
    gtk::prelude::CellLayoutExt::pack_start(&col, &pr, false);
    gtk::prelude::CellLayoutExt::pack_start(&col, &tr_, true);
    gtk::prelude::CellLayoutExt::add_attribute(&col, &pr, "pixbuf", 0);
    gtk::prelude::CellLayoutExt::add_attribute(&col, &tr_, "text", 1);
    tree.append_column(&col);
    let sw = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    sw.add(&tree);
    sw.set_vexpand(true);
    dialog.content_area().pack_start(&sw, true, true, 0);

    let selected_exec = {
        let tree = tree.clone();
        move || -> Option<String> {
            let (model, iter) = tree.selection().selected()?;
            let exec: String = model.value(&iter, 2).get().ok()?;
            if exec.is_empty() {
                None
            } else {
                Some(exec)
            }
        }
    };
    let launch = {
        let fp = file_path.clone();
        move |exec: String| {
            let mut parts: Vec<String> = exec.split(' ').map(|s| s.to_string()).collect();
            let prog = parts.remove(0);
            open_with(&fp, &OpenWithItem { exec: prog, args: parts.into_iter().filter(|s| !s.is_empty()).collect(), ..Default::default() });
        }
    };
    {
        let d = dialog.clone();
        let sel = selected_exec.clone();
        let launch = launch.clone();
        tree.connect_row_activated(move |_, _, _| {
            if let Some(e) = sel() {
                launch(e);
                d.close();
            }
        });
    }
    dialog.connect_response(move |d, r| {
        if r == gtk::ResponseType::Ok {
            if let Some(e) = selected_exec() {
                launch(e);
            }
        }
        d.close();
    });
    dialog.show_all();
}
