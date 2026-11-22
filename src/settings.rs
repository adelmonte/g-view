//! Port of qView's SettingsManager plus the on-disk config store.
//!
//! qView keeps everything in a QSettings INI file. We keep the same groups and keys
//! ([General], [options], [recents], [shortcuts]) in a GLib key file of our own at
//! `$XDG_CONFIG_HOME/g-view/g-view.conf`. On first run an existing qView config is
//! migrated into it (see `qtconf.rs`).

use std::cell::RefCell;
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Setting {
    // Window settings
    BgColorEnabled = 0,
    BgColor,
    TitleBarMode,
    WindowResizeMode,
    MinWindowResizedPercentage,
    MaxWindowResizedPercentage,
    HideTitlebar,
    ForceDarkMode,
    QuitOnLastWindow,
    MenuBarEnabled,
    FullScreenDetails,
    CtrlDragWindow,
    // Image settings
    FilteringEnabled,
    ScalingEnabled,
    ScalingTwoEnabled,
    ScaleFactor,
    ScrollZoom,
    FractionalZoom,
    CursorZoom,
    CropMode,
    PastActualSizeEnabled,
    // Miscellaneous settings
    Language,
    SortMode,
    SortDescending,
    PreloadingMode,
    LoopFoldersEnabled,
    SlideshowReversed,
    SlideshowTimer,
    AfterDelete,
    AskDelete,
    AllowMimeContentDetection,
    SaveRecents,
    SkipHidden,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i32),
    Double(f64),
    Str(String),
}

impl Value {
    pub fn to_bool(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::Double(d) => *d != 0.0,
            Value::Str(s) => !(s.is_empty() || s == "0" || s.eq_ignore_ascii_case("false")),
        }
    }
    pub fn to_int(&self) -> i32 {
        match self {
            Value::Bool(b) => *b as i32,
            Value::Int(i) => *i,
            Value::Double(d) => d.round() as i32,
            Value::Str(s) => s.trim().parse().unwrap_or(0),
        }
    }
    pub fn to_double(&self) -> f64 {
        match self {
            Value::Bool(b) => *b as i32 as f64,
            Value::Int(i) => *i as f64,
            Value::Double(d) => *d,
            Value::Str(s) => s.trim().parse().unwrap_or(0.0),
        }
    }
    pub fn to_str(&self) -> String {
        match self {
            Value::Bool(b) => if *b { "true".into() } else { "false".into() },
            Value::Int(i) => i.to_string(),
            Value::Double(d) => fmt_double(*d),
            Value::Str(s) => s.clone(),
        }
    }
    /// Semantic equality used by the options dialog (Apply / Restore Defaults buttons).
    pub fn same(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Str(_), _) | (_, Value::Str(_)) => self.to_str() == other.to_str(),
            (Value::Double(_), _) | (_, Value::Double(_)) => {
                (self.to_double() - other.to_double()).abs() < 1e-9
            }
            _ => self.to_int() == other.to_int(),
        }
    }
}

