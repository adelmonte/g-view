//! Port of ShortcutManager, plus parsing/matching of Qt key sequence strings
//! (e.g. "Ctrl+Shift+O", "Ctrl++") against GDK key events.

use crate::i18n::tr;
use crate::settings::Store;

#[derive(Clone, Debug)]
pub struct SShortcut {
    pub readable_name: String,
    pub name: &'static str,
    pub default_shortcuts: Vec<String>,
    pub shortcuts: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyCombo {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
    pub key: String,
}

impl KeyCombo {
    pub fn parse(s: &str) -> Option<KeyCombo> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        let mut kc = KeyCombo { ctrl: false, shift: false, alt: false, meta: false, key: String::new() };
        let mut rest = s;
        loop {
            let lower = rest.to_ascii_lowercase();
            let take = |p: &str| lower.starts_with(p) && rest.len() > p.len();
            if take("ctrl+") {
                kc.ctrl = true;
                rest = &rest[5..];
            } else if take("shift+") {
                kc.shift = true;
                rest = &rest[6..];
            } else if take("alt+") {
                kc.alt = true;
                rest = &rest[4..];
            } else if take("meta+") {
                kc.meta = true;
                rest = &rest[5..];
            } else if take("num+") {
                rest = &rest[4..];
            } else {
                break;
            }
        }
        kc.key = normalize_key_name(rest)?;
        Some(kc)
    }

    pub fn to_qt_string(&self) -> String {
        let mut s = String::new();
        if self.ctrl {
            s.push_str("Ctrl+");
        }
        if self.alt {
            s.push_str("Alt+");
        }
        if self.shift {
            s.push_str("Shift+");
        }
        if self.meta {
            s.push_str("Meta+");
        }
        s.push_str(&self.key);
        s
    }
}

fn normalize_key_name(name: &str) -> Option<String> {
    if name.chars().count() == 1 {
        return Some(name.to_uppercase());
    }
    let lower = name.to_ascii_lowercase();
    let n = match lower.as_str() {
        "esc" | "escape" => "Esc",
        "del" | "delete" => "Del",
        "ins" | "insert" => "Ins",
        "return" => "Return",
        "enter" => "Enter",
        "backspace" => "Backspace",
        "tab" => "Tab",
        "backtab" => "Backtab",
        "space" => "Space",
        "home" => "Home",
        "end" => "End",
        "left" => "Left",
        "right" => "Right",
        "up" => "Up",
        "down" => "Down",
        "pgup" | "pageup" => "PgUp",
        "pgdown" | "pagedown" => "PgDown",
        "pause" => "Pause",
        "print" => "Print",
        "menu" => "Menu",
        _ => {
            if lower.starts_with('f') && lower[1..].parse::<u32>().is_ok() {
                return Some(format!("F{}", &lower[1..]));
            }
            return Some(name.to_string());
        }
    };
    Some(n.to_string())
}

/// Qt key name for a GDK keyval
pub fn key_name_for_keyval(kv: gdk::keys::Key) -> Option<String> {
    use gdk::keys::constants as k;
    let name = match kv {
        k::Escape => "Esc",
        k::Delete | k::KP_Delete => "Del",
        k::Insert | k::KP_Insert => "Ins",
        k::Return => "Return",
        k::KP_Enter | k::ISO_Enter => "Enter",
        k::BackSpace => "Backspace",
        k::Tab | k::KP_Tab => "Tab",
        k::ISO_Left_Tab => "Backtab",
        k::space | k::KP_Space => "Space",
        k::Home | k::KP_Home => "Home",
        k::End | k::KP_End => "End",
        k::Left | k::KP_Left => "Left",
        k::Right | k::KP_Right => "Right",
        k::Up | k::KP_Up => "Up",
        k::Down | k::KP_Down => "Down",
        k::Page_Up | k::KP_Page_Up => "PgUp",
        k::Page_Down | k::KP_Page_Down => "PgDown",
        k::Pause => "Pause",
        k::Print => "Print",
        k::Menu => "Menu",
        k::KP_Add => "+",
        k::KP_Subtract => "-",
        k::KP_Multiply => "*",
        k::KP_Divide => "/",
        k::KP_Decimal => ".",
        k::KP_Equal => "=",
        _ => {
            let raw = *kv;
            if (*k::F1..=*k::F35).contains(&raw) {
                return Some(format!("F{}", raw - *k::F1 + 1));
            }
            if (*k::KP_0..=*k::KP_9).contains(&raw) {
                return Some(((b'0' + (raw - *k::KP_0) as u8) as char).to_string());
            }
            let c = kv.to_unicode()?;
            if c.is_control() {
                return None;
            }
            return Some(c.to_uppercase().collect());
        }
    };
    Some(name.to_string())
}

