// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
//! The `$WindowsVersions` table of Fido.ps1.

#[derive(Debug, PartialEq)]
pub(super) struct Version {
    pub name: String,
    pub page: String,
    pub releases: Vec<Release>,
}

#[derive(Debug, PartialEq)]
pub(super) struct Release {
    pub name: String,
    /// Edition name and its product edition IDs (one per session: x64, ARM64).
    pub editions: Vec<(String, Vec<u32>)>,
}

#[derive(Debug)]
enum Value {
    Str(String),
    Num(u32),
    List(Vec<Value>),
}

/// Parses PowerShell's nested `@(...)` literal starting at `s` (after its `@(`).
/// Items may be separated by commas or just newlines; `#` starts a comment.
fn parse_list(s: &mut std::iter::Peekable<std::str::Chars>) -> Result<Vec<Value>, String> {
    let mut items = Vec::new();
    while let Some(c) = s.next() {
        match c {
            ')' => return Ok(items),
            '@' if s.peek() == Some(&'(') => {
                s.next();
                items.push(Value::List(parse_list(s)?));
            }
            '(' => items.push(Value::List(parse_list(s)?)),
            '"' => items.push(Value::Str(s.by_ref().take_while(|&c| c != '"').collect())),
            '#' => while s.next_if(|&c| c != '\n').is_some() {},
            '0'..='9' => {
                let mut n = c.to_string();
                while let Some(d) = s.next_if(char::is_ascii_digit) {
                    n.push(d);
                }
                items.push(Value::Num(n.parse().map_err(|_| "Fido.ps1: bad number")?));
            }
            c if c.is_whitespace() || c == ',' => {}
            c => return Err(format!("Fido.ps1: unexpected {c:?} in version table")),
        }
    }
    Err("Fido.ps1: unterminated version table".into())
}

pub(super) fn parse_versions(ps1: &str) -> Result<Vec<Version>, String> {
    let start = ps1
        .find("$WindowsVersions = @(")
        .ok_or("Fido.ps1: no version table")?;
    let mut s = ps1[start + "$WindowsVersions = @(".len()..]
        .chars()
        .peekable();
    let bad = || "Fido.ps1: unexpected version table layout".to_string();
    let str_of = |v: &Value| match v {
        Value::Str(s) => Ok(s.trim().to_string()),
        _ => Err(bad()),
    };
    let mut out = Vec::new();
    for v in parse_list(&mut s)? {
        let Value::List(v) = v else { return Err(bad()) };
        let Some(Value::List(head)) = v.first() else {
            return Err(bad());
        };
        let (Some(name), Some(page)) = (head.first(), head.get(1)) else {
            return Err(bad());
        };
        let mut releases = Vec::new();
        for r in &v[1..] {
            let Value::List(r) = r else { return Err(bad()) };
            let name = str_of(r.first().ok_or_else(bad)?)?;
            let mut editions = Vec::new();
            for e in &r[1..] {
                let Value::List(e) = e else { return Err(bad()) };
                let ids = match e.get(1) {
                    Some(Value::Num(n)) => vec![*n],
                    Some(Value::List(l)) => l
                        .iter()
                        .map(|n| match n {
                            Value::Num(n) => Ok(*n),
                            _ => Err(bad()),
                        })
                        .collect::<Result<_, _>>()?,
                    _ => return Err(bad()),
                };
                editions.push((str_of(e.first().ok_or_else(bad)?)?, ids));
            }
            releases.push(Release { name, editions });
        }
        out.push(Version {
            name: str_of(name)?,
            page: str_of(page)?,
            releases,
        });
    }
    Ok(out)
}

#[cfg(test)]
#[path = "../../tests/unit/fido/table.rs"]
mod tests;
