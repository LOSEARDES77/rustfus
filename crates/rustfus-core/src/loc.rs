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
                unescape(
                    body,
                    map.get_mut(id)
                        .expect("`last` is set after inserting its id"),
                );
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
    *CURRENT.write().expect("language lock poisoned") = idx;
}

pub fn current() -> String {
    parsed().langs[*CURRENT.read().expect("language lock poisoned")]
        .code
        .clone()
}

/// Closest Rufus language for a BCP 47 tag (see `wue::locale`) such as
/// `fr-CA` or `zh-Hant-HK`: exact match, else same primary language.
fn closest(sys: &str, codes: &[&str]) -> String {
    let lower = sys.to_ascii_lowercase();
    if let Some(c) = codes.iter().find(|c| c.eq_ignore_ascii_case(sys)) {
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
    let cur = *CURRENT.read().expect("language lock poisoned");
    [cur, 0]
        .iter()
        .find_map(|&i| p.strings[i].get(id))
        .map_or_else(|| id.to_string(), |s| strip_amp(s))
}

/// `t(id)` with `%s`/`%d` filled from `args` in order (`%%` is a literal `%`).
pub fn tf(id: &str, args: &[&str]) -> String {
    printf(&t(id), args)
}

// ponytail: width/precision/hex (`%02X`, `%0.2f`) are skipped and the argument inserted
// as given; no string Rustfus uses needs them.
fn printf(fmt: &str, args: &[&str]) -> String {
    let mut args = args.iter();
    let mut out = String::new();
    let mut it = fmt.chars().peekable();
    while let Some(c) = it.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        while it
            .next_if(|d| d.is_ascii_digit() || matches!(d, '.' | '-' | 'l' | 'h'))
            .is_some()
        {}
        match it.next() {
            Some('%') | None => out.push('%'),
            Some(_) => out.push_str(args.next().copied().unwrap_or("")),
        }
    }
    out
}

#[cfg(test)]
#[path = "../tests/unit/loc.rs"]
mod tests;