/// Format a double the way QVariant::toString does for "nice" values (5 -> "5", 2.5 -> "2.5").
pub fn fmt_double(d: f64) -> String {
    if d.fract() == 0.0 && d.abs() < 1e15 {
        format!("{}", d as i64)
    } else {
        let s = format!("{:.6}", d);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

#[derive(Clone, Copy)]
pub enum Kind {
    Bool,
    Int,
    Double,
    Str,
}

pub struct SettingDef {
    pub setting: Setting,
    pub key: &'static str,
    pub kind: Kind,
}

macro_rules! def {
    ($s:ident, $k:literal, $kind:ident) => {
        SettingDef { setting: Setting::$s, key: $k, kind: Kind::$kind }
    };
}

pub static DEFS: [SettingDef; 33] = [
    def!(BgColorEnabled, "bgcolorenabled", Bool),
    def!(BgColor, "bgcolor", Str),
    def!(TitleBarMode, "titlebarmode", Int),
    def!(WindowResizeMode, "windowresizemode", Int),
    def!(MinWindowResizedPercentage, "minwindowresizedpercentage", Int),
    def!(MaxWindowResizedPercentage, "maxwindowresizedpercentage", Int),
    def!(HideTitlebar, "hidetitlebar", Bool),
    def!(ForceDarkMode, "forcedarkmode", Bool),
    def!(QuitOnLastWindow, "quitonlastwindow", Bool),
    def!(MenuBarEnabled, "menubarenabled", Bool),
    def!(FullScreenDetails, "fullscreendetails", Bool),
    def!(CtrlDragWindow, "ctrldragwindow", Bool),
    def!(FilteringEnabled, "filteringenabled", Bool),
    def!(ScalingEnabled, "scalingenabled", Bool),
    def!(ScalingTwoEnabled, "scalingtwoenabled", Bool),
    def!(ScaleFactor, "scalefactor", Int),
    def!(ScrollZoom, "scrollzoom", Int),
    def!(FractionalZoom, "fractionalzoom", Bool),
    def!(CursorZoom, "cursorzoom", Bool),
    def!(CropMode, "cropmode", Int),
    def!(PastActualSizeEnabled, "pastactualsizeenabled", Bool),
    def!(Language, "language", Str),
    def!(SortMode, "sortmode", Int),
    def!(SortDescending, "sortdescending", Bool),
    def!(PreloadingMode, "preloadingmode", Int),
    def!(LoopFoldersEnabled, "loopfoldersenabled", Bool),
    def!(SlideshowReversed, "slideshowreversed", Bool),
    def!(SlideshowTimer, "slideshowtimer", Double),
    def!(AfterDelete, "afterdelete", Int),
    def!(AskDelete, "askdelete", Bool),
    def!(AllowMimeContentDetection, "allowmimecontentdetection", Bool),
    def!(SaveRecents, "saverecents", Bool),
    def!(SkipHidden, "skiphidden", Bool),
];

pub fn default_value(s: Setting) -> Value {
    use Setting::*;
    use Value::*;
    match s {
        BgColorEnabled => Bool(true),
        BgColor => Str("#212121".into()),
        TitleBarMode => Int(1),
        WindowResizeMode => Int(1),
        MinWindowResizedPercentage => Int(20),
        MaxWindowResizedPercentage => Int(70),
        HideTitlebar => Bool(false),
        ForceDarkMode => Bool(true),
        QuitOnLastWindow => Bool(false),
        MenuBarEnabled => Bool(false),
        FullScreenDetails => Bool(false),
        CtrlDragWindow => Bool(false),
        FilteringEnabled => Bool(true),
        ScalingEnabled => Bool(true),
        ScalingTwoEnabled => Bool(true),
        ScaleFactor => Int(25),
        ScrollZoom => Int(1),
        FractionalZoom => Bool(false),
        CursorZoom => Bool(true),
        CropMode => Int(0),
        PastActualSizeEnabled => Bool(true),
        Language => Str("system".into()),
        SortMode => Int(0),
        SortDescending => Bool(false),
        PreloadingMode => Int(1),
        LoopFoldersEnabled => Bool(true),
        SlideshowReversed => Bool(false),
        SlideshowTimer => Int(5),
        AfterDelete => Int(2),
        AskDelete => Bool(true),
        AllowMimeContentDetection => Bool(false),
        SaveRecents => Bool(true),
        SkipHidden => Bool(true),
    }
}

pub fn def_for_key(key: &str) -> Option<&'static SettingDef> {
    DEFS.iter().find(|d| d.key == key)
}

pub fn parse_value(kind: Kind, s: &str) -> Value {
    match kind {
        Kind::Bool => Value::Bool(Value::Str(s.to_string()).to_bool()),
        Kind::Int => {
            let t = s.trim();
            Value::Int(t.parse::<i32>().unwrap_or_else(|_| t.parse::<f64>().map(|d| d as i32).unwrap_or(0)))
        }
        Kind::Double => Value::Double(s.trim().parse().unwrap_or(0.0)),
        Kind::Str => Value::Str(s.to_string()),
    }
}

// ---------------------------------------------------------------------------------------------
// Config file store (QSettings equivalent)
// ---------------------------------------------------------------------------------------------

pub fn config_dir() -> PathBuf {
    glib::user_config_dir().join("g-view")
}

pub fn config_path() -> PathBuf {
    config_dir().join("g-view.conf")
}

/// Read-modify-write access to the config file. Every write re-reads the file first so that
/// several running instances don't clobber each other (QSettings behaves the same way).
pub struct Store;

/// glib 0.18 has no KeyFile::set_string_list; write the raw value with GKeyFile list escaping.
pub fn kf_set_list(kf: &glib::KeyFile, group: &str, key: &str, list: &[&str]) {
    let mut v = String::new();
    for item in list {
        for (i, c) in item.chars().enumerate() {
            match c {
                '\\' => v.push_str("\\\\"),
                ';' => v.push_str("\\;"),
                '\n' => v.push_str("\\n"),
                '\t' => v.push_str("\\t"),
                '\r' => v.push_str("\\r"),
                ' ' if i == 0 => v.push_str("\\s"),
                c => v.push(c),
            }
        }
        v.push(';');
    }
    kf.set_value(group, key, &v);
}

impl Store {
    pub fn load() -> glib::KeyFile {
        let kf = glib::KeyFile::new();
        let _ = kf.load_from_file(config_path(), glib::KeyFileFlags::KEEP_COMMENTS);
        kf
    }

    fn save(kf: &glib::KeyFile) {
        let _ = std::fs::create_dir_all(config_dir());
        let data = kf.to_data();
        let path = config_path();
        let tmp = path.with_extension("conf.tmp");
        if std::fs::write(&tmp, data.as_bytes()).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }

    pub fn get(group: &str, key: &str) -> Option<String> {
        Self::load().string(group, key).ok().map(|s| s.to_string())
    }

    pub fn set(group: &str, key: &str, value: &str) {
        let kf = Self::load();
        kf.set_string(group, key, value);
        Self::save(&kf);
    }

    pub fn get_list(group: &str, key: &str) -> Option<Vec<String>> {
        let kf = Self::load();
        if !kf.has_key(group, key).unwrap_or(false) {
            return None;
        }
        Some(kf.string_list(group, key).map(|l| l.iter().map(|s| s.to_string()).collect()).unwrap_or_default())
    }

    pub fn set_list(group: &str, key: &str, values: &[String]) {
        let kf = Self::load();
        let refs: Vec<&str> = values.iter().map(|s| s.as_str()).collect();
        kf_set_list(&kf, group, key, &refs);
        Self::save(&kf);
    }

    pub fn set_many(group: &str, values: &[(String, String)]) {
        let kf = Self::load();
        for (k, v) in values {
            kf.set_string(group, k, v);
        }
        Self::save(&kf);
    }

    pub fn write_keyfile(kf: &glib::KeyFile) {
        Self::save(kf);
    }

    pub fn exists() -> bool {
        config_path().exists()
    }
}

// ---------------------------------------------------------------------------------------------
// SettingsManager
// ---------------------------------------------------------------------------------------------

pub struct SettingsManager {
    values: RefCell<Vec<Option<Value>>>,
    mtime: RefCell<Option<std::time::SystemTime>>,
}

impl SettingsManager {
    pub fn new() -> Self {
        let sm = SettingsManager { values: RefCell::new(vec![None; DEFS.len()]), mtime: RefCell::new(None) };
        sm.load_settings();
        sm
    }

    /// Returns true if any value changed (caller then broadcasts settingsUpdated).
    pub fn load_settings(&self) -> bool {
        *self.mtime.borrow_mut() = std::fs::metadata(config_path()).and_then(|m| m.modified()).ok();
        let kf = Store::load();
        let mut changed = false;
        let mut values = self.values.borrow_mut();
        for (i, def) in DEFS.iter().enumerate() {
            let new = match kf.string("options", def.key) {
                Ok(s) => Some(parse_value(def.kind, s.as_str())),
                Err(_) => None,
            };
            // QSettings returns the default when the key is missing
            let new = new.or_else(|| Some(default_value(def.setting)));
            if values[i] != new {
                changed = true;
                values[i] = new;
            }
        }
        changed
    }

    /// Reload only if the file changed on disk since the last load (cheap check on focus-in).
    pub fn reload_if_changed(&self) -> bool {
        let m = std::fs::metadata(config_path()).and_then(|m| m.modified()).ok();
        if m != *self.mtime.borrow() {
            return self.load_settings();
        }
        false
    }

    pub fn get(&self, s: Setting, defaults: bool) -> Value {
        if defaults {
            return default_value(s);
        }
        self.values.borrow()[s as usize].clone().unwrap_or_else(|| default_value(s))
    }

    pub fn get_by_key(&self, key: &str, defaults: bool) -> Value {
        match def_for_key(key) {
            Some(d) => self.get(d.setting, defaults),
            None => Value::Str(String::new()),
        }
    }

    pub fn bool(&self, s: Setting) -> bool {
        self.get(s, false).to_bool()
    }
    pub fn int(&self, s: Setting) -> i32 {
        self.get(s, false).to_int()
    }
    pub fn double(&self, s: Setting) -> f64 {
        self.get(s, false).to_double()
    }
    pub fn string(&self, s: Setting) -> String {
        self.get(s, false).to_str()
    }

}
