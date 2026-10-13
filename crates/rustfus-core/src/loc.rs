// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! Rufus' translations (`res/loc/rufus.loc`), parsed like Rufus'
//! `get_loc_data_file()`: `l` starts a language, `t ID "text"` adds a string,
//! quoted continuation lines are collated, and missing strings fall back to
//! en-US (the first language, which every other one is based on).

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

static LOC_FILE: &str = include_str!("../../../res/loc/rufus.loc");

pub struct Lang {
    pub code: String,
    pub name: String,
}

struct Parsed {
    langs: Vec<Lang>,
    strings: Vec<HashMap<String, String>>,
}

static PARSED: OnceLock<Parsed> = OnceLock::new();
static CURRENT: RwLock<usize> = RwLock::new(0);

/// Unescape a quoted string body the way Rufus' parser does: `\n`, `\"` and
/// `\\` are kept, any other escape (`\t`, `\v`) is dropped.
fn unescape(s: &str, out: &mut String) {
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some(c @ ('"' | '\\')) => out.push(c),
            _ => {}
        }
    }
}

/// Body of the first `"..."` in `s` (honouring `\"`), plus the rest.
fn quoted(s: &str) -> Option<(&str, &str)> {
    let s = &s[s.find('"')? + 1..];
    let b = s.as_bytes();
    let end = (0..b.len()).find(|&i| b[i] == b'"' && (i == 0 || b[i - 1] != b'\\'))?;
    Some((&s[..end], &s[end + 1..]))
}

fn parse(data: &str) -> Parsed {
    let mut p = Parsed {
        langs: Vec::new(),
        strings: Vec::new(),
    };
    let mut last: Option<String> = None; // id of the last `t` line, for continuations
    for line in data.lines() {
        let l = line.trim_start();
        if l.starts_with('"') {
            if let (Some(id), Some(map), Some((body, _))) = (&last, p.strings.last_mut(), quoted(l))
            {
                unescape(body, map.get_mut(id).unwrap());
            }
            continue;
        }
        last = None;
        let (cmd, rest) = l.split_once([' ', '\t']).unwrap_or((l, ""));
        match cmd {
            "l" => {
                let Some((code, rest)) = quoted(rest) else {
                    continue;
                };
                let name = quoted(rest).map_or("", |q| q.0);
                p.langs.push(Lang {
                    code: code.into(),
                    name: name.into(),
                });
                p.strings.push(HashMap::new());
            }
            "t" => {
                let rest = rest.trim_start();
                let id = rest.split_whitespace().next().unwrap_or("");
                let (Some(map), Some((body, _))) = (p.strings.last_mut(), quoted(rest)) else {
                    continue;
                };
                let mut s = String::new();
                unescape(body, &mut s);
                map.insert(id.into(), s);
                last = Some(id.into());
            }
            _ => {} // comments, v/b/g/a/f/s lines
        }
    }
    p
}

fn parsed() -> &'static Parsed {
    PARSED.get_or_init(|| parse(LOC_FILE))
}

/// All languages in file order (en-US first).
pub fn languages() -> Vec<Lang> {
    parsed()
        .langs
        .iter()
        .map(|l| Lang {
            code: l.code.clone(),
            name: l.name.clone(),
        })
        .collect()
}

pub fn set_language(code: &str) {
    let idx = parsed()
        .langs
        .iter()
        .position(|l| l.code.eq_ignore_ascii_case(code))
        .unwrap_or(0);
    *CURRENT.write().unwrap() = idx;
}

pub fn current() -> String {
    parsed().langs[*CURRENT.read().unwrap()].code.clone()
}

/// Closest Rufus language for a system locale such as `fr_FR.UTF-8`,
/// `pt-PT` or `zh-Hant-HK`: exact match, else same primary language.
fn closest(sys: &str, codes: &[&str]) -> String {
    let sys = sys
        .trim()
        .split(['.', '@'])
        .next()
        .unwrap_or("")
        .replace('_', "-");
    let lower = sys.to_ascii_lowercase();
    if let Some(c) = codes.iter().find(|c| c.eq_ignore_ascii_case(&sys)) {
        return c.to_string();
    }
    let primary = lower.split('-').next().unwrap_or("");
    let want = match primary {
        "zh" if ["hant", "tw", "hk", "mo"]
            .iter()
            .any(|t| lower.split('-').any(|p| p == *t)) =>
        {
            "zh-TW"
        }
        "no" | "nn" => "nb",
        p => p,
    };
    codes
        .iter()
        .find(|c| {
            c.eq_ignore_ascii_case(want)
                || c.split('-')
                    .next()
                    .is_some_and(|p| p.eq_ignore_ascii_case(want))
        })
        .map_or_else(|| "en-US".into(), |c| c.to_string())
}

/// Rufus language matching the user's locale (LANG/LC_ALL, AppleLocale,
/// Get-Culture; see `wue::locale`), else en-US.
pub fn default_for_system() -> String {
    let codes: Vec<&str> = parsed().langs.iter().map(|l| l.code.as_str()).collect();
    closest(&crate::wue::locale(), &codes)
}