fn is_modifier_key(kv: &gdk::keys::Key) -> bool {
    use gdk::keys::constants as k;
    matches!(
        *kv,
        k::Shift_L
            | k::Shift_R
            | k::Control_L
            | k::Control_R
            | k::Alt_L
            | k::Alt_R
            | k::Meta_L
            | k::Meta_R
            | k::Super_L
            | k::Super_R
            | k::Hyper_L
            | k::Hyper_R
            | k::ISO_Level3_Shift
            | k::ISO_Level5_Shift
            | k::Caps_Lock
            | k::Num_Lock
    )
}

/// All combos an event could match (mirrors Qt's QKeyMapper::possibleKeys closely enough).
pub fn combos_for_event(ev: &gdk::EventKey) -> Vec<KeyCombo> {
    let kv = ev.keyval();
    if is_modifier_key(&kv) {
        return vec![];
    }
    let st = ev.state();
    let ctrl = st.contains(gdk::ModifierType::CONTROL_MASK);
    let shift = st.contains(gdk::ModifierType::SHIFT_MASK);
    let alt = st.contains(gdk::ModifierType::MOD1_MASK);
    let meta = st.contains(gdk::ModifierType::SUPER_MASK) || st.contains(gdk::ModifierType::META_MASK);
    let mut out = vec![];
    let mut push = |key: String, shift: bool| {
        let c = KeyCombo { ctrl, shift, alt, meta, key };
        if !out.contains(&c) {
            out.push(c);
        }
    };
    let mut names = vec![];
    if let Some(n) = key_name_for_keyval(kv) {
        names.push(n);
    }
    // Fall back to the latin / unshifted key on the same physical key (non-latin layouts, Shift+=)
    if let Some(display) = gdk::Display::default() {
        if let Some(keymap) = gdk::Keymap::for_display(&display) {
            for (key, kv0) in keymap.entries_for_keycode(ev.hardware_keycode() as u32) {
                if key.level() == 0 {
                    if let Some(n) = key_name_for_keyval(gdk::keys::Key::from(kv0)) {
                        if n.is_ascii() && !names.contains(&n) {
                            names.push(n);
                        }
                    }
                }
            }
        }
    }
    for n in names {
        let is_letter = n.len() == 1 && n.chars().next().unwrap().is_alphabetic();
        push(n.clone(), shift);
        if shift && !is_letter && n.chars().count() == 1 {
            push(n, false);
        }
    }
    out
}

/// Build a combo from an event for the shortcut recorder.
pub fn record_combo(ev: &gdk::EventKey) -> Option<KeyCombo> {
    let kv = ev.keyval();
    if is_modifier_key(&kv) {
        return None;
    }
    let st = ev.state();
    let mut key = key_name_for_keyval(kv)?;
    let mut shift = st.contains(gdk::ModifierType::SHIFT_MASK);
    // Qt records shifted symbols as e.g. "Ctrl++" rather than "Ctrl+Shift+="
    if shift && key.chars().count() == 1 && !key.chars().next().unwrap().is_alphanumeric() {
        shift = false;
    }
    if key == "Backtab" {
        key = "Backtab".into();
    }
    Some(KeyCombo {
        ctrl: st.contains(gdk::ModifierType::CONTROL_MASK),
        shift,
        alt: st.contains(gdk::ModifierType::MOD1_MASK),
        meta: st.contains(gdk::ModifierType::SUPER_MASK) || st.contains(gdk::ModifierType::META_MASK),
        key,
    })
}

