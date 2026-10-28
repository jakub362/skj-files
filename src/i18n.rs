//! Translations. English is the source language: every user-visible string in the
//! Rust code is written in English and looked up in a per-language table at runtime.
//! The Slint UI uses `@tr()` with the bundled `lang/<code>/LC_MESSAGES/skj-files.po`.
//!
//! - `ts("Copy")`              → translated `&'static str`
//! - `tf!("Moved {} items", n)` → translated `format!` (only plain `{}` placeholders)
//! - `t(&dynamic)`              → lookup for strings that may or may not be keys
//!
//! Translated templates may use `{0}`, `{1}` … to reorder or drop arguments, and
//! `{0|one|few|many}` to pick a word form from the number in argument 0
//! (Polish has three plural forms: 1 plik, 2–4 pliki, 5+ plików).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

const EN: u8 = 0;
const PL: u8 = 1;

static LANG: AtomicU8 = AtomicU8::new(EN);

/// Turn the saved setting ("auto", "en", "pl") into a concrete language code.
pub fn resolve(setting: &str) -> &'static str {
    match setting {
        "en" => "en",
        "pl" => "pl",
        _ => {
            let sys = sys_locale::get_locale().unwrap_or_default().to_lowercase();
            if sys.starts_with("pl") { "pl" } else { "en" }
        }
    }
}

/// Switch the Rust side to `code` ("en" / "pl").
pub fn set(code: &str) {
    LANG.store(if code == "pl" { PL } else { EN }, Ordering::Relaxed);
}


pub fn pl() -> bool {
    LANG.load(Ordering::Relaxed) == PL
}

fn table() -> &'static HashMap<&'static str, &'static str> {
    static T: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    T.get_or_init(|| crate::i18n_pl::PL.iter().copied().collect())
}

