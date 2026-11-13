//! Detects an existing qView (Qt) configuration and migrates it into our config file.
//!
//! qView stores its settings with QSettings in INI format at `~/.config/qView/qView.conf`
//! (or inside the Flatpak / Snap sandbox). Values use QSettings' own escaping, and some values
//! (window geometry, recent files) are binary QDataStream blobs wrapped in `@ByteArray(...)` /
//! `@Variant(...)`. This module parses all of that.

use crate::settings::{self, Store};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub enum QVal {
    Str(String),
    List(Vec<QVal>),
    Bytes(Vec<u8>),
    Variant(Vec<u8>),
    Invalid,
}

impl QVal {
    pub fn as_string(&self) -> Option<String> {
        match self {
            QVal::Str(s) => Some(s.clone()),
            QVal::List(l) if l.len() == 1 => l[0].as_string(),
            _ => None,
        }
    }
    pub fn as_string_list(&self) -> Vec<String> {
        match self {
            QVal::Str(s) => vec![s.clone()],
            QVal::List(l) => l.iter().filter_map(|v| v.as_string()).collect(),
            QVal::Variant(b) => decode_variant_string_list(b).unwrap_or_default(),
            _ => vec![],
        }
    }
}

/// group -> key -> value
pub type QIni = HashMap<String, HashMap<String, QVal>>;

fn decode_percent(name: &str) -> String {
    // QSettings encodes special chars in keys as %XX or %UXXXX
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '%' {
            if i + 1 < chars.len() && chars[i + 1] == 'U' && i + 5 < chars.len() {
                let hex: String = chars[i + 2..i + 6].iter().collect();
                if let Some(c) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    out.push(c);
                    i += 6;
                    continue;
                }
            } else if i + 2 < chars.len() {
                let hex: String = chars[i + 1..i + 3].iter().collect();
                if let Some(c) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    out.push(c);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Port of QSettingsPrivate::iniUnescapedStringList. Returns (items, is_list).
fn unescape_list(s: &str) -> (Vec<String>, bool) {
    let chars: Vec<char> = s.chars().collect();
    let mut items: Vec<String> = vec![];
    let mut cur = String::new();
    let mut is_list = false;
    let mut in_quotes = false;
    let mut i = 0;
    // skip leading whitespace
    while i < chars.len() && (chars[i] == ' ' || chars[i] == '\t') {
        i += 1;
    }
    let mut pending_ws = String::new();
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() {
            cur.push_str(&pending_ws);
            pending_ws.clear();
            let e = chars[i + 1];
            i += 2;
            match e {
                'a' => cur.push('\u{7}'),
                'b' => cur.push('\u{8}'),
                'f' => cur.push('\u{c}'),
                'n' => cur.push('\n'),
                'r' => cur.push('\r'),
                't' => cur.push('\t'),
                'v' => cur.push('\u{b}'),
                'x' => {
                    let mut v: u32 = 0;
                    while i < chars.len() && chars[i].is_ascii_hexdigit() {
                        v = v.wrapping_mul(16).wrapping_add(chars[i].to_digit(16).unwrap());
                        i += 1;
                    }
                    cur.push(char::from_u32(v).unwrap_or('\u{fffd}'));
                }
                '0'..='7' => {
                    let mut v: u32 = e.to_digit(8).unwrap();
                    while i < chars.len() && ('0'..='7').contains(&chars[i]) {
                        v = v * 8 + chars[i].to_digit(8).unwrap();
                        i += 1;
                    }
                    cur.push(char::from_u32(v).unwrap_or('\u{fffd}'));
                }
                other => cur.push(other),
            }
            continue;
        }
        if c == '"' {
            cur.push_str(&pending_ws);
            pending_ws.clear();
            in_quotes = !in_quotes;
            i += 1;
            continue;
        }
        if !in_quotes && c == ',' {
            is_list = true;
            items.push(std::mem::take(&mut cur));
            pending_ws.clear();
            i += 1;
            while i < chars.len() && (chars[i] == ' ' || chars[i] == '\t') {
                i += 1;
            }
            continue;
        }
        if !in_quotes && (c == ' ' || c == '\t') {
            pending_ws.push(c);
            i += 1;
            continue;
        }
        cur.push_str(&pending_ws);
        pending_ws.clear();
        cur.push(c);
        i += 1;
    }
    items.push(cur);
    (items, is_list)
}

fn string_to_qval(s: &str) -> QVal {
    let latin1 = |inner: &str| -> Vec<u8> { inner.chars().map(|c| (c as u32 & 0xff) as u8).collect() };
    if let Some(rest) = s.strip_prefix("@ByteArray(").and_then(|r| r.strip_suffix(')')) {
        return QVal::Bytes(latin1(rest));
    }
    if let Some(rest) = s.strip_prefix("@Variant(").and_then(|r| r.strip_suffix(')')) {
        return QVal::Variant(latin1(rest));
    }
    if s == "@Invalid()" || s == "@EmptyList()" {
        return QVal::Invalid;
    }
    if let Some(rest) = s.strip_prefix("@@") {
        return QVal::Str(format!("@{}", rest));
    }
    QVal::Str(s.to_string())
}

pub fn parse_ini(text: &str) -> QIni {
    let mut ini: QIni = HashMap::new();
    let mut group = "General".to_string();
    for raw in text.lines() {
        let line = raw.trim_start();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            if let Some(end) = line.find(']') {
                group = decode_percent(&line[1..end]);
                continue;
            }
        }
        let Some(eq) = line.find('=') else { continue };
        let key = decode_percent(line[..eq].trim());
        let (items, is_list) = unescape_list(&line[eq + 1..]);
        let val = if is_list {
            QVal::List(items.iter().map(|s| string_to_qval(s)).collect())
        } else {
            string_to_qval(&items[0])
        };
        // Keys written while inside a group can also contain the group as "group\key"
        let (g, k) = match key.rfind(['\\', '/']) {
            Some(p) if group == "General" => (key[..p].to_string(), key[p + 1..].to_string()),
            _ => (group.clone(), key),
        };
        ini.entry(g).or_default().insert(k, val);
    }
    ini
}

