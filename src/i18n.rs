//! Translations. Uses qView's own Qt Linguist (.ts) files, embedded in the binary, so every
//! language qView ships is available with the same strings.

use std::cell::RefCell;
use std::collections::HashMap;

macro_rules! ts {
    ($($code:literal),*) => {
        &[$(($code, include_str!(concat!("../i18n/qview_", $code, ".ts")))),*]
    };
}

pub static TS_FILES: &[(&str, &str)] = ts!(
    "bg", "bn", "cs", "de", "es", "et", "fr", "gl", "hr", "hu", "id", "is", "it", "ja", "kmr", "ko",
    "nb_NO", "nl", "pl", "pt_BR", "pt_PT", "ru", "sl", "sv", "tr", "uk", "vi", "zh_Hans", "zh_Hant"
);

pub fn native_name(code: &str) -> &'static str {
    match code {
        "bg" => "български",
        "bn" => "বাংলা",
        "cs" => "čeština",
        "de" => "Deutsch",
        "es" => "español",
        "et" => "eesti",
        "fr" => "français",
        "gl" => "galego",
        "hr" => "hrvatski",
        "hu" => "magyar",
        "id" => "Indonesia",
        "is" => "íslenska",
        "it" => "italiano",
        "ja" => "日本語",
        "kmr" => "kurmancî",
        "ko" => "한국어",
        "nb_NO" => "norsk bokmål",
        "nl" => "Nederlands",
        "pl" => "polski",
        "pt_BR" => "português",
        "pt_PT" => "português europeu",
        "ru" => "русский",
        "sl" => "slovenščina",
        "sv" => "svenska",
        "tr" => "Türkçe",
        "uk" => "українська",
        "vi" => "Tiếng Việt",
        "zh_Hans" => "简体中文",
        "zh_Hant" => "繁體中文",
        _ => "",
    }
}

thread_local! {
    static TABLE: RefCell<Option<HashMap<(String, String), String>>> = const { RefCell::new(None) };
    static BY_SOURCE: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

/// Mirrors SettingsManager::getSystemLanguage
pub fn system_language() -> String {
    let mut entries: Vec<&str> = vec!["en"];
    entries.extend(TS_FILES.iter().map(|(c, _)| *c));
    for lang in glib::language_names() {
        let lang = lang.as_str();
        let lang = lang.split(['.', '@']).next().unwrap_or("").replace('-', "_");
        if lang.is_empty() || lang == "C" || lang == "POSIX" {
            continue;
        }
        let countryless: String = lang.chars().take(2).collect();
        for e in &entries {
            if *e == lang {
                return lang;
            }
            if *e == countryless {
                return countryless;
            }
        }
    }
    "en".to_string()
}

fn unescape_xml(s: &str) -> String {
    quick_xml::escape::unescape(s).map(|c| c.into_owned()).unwrap_or_else(|_| s.to_string())
}

fn parse_ts(text: &str) -> HashMap<(String, String), String> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(text);
    let mut map = HashMap::new();
    let mut context = String::new();
    let mut source = String::new();
    let mut translation = String::new();
    let mut skip = false;
    let mut cur: Option<&'static str> = None;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => match e.name().into_inner() {
                "name" => {
                    cur = Some("name");
                    context.clear();
                }
                "source" => {
                    cur = Some("source");
                    source.clear();
                }
                "translation" => {
                    cur = Some("translation");
                    translation.clear();
                    skip = e.attributes().flatten().any(|a| {
                        a.key.into_inner() == "type" && matches!(&*a.value, "vanished" | "obsolete")
                    });
                }
                _ => cur = None,
            },
            Ok(Event::Text(t)) => {
                let s = unescape_xml(&t.into_inner());
                match cur {
                    Some("name") => context.push_str(&s),
                    Some("source") => source.push_str(&s),
                    Some("translation") => translation.push_str(&s),
                    _ => {}
                }
            }
            Ok(Event::GeneralRef(r)) => {
                let name = r.into_inner().to_string();
                let s = match name.as_str() {
                    "amp" => "&".to_string(),
                    "lt" => "<".to_string(),
                    "gt" => ">".to_string(),
                    "quot" => "\"".to_string(),
                    "apos" => "'".to_string(),
                    n if n.starts_with("#x") => u32::from_str_radix(&n[2..], 16)
                        .ok()
                        .and_then(char::from_u32)
                        .map(|c| c.to_string())
                        .unwrap_or_default(),
                    n if n.starts_with('#') => {
                        n[1..].parse().ok().and_then(char::from_u32).map(|c| c.to_string()).unwrap_or_default()
                    }
                    _ => String::new(),
                };
                match cur {
                    Some("name") => context.push_str(&s),
                    Some("source") => source.push_str(&s),
                    Some("translation") => translation.push_str(&s),
                    _ => {}
                }
            }
            Ok(Event::End(e)) => {
                if e.name().into_inner() == "translation" && !skip && !translation.is_empty() {
                    map.insert((context.clone(), source.clone()), translation.clone());
                }
                if matches!(e.name().into_inner(), "name" | "source" | "translation") {
                    cur = None;
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    map
}

/// Load the translation for the given language setting ("system", "en" or a code).
pub fn load(lang_setting: &str) -> bool {
    let lang = if lang_setting == "system" { system_language() } else { lang_setting.to_string() };
    if lang == "en" {
        return true;
    }
    let Some((_, text)) = TS_FILES.iter().find(|(c, _)| *c == lang) else { return false };
    let map = parse_ts(text);
    BY_SOURCE.with(|b| {
        let mut b = b.borrow_mut();
        for ((_, src), tr) in &map {
            b.entry(src.clone()).or_insert_with(|| tr.clone());
        }
    });
    TABLE.with(|t| *t.borrow_mut() = Some(map));
    eprintln!("Loaded translation {}", lang);
    true
}

/// Qt-style tr(context, source)
pub fn tr(ctx: &str, src: &str) -> String {
    let s = tr_raw(ctx, src);
    // Our app is g-view; keep "qView" only where it really means upstream qView releases.
    if ctx == "UpdateChecker" {
        s
    } else {
        s.replace("qView", "g-view")
    }
}

fn tr_raw(ctx: &str, src: &str) -> String {
    let hit = TABLE.with(|t| {
        t.borrow().as_ref().and_then(|m| m.get(&(ctx.to_string(), src.to_string())).cloned())
    });
    if let Some(h) = hit {
        return h;
    }
    BY_SOURCE.with(|b| b.borrow().get(src).cloned()).unwrap_or_else(|| src.to_string())
}

/// Convert a Qt mnemonic label ("&Open...") into a GTK one ("_Open...").
pub fn mnemonic(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '&' => {
                if chars.peek() == Some(&'&') {
                    chars.next();
                    out.push('&');
                } else {
                    out.push('_');
                }
            }
            '_' => out.push_str("__"),
            c => out.push(c),
        }
    }
    out
}

/// Strip a Qt mnemonic ("&Open..." -> "Open...")
pub fn plain(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            if chars.peek() == Some(&'&') {
                chars.next();
                out.push('&');
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[macro_export]
macro_rules! tr {
    ($ctx:literal, $s:literal) => {
        $crate::i18n::tr($ctx, $s)
    };
}
