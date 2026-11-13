//! gtk-view: a Rust + GTK3 port of qView (https://github.com/jurplel/qView).

mod actions;
mod app;
mod decode;
mod dialogs;
mod i18n;
mod imagecore;
mod openwith;
mod qtconf;
mod settings;
mod shortcuts;
mod trash;
mod util;
mod view;
mod window;

use settings::Setting;

static FONTS: &[(&str, &[u8])] = &[
    ("Lato-Light.ttf", include_bytes!("../resources/Lato-Light.ttf")),
    ("Lato-Regular.ttf", include_bytes!("../resources/Lato-Regular.ttf")),
    ("MaterialIconsOutlined-Regular.otf", include_bytes!("../resources/MaterialIconsOutlined-Regular.otf")),
];

#[link(name = "fontconfig")]
extern "C" {
    fn FcConfigAppFontAddFile(config: *mut std::ffi::c_void, file: *const std::ffi::c_char) -> i32;
}

/// QVApplication::ensureFontLoaded for the bundled fonts
fn register_fonts() {
    let dir = glib::user_cache_dir().join("gtk-view").join("fonts");
    let _ = std::fs::create_dir_all(&dir);
    for (name, data) in FONTS {
        let p = dir.join(name);
        let up_to_date = std::fs::metadata(&p).map(|m| m.len() == data.len() as u64).unwrap_or(false);
        if !up_to_date {
            let _ = std::fs::write(&p, data);
        }
        if let Ok(c) = std::ffi::CString::new(p.to_string_lossy().as_bytes()) {
            unsafe {
                FcConfigAppFontAddFile(std::ptr::null_mut(), c.as_ptr());
            }
        }
    }
}

fn print_help() {
    println!("Usage: gtk-view [options] file\n");
    println!("Options:");
    println!("  -h, --help     Displays help on commandline options.");
    println!("  -v, --version  Displays version information.\n");
    println!("Arguments:");
    println!("  file           The file to open.");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut file: Option<String> = None;
    let mut only_files = false;
    for a in &args {
        if !only_files && (a == "-h" || a == "--help" || a == "-?") {
            print_help();
            return;
        }
        if !only_files && (a == "-v" || a == "--version") {
            println!("gtk-view {}", settings::fmt_double(app::VERSION));
            return;
        }
        if !only_files && a == "--" {
            only_files = true;
            continue;
        }
        if !only_files && a.starts_with('-') && a.len() > 1 {
            // Ignore GTK/X11 style options we don't know (Qt does the same for its own)
            continue;
        }
        if file.is_none() {
            file = Some(a.clone());
        }
    }

    glib::set_prgname(Some("gtk-view"));
    glib::set_application_name("gtk-view");
    register_fonts();
    qtconf::migrate_if_needed();

    if gtk::init().is_err() {
        eprintln!("gtk-view: cannot open display");
        std::process::exit(1);
    }
    gtk::Window::set_default_icon_name("gtk-view");

    // SettingsManager constructor: load settings, then translation
    let lang = settings::SettingsManager::new().string(Setting::Language);
    i18n::load(&lang);
    app::init();

    let window = app::new_window();
    if let Some(f) = file {
        app::open_file_in(&window, &f, true);
    }

    gtk::main();
}