// --- QDataStream helpers -------------------------------------------------------------------

struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}
impl<'a> Reader<'a> {
    fn u8(&mut self) -> Option<u8> {
        let v = *self.b.get(self.p)?;
        self.p += 1;
        Some(v)
    }
    fn u16(&mut self) -> Option<u16> {
        let s = self.b.get(self.p..self.p + 2)?;
        self.p += 2;
        Some(u16::from_be_bytes([s[0], s[1]]))
    }
    fn u32(&mut self) -> Option<u32> {
        let s = self.b.get(self.p..self.p + 4)?;
        self.p += 4;
        Some(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn i32(&mut self) -> Option<i32> {
        self.u32().map(|v| v as i32)
    }
    fn i64(&mut self) -> Option<i64> {
        let s = self.b.get(self.p..self.p + 8)?;
        self.p += 8;
        Some(i64::from_be_bytes(s.try_into().ok()?))
    }
    fn size(&mut self) -> Option<usize> {
        let v = self.u32()?;
        if v == 0xFFFF_FFFE {
            return self.i64().map(|v| v as usize);
        }
        Some(v as usize)
    }
    fn qstring(&mut self) -> Option<String> {
        let len = self.u32()?;
        if len == 0xFFFF_FFFF {
            return Some(String::new());
        }
        let len = if len == 0xFFFF_FFFE { self.i64()? as usize } else { len as usize };
        let s = self.b.get(self.p..self.p + len)?;
        self.p += len;
        let units: Vec<u16> = s.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        Some(String::from_utf16_lossy(&units))
    }
    fn remaining(&self) -> usize {
        self.b.len().saturating_sub(self.p)
    }
}

fn decode_variant_string_list(b: &[u8]) -> Option<Vec<String>> {
    // Qt 6 streams QVariant as (typeId, value); Qt 5 adds a null-flag byte after typeId.
    decode_variant_with(b, false).or_else(|| decode_variant_with(b, true))
}

fn decode_variant_with(b: &[u8], null_flag: bool) -> Option<Vec<String>> {
    let mut r = Reader { b, p: 0 };
    let type_id = r.u32()?;
    if null_flag {
        r.u8()?;
    }
    let out = match type_id {
        11 => {
            // QStringList
            let n = r.size()?;
            if n > 100_000 {
                return None;
            }
            let mut out = Vec::with_capacity(n);
            for _ in 0..n {
                out.push(r.qstring()?);
            }
            out
        }
        10 => vec![r.qstring()?], // QString
        _ => return None,
    };
    // the whole blob must be consumed, otherwise we guessed the layout wrong
    if r.remaining() != 0 {
        return None;
    }
    Some(out)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geometry {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub maximized: bool,
}

/// Parse the blob written by QWidget::saveGeometry.
fn decode_qt_geometry(b: &[u8]) -> Option<Geometry> {
    let mut r = Reader { b, p: 0 };
    if r.u32()? != 0x1D9D0CB {
        return None;
    }
    let _major = r.u16()?;
    let _minor = r.u16()?;
    let rect = |r: &mut Reader| -> Option<(i32, i32, i32, i32)> {
        let x1 = r.i32()?;
        let y1 = r.i32()?;
        let x2 = r.i32()?;
        let y2 = r.i32()?;
        Some((x1, y1, x2 - x1 + 1, y2 - y1 + 1))
    };
    let _frame = rect(&mut r)?;
    let normal = rect(&mut r)?;
    let _screen = r.i32()?;
    let maximized = r.u8()? != 0;
    let _fullscreen = r.u8()?;
    if r.remaining() >= 4 {
        let _screen_width = r.i32();
    }
    // Prefer the client geometry (format >= 2) when present
    let geo = if r.remaining() >= 16 { rect(&mut r).unwrap_or(normal) } else { normal };
    let geo = if maximized { normal } else { geo };
    if geo.2 <= 0 || geo.3 <= 0 {
        return None;
    }
    Some(Geometry { x: geo.0, y: geo.1, w: geo.2, h: geo.3, maximized })
}

// --- Migration ----------------------------------------------------------------------------

/// All places a qView config may live, most likely first.
pub fn candidate_paths() -> Vec<PathBuf> {
    let home = glib::home_dir();
    let mut v = vec![glib::user_config_dir().join("qView").join("qView.conf")];
    v.push(home.join(".config/qView/qView.conf"));
    v.push(home.join(".var/app/com.interversehq.qView/config/qView/qView.conf"));
    v.push(home.join("snap/qview/current/.config/qView/qView.conf"));
    v.dedup();
    v
}

pub fn find_qview_config() -> Option<PathBuf> {
    candidate_paths()
        .into_iter()
        .filter(|p| p.is_file())
        .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
}

/// Migrate the given qView config into a fresh key file. Returns a short summary.
pub fn migrate_from(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let ini = parse_ini(&text);
    let kf = glib::KeyFile::new();
    let mut n_opts = 0;
    let mut n_short = 0;
    let mut n_recent = 0;

    if let Some(opts) = ini.get("options") {
        for def in settings::DEFS.iter() {
            if let Some(s) = opts.get(def.key).and_then(|v| v.as_string()) {
                let v = settings::parse_value(def.kind, &s);
                kf.set_string("options", def.key, &v.to_str());
                n_opts += 1;
            }
        }
    }
    if let Some(sc) = ini.get("shortcuts") {
        for (name, val) in sc {
            let list: Vec<String> = match val {
                QVal::Invalid => vec![],
                QVal::Str(s) if s.is_empty() => vec![],
                v => v.as_string_list(),
            };
            let refs: Vec<&str> = list.iter().map(|s| s.as_str()).collect();
            crate::settings::kf_set_list(&kf, "shortcuts", name, &refs);
            n_short += 1;
        }
    }
    if let Some(rec) = ini.get("recents") {
        if let Some(dir) = rec.get("lastFileDialogDir").and_then(|v| v.as_string()) {
            kf.set_string("recents", "lastFileDialogDir", &dir);
        }
        if let Some(val) = rec.get("recentFiles") {
            let mut paths: Vec<String> = vec![];
            let elems: Vec<&QVal> = match val {
                QVal::List(l) => l.iter().collect(),
                other => vec![other],
            };
            for e in elems {
                if let QVal::Variant(b) = e {
                    if let Some(l) = decode_variant_string_list(b) {
                        if let Some(p) = l.get(1) {
                            paths.push(p.clone());
                        }
                    }
                }
            }
            n_recent = paths.len();
            let refs: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
            crate::settings::kf_set_list(&kf, "recents", "recentFiles", &refs);
        }
    }
    if let Some(gen) = ini.get("General") {
        for key in ["firstlaunch", "configversion", "optionstab", "lastupdatecheck"] {
            if let Some(s) = gen.get(key).and_then(|v| v.as_string()) {
                kf.set_string("General", key, &s);
            }
        }
        if let Some(QVal::Bytes(b)) = gen.get("geometry") {
            if let Some(g) = decode_qt_geometry(b) {
                kf.set_string("General", "geometry", &encode_geometry(&g));
            }
        }
        if let Some(QVal::Bytes(b)) = gen.get("optionsgeometry") {
            if let Some(g) = decode_qt_geometry(b) {
                kf.set_string("General", "optionsgeometry", &encode_geometry(&g));
            }
        }
    }
    kf.set_string("General", "migratedfrom", &path.to_string_lossy());
    Store::write_keyfile(&kf);
    Some(format!(
        "Migrated qView settings from {} ({} options, {} shortcuts, {} recent files)",
        path.display(),
        n_opts,
        n_short,
        n_recent
    ))
}

pub fn encode_geometry(g: &Geometry) -> String {
    format!("{},{},{},{},{}", g.x, g.y, g.w, g.h, g.maximized as i32)
}

pub fn decode_geometry(s: &str) -> Option<Geometry> {
    let p: Vec<i32> = s.split(',').filter_map(|t| t.trim().parse().ok()).collect();
    if p.len() < 4 || p[2] <= 0 || p[3] <= 0 {
        return None;
    }
    Some(Geometry { x: p[0], y: p[1], w: p[2], h: p[3], maximized: p.get(4).copied().unwrap_or(0) != 0 })
}

/// Called at startup: if we have no config yet but qView does, migrate it.
pub fn migrate_if_needed() {
    if Store::exists() {
        return;
    }
    if let Some(p) = find_qview_config() {
        if let Some(msg) = migrate_from(&p) {
            eprintln!("{}", msg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lists_and_escapes() {
        let ini = parse_ini(
            "[shortcuts]\nzoomin=Ctrl++, Ctrl+=\nrename=Ctrl+Return, F2, Ctrl+R\nslideshow=@Invalid()\ncomma=\"Ctrl+,\"\n[options]\nbgcolor=#ff0000\n",
        );
        let sc = &ini["shortcuts"];
        assert_eq!(sc["zoomin"].as_string_list(), vec!["Ctrl++", "Ctrl+="]);
        assert_eq!(sc["rename"].as_string_list(), vec!["Ctrl+Return", "F2", "Ctrl+R"]);
        assert!(matches!(sc["slideshow"], QVal::Invalid));
        assert_eq!(sc["comma"].as_string_list(), vec!["Ctrl+,"]);
        assert_eq!(ini["options"]["bgcolor"].as_string().unwrap(), "#ff0000");
    }

    #[test]
    fn decodes_recent_variant() {
        // Build a QStringList variant: type 11, null flag 0, count 2, two QStrings
        let mut b = vec![];
        b.extend(11u32.to_be_bytes());
        b.push(0);
        b.extend(2u32.to_be_bytes());
        for s in ["a.png", "/tmp/a.png"] {
            let u: Vec<u16> = s.encode_utf16().collect();
            b.extend(((u.len() * 2) as u32).to_be_bytes());
            for c in u {
                b.extend(c.to_be_bytes());
            }
        }
        // Escape like QSettings does
        let mut esc = String::new();
        let mut escape_next_if_digit = false;
        for &c in &b {
            if c < 0x20 || c >= 0x7f || (escape_next_if_digit && (c as char).is_ascii_hexdigit()) {
                esc.push_str(&format!("\\x{:x}", c));
                escape_next_if_digit = true;
            } else {
                esc.push(c as char);
                escape_next_if_digit = false;
            }
        }
        let ini = parse_ini(&format!("[recents]\nrecentFiles=@Variant({}), @Variant({})\n", esc, esc));
        match &ini["recents"]["recentFiles"] {
            QVal::List(l) => {
                assert_eq!(l.len(), 2);
                assert_eq!(l[0].as_string_list(), vec!["a.png", "/tmp/a.png"]);
            }
            other => panic!("{:?}", other),
        }
    }
}