/// (keyval, modifiers) for showing an accelerator label in menus
pub fn accel_for(s: &str) -> Option<(u32, gdk::ModifierType)> {
    let kc = KeyCombo::parse(s)?;
    let mut m = gdk::ModifierType::empty();
    if kc.ctrl {
        m |= gdk::ModifierType::CONTROL_MASK;
    }
    if kc.shift {
        m |= gdk::ModifierType::SHIFT_MASK;
    }
    if kc.alt {
        m |= gdk::ModifierType::MOD1_MASK;
    }
    if kc.meta {
        m |= gdk::ModifierType::SUPER_MASK;
    }
    let name = match kc.key.as_str() {
        "Esc" => "Escape".to_string(),
        "Del" => "Delete".into(),
        "Ins" => "Insert".into(),
        "Backspace" => "BackSpace".into(),
        "PgUp" => "Page_Up".into(),
        "PgDown" => "Page_Down".into(),
        "Enter" => "KP_Enter".into(),
        "Space" => "space".into(),
        "Backtab" => "ISO_Left_Tab".into(),
        k if k.chars().count() == 1 => {
            let c = k.chars().next().unwrap();
            let kv = *gdk::keys::Key::from_unicode(c.to_lowercase().next().unwrap_or(c));
            return Some((kv, m));
        }
        k => k.to_string(),
    };
    let kv = *gdk::keys::Key::from_name(&name);
    if kv == 0 || kv == 0xffffff {
        return None;
    }
    Some((kv, m))
}

pub struct ShortcutManager {
    pub list: Vec<SShortcut>,
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

impl ShortcutManager {
    pub fn new() -> Self {
        let mut sm = ShortcutManager { list: Self::initial_list() };
        sm.update_shortcuts();
        sm
    }

    fn initial_list() -> Vec<SShortcut> {
        let t = |x: &str| tr("ShortcutManager", x);
        let mk = |readable: String, name: &'static str, defaults: Vec<String>| SShortcut {
            readable_name: readable,
            name,
            default_shortcuts: defaults,
            shortcuts: vec![],
        };
        // Defaults are what Qt's QKeySequence standard bindings resolve to on Linux.
        vec![
            mk(t("Open"), "open", s(&["Ctrl+O"])),
            mk(t("Reload File"), "reloadfile", s(&["F5"])),
            mk(t("Open Containing Folder"), "opencontainingfolder", vec![]),
            mk(t("Show File Info"), "showfileinfo", s(&["I"])),
            mk(t("Restore from Trash"), "undo", s(&["Ctrl+Z"])),
            mk(t("Copy"), "copy", s(&["Ctrl+C", "Ctrl+Ins"])),
            mk(t("Paste"), "paste", s(&["Ctrl+V", "Shift+Ins"])),
            mk(t("Rename"), "rename", s(&["Ctrl+Return", "F2", "Ctrl+R"])),
            mk(t("Move to Trash"), "delete", s(&["Ctrl+Backspace", "Del"])),
            mk(t("Delete Permanently"), "deletepermanent", s(&["Shift+Del"])),
            mk(t("First File"), "firstfile", s(&["Home"])),
            mk(t("Previous File"), "previousfile", s(&["Left"])),
            mk(t("Next File"), "nextfile", s(&["Right"])),
            mk(t("Last File"), "lastfile", s(&["End"])),
            mk(t("Random File"), "randomfile", s(&["R"])),
            mk(t("Zoom In"), "zoomin", s(&["Ctrl++", "Ctrl+="])),
            mk(t("Zoom Out"), "zoomout", s(&["Ctrl+-"])),
            mk(t("Reset Zoom"), "resetzoom", s(&["Ctrl+0"])),
            mk(t("Original Size"), "originalsize", s(&["O"])),
            mk(t("Rotate Right"), "rotateright", s(&["Up"])),
            mk(t("Rotate Left"), "rotateleft", s(&["Down"])),
            mk(t("Mirror"), "mirror", s(&["F"])),
            mk(t("Flip"), "flip", s(&["Ctrl+F"])),
            // qView lost its Full Screen shortcut entry in a refactor (commit d0c945a); restored
            // here with the Linux default F11 since the code right below it still expects it.
            mk(t("Full Screen"), "fullscreen", s(&["F11"])),
            mk(t("Save Frame As"), "saveframeas", s(&["Ctrl+S"])),
            mk(t("Pause"), "pause", s(&["P"])),
            mk(t("Next Frame"), "nextframe", s(&["N"])),
            mk(t("Decrease Speed"), "decreasespeed", s(&["["])),
            mk(t("Reset Speed"), "resetspeed", s(&["\\"])),
            mk(t("Increase Speed"), "increasespeed", s(&["]"])),
            mk(t("Toggle Slideshow"), "slideshow", vec![]),
            mk(t("Settings"), "options", vec![]),
            mk(t("Quit"), "quit", s(&["Ctrl+Q", "Ctrl+W"])),
        ]
    }