/// Translate a fixed string.
pub fn ts(s: &'static str) -> &'static str {
    if !pl() {
        return s;
    }
    table().get(s).copied().unwrap_or(s)
}

/// Translate a string that may not be a key (menu labels, toasts…); unknown text is returned as is.
pub fn t(s: &str) -> String {
    if !pl() {
        return s.to_string();
    }
    table().get(s).map(|x| x.to_string()).unwrap_or_else(|| s.to_string())
}

/// Polish plural form index for n: 0 = one, 1 = few (2–4, 22–24…), 2 = many.
pub fn pl_form(n: u64) -> usize {
    if n == 1 {
        0
    } else if (2..=4).contains(&(n % 10)) && !(12..=14).contains(&(n % 100)) {
        1
    } else {
        2
    }
}

fn number_in(s: &str) -> u64 {
    let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    digits.parse().unwrap_or(0)
}

/// Fill a template: `{}` takes the next argument, `{N}` argument N, `{N|a|b|c}` a plural
/// form picked by argument N, `{{` / `}}` are literal braces.
pub fn fill(tmpl: &str, args: &[String]) -> String {
    let mut out = String::with_capacity(tmpl.len() + 16);
    let mut next = 0usize;
    let mut it = tmpl.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        match c {
            '{' if matches!(it.peek(), Some((_, '{'))) => {
                it.next();
                out.push('{');
            }
            '}' if matches!(it.peek(), Some((_, '}'))) => {
                it.next();
                out.push('}');
            }
            '{' => {
                let rest = &tmpl[i + 1..];
                let Some(end) = rest.find('}') else {
                    out.push('{');
                    continue;
                };
                let inner = &rest[..end];
                for _ in 0..inner.chars().count() + 1 {
                    it.next();
                }
                if inner.is_empty() {
                    out.push_str(args.get(next).map(|s| s.as_str()).unwrap_or(""));
                    next += 1;
                } else if let Some((idx, forms)) = inner.split_once('|') {
                    let n = idx.trim().parse::<usize>().ok().and_then(|k| args.get(k)).map(|a| number_in(a)).unwrap_or(0);
                    let forms: Vec<&str> = forms.split('|').collect();
                    let f = if pl() || forms.len() == 3 {
                        forms.get(pl_form(n)).or(forms.last()).copied().unwrap_or("")
                    } else {
                        forms.get(if n == 1 { 0 } else { 1 }).copied().unwrap_or("")
                    };
                    out.push_str(f);
                } else if let Ok(k) = inner.trim().parse::<usize>() {
                    out.push_str(args.get(k).map(|s| s.as_str()).unwrap_or(""));
                } else {
                    out.push('{');
                    out.push_str(inner);
                    out.push('}');
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// Translated `format!`: `tf!("Moved {} items to {}", n, dest)`.
#[macro_export]
macro_rules! tf {
    ($t:literal $(, $a:expr)* $(,)?) => {
        $crate::i18n::fill($crate::i18n::ts($t), &[$(format!("{}", $a)),*])
    };
}

/// "3 files" / "3 pliki" for the few nouns the app counts.
pub fn count_word(n: usize, word: &str) -> String {
    let num = crate::fsx::fmt_count(n);
    if pl() {
        let forms = match word {
            "folder" => ["folder", "foldery", "folderów"],
            "file" => ["plik", "pliki", "plików"],
            "item" => ["element", "elementy", "elementów"],
            _ => [word, word, word],
        };
        return format!("{} {}", num, forms[pl_form(n as u64)]);
    }
    format!("{} {}{}", num, word, if n == 1 { "" } else { "s" })
}

/// Shortcut text for display ("Space" → "Spacja"); stored shortcuts stay in English.
pub fn key_label(combo: &str) -> String {
    if !pl() || combo.is_empty() {
        return combo.to_string();
    }
    combo.split('+').map(|k| ts_dyn(k)).collect::<Vec<_>>().join("+")
}

fn ts_dyn(k: &str) -> String {
    match k {
        "Space" => "Spacja".into(),
        "Middle-click" => "Środkowy przycisk".into(),
        "Left" => "Lewo".into(),
        "Right" => "Prawo".into(),
        "Up" => "Góra".into(),
        "Down" => "Dół".into(),
        _ => k.to_string(),
    }
}

/// The well-known folders keep their English names on disk on Windows/macOS; show them in Polish.
pub fn place_label(label: &str) -> String {
    if !pl() {
        return label.to_string();
    }
    match label {
        "Desktop" => "Pulpit",
        "Downloads" => "Pobrane",
        "Documents" => "Dokumenty",
        "Pictures" => "Obrazy",
        "Music" => "Muzyka",
        "Videos" | "Movies" => "Wideo",
        _ => label,
    }
    .to_string()
}

/// Polish weekday name (chrono's `%A` is English only).
pub fn weekday(d: chrono::Weekday) -> &'static str {
    use chrono::Weekday::*;
    if !pl() {
        return match d {
            Mon => "Monday",
            Tue => "Tuesday",
            Wed => "Wednesday",
            Thu => "Thursday",
            Fri => "Friday",
            Sat => "Saturday",
            Sun => "Sunday",
        };
    }
    match d {
        Mon => "Poniedziałek",
        Tue => "Wtorek",
        Wed => "Środa",
        Thu => "Czwartek",
        Fri => "Piątek",
        Sat => "Sobota",
        Sun => "Niedziela",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_works() {
        set("en");
        assert_eq!(fill("Moved {} to {}", &["3".into(), "x".into()]), "Moved 3 to x");
        assert_eq!(fill("{1} {0}", &["a".into(), "b".into()]), "b a");
        assert_eq!(fill("add {{n}}", &[]), "add {n}");
        set("pl");
        let f = |n: &str| fill("{0} {0|plik|pliki|plików}", &[n.into()]);
        assert_eq!(f("1"), "1 plik");
        assert_eq!(f("3"), "3 pliki");
        assert_eq!(f("5"), "5 plików");
        assert_eq!(f("12"), "12 plików");
        assert_eq!(f("22"), "22 pliki");
        assert_eq!(f("1\u{a0}002"), "1\u{a0}002 pliki");
        set("en");
    }
}