/// Strip Windows mnemonic markers: `&x` -> `x`, `&&` -> `&`.
fn strip_amp(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '&' {
            if it.peek() == Some(&'&') {
                it.next();
                out.push('&');
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Translated text for `id` in the current language, else en-US, else `id`.
pub fn t(id: &str) -> String {
    let p = parsed();
    let cur = *CURRENT.read().unwrap();
    [cur, 0]
        .iter()
        .find_map(|&i| p.strings[i].get(id))
        .map_or_else(|| id.to_string(), |s| strip_amp(s))
}

/// `t(id)` with printf-style `%s`/`%d`/`%02X`/`%0.2f`/`%%` filled from `args` in order.
pub fn tf(id: &str, args: &[&str]) -> String {
    printf(&t(id), args)
}

fn printf(fmt: &str, args: &[&str]) -> String {
    let mut out = String::new();
    let mut args = args.iter();
    let mut it = fmt.chars().peekable();
    while let Some(c) = it.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let mut spec = String::new();
        while let Some(&d) = it
            .peek()
            .filter(|d| d.is_ascii_digit() || matches!(**d, '.' | '$' | 'l' | 'h' | '-'))
        {
            spec.push(d);
            it.next();
        }
        let Some(conv) = it.next() else { break };
        if conv == '%' {
            out.push('%');
            continue;
        }
        // ponytail: ignores positional `%n$`; rufus.loc has none
        let spec = spec.trim_end_matches(['l', 'h']);
        let arg = args.next().copied().unwrap_or("");
        let zero = spec.starts_with('0') && !spec.starts_with("0.");
        let (width, prec) = spec.split_once('.').unwrap_or((spec, ""));
        let width: usize = width.parse().unwrap_or(0);
        let s = match conv {
            'x' | 'X' => match arg.parse::<i64>() {
                Ok(n) if conv == 'X' => format!("{n:X}"),
                Ok(n) => format!("{n:x}"),
                Err(_) => arg.into(),
            },
            'f' => match (arg.parse::<f64>(), prec.parse::<usize>()) {
                (Ok(v), Ok(p)) => format!("{v:.p$}"),
                (Ok(v), _) => format!("{v:.6}"),
                _ => arg.into(),
            },
            _ => arg.into(),
        };
        let pad = width.saturating_sub(s.chars().count());
        let (sign, digits) = if zero && s.starts_with('-') {
            ("-", &s[1..])
        } else {
            ("", &s[..])
        };
        out.push_str(sign);
        out.extend(std::iter::repeat_n(if zero { '0' } else { ' ' }, pad));
        out.push_str(digits);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // CURRENT is global: keep language-switching assertions in one test.
    #[test]
    fn loc_translations() {
        let langs = languages();
        assert_eq!(langs[0].code, "en-US");
        assert!(langs.len() >= 30);
        assert!(
            langs
                .iter()
                .any(|l| l.code == "fr-FR" && l.name == "French (Français)")
        );
        set_language("en-US");
        assert_eq!(t("IDS_DEVICE_TXT"), "Device");
        assert_eq!(
            t("MSG_002"),
            "Another Rufus application is running.\nPlease close the first application before running another one."
        );
        assert_eq!(t("NO_SUCH_ID"), "NO_SUCH_ID");
        set_language("fr-FR");
        assert_eq!(current(), "fr-FR");
        assert_eq!(t("IDS_DEVICE_TXT"), "Périphérique");
        set_language("xx-XX");
        assert_eq!(current(), "en-US");
    }

    #[test]
    fn loc_fallback_to_en_us() {
        let p = parse(
            "l \"en-US\" \"English\" 0x0409\nt A \"en a\"\nt B \"en b\"\nl \"fr-FR\" \"French\" 0x040c\nb \"en-US\"\nt A \"fr a\"\n",
        );
        assert_eq!(p.strings[1].get("A").unwrap(), "fr a");
        assert!(!p.strings[1].contains_key("B"));
    }

    #[test]
    fn loc_printf() {
        assert_eq!(printf("%d of %s (%%)", &["3", "x"]), "3 of x (%)");
        assert_eq!(printf("0x%02X (Disk %d)", &["10", "2"]), "0x0A (Disk 2)");
        assert_eq!(printf("%d:%02d", &["5", "7"]), "5:07");
        assert_eq!(printf("%0.2f%%", &["1.5"]), "1.50%");
        assert_eq!(strip_amp("&Start && go"), "Start & go");
        assert_eq!(tf("NO_SUCH_%s", &[]), "NO_SUCH_");
    }

    #[test]
    fn loc_system_mapping() {
        let codes = [
            "en-US", "fr-FR", "pt-BR", "pt-PT", "zh-CN", "zh-TW", "nb-NO", "es-ES",
        ];
        assert_eq!(closest("fr_FR.UTF-8", &codes), "fr-FR");
        assert_eq!(closest("fr_CA.UTF-8", &codes), "fr-FR");
        assert_eq!(closest("pt-PT", &codes), "pt-PT");
        assert_eq!(closest("pt_AO", &codes), "pt-BR");
        assert_eq!(closest("es_MX", &codes), "es-ES");
        assert_eq!(closest("zh_HK", &codes), "zh-TW");
        assert_eq!(closest("zh-Hans-CN", &codes), "zh-CN");
        assert_eq!(closest("nn_NO", &codes), "nb-NO");
        assert_eq!(closest("en_GB", &codes), "en-US");
        assert_eq!(closest("xx_YY", &codes), "en-US");
        assert_eq!(closest("C", &codes), "en-US");
    }
}