    /// Re-read user shortcuts from the config (ShortcutManager::updateShortcuts)
    pub fn update_shortcuts(&mut self) {
        let kf = Store::load();
        for sc in &mut self.list {
            sc.shortcuts = if kf.has_key("shortcuts", sc.name).unwrap_or(false) {
                kf.string_list("shortcuts", sc.name)
                    .map(|l| l.iter().map(|s| s.to_string()).filter(|s| !s.is_empty()).collect())
                    .unwrap_or_default()
            } else {
                sc.default_shortcuts.clone()
            };
        }
    }

    pub fn shortcuts_for(&self, name: &str) -> Vec<String> {
        self.list.iter().find(|s| s.name == name).map(|s| s.shortcuts.clone()).unwrap_or_default()
    }

    /// Find the action bound to a key event
    pub fn action_for_event(&self, ev: &gdk::EventKey) -> Option<&'static str> {
        let combos = combos_for_event(ev);
        if combos.is_empty() {
            return None;
        }
        for sc in &self.list {
            for s in &sc.shortcuts {
                if let Some(kc) = KeyCombo::parse(s) {
                    if combos.contains(&kc) {
                        return Some(sc.name);
                    }
                }
            }
        }
        None
    }

    pub fn escape_is_bound(&self) -> bool {
        self.list.iter().any(|s| s.shortcuts.iter().any(|k| KeyCombo::parse(k).map(|c| c.key == "Esc" && !c.ctrl && !c.shift && !c.alt && !c.meta).unwrap_or(false)))
    }
}

/// "Ctrl+O, F5" display text (QKeySequence NativeText on Linux is the portable text)
pub fn readable(list: &[String]) -> String {
    list.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_plus() {
        let k = KeyCombo::parse("Ctrl++").unwrap();
        assert!(k.ctrl && k.key == "+");
        let k = KeyCombo::parse("Ctrl+Shift+O").unwrap();
        assert!(k.ctrl && k.shift && k.key == "O");
        let k = KeyCombo::parse("\\").unwrap();
        assert_eq!(k.key, "\\");
        let k = KeyCombo::parse("Shift+Del").unwrap();
        assert!(k.shift && k.key == "Del");
        assert_eq!(KeyCombo::parse("f11").unwrap().key, "F11");
    }
}
