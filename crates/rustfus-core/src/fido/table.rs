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
mod tests {
    use super::*;

    #[test]
    fn parses_version_table() {
        let ps1 = r#"
$WindowsVersions = @(
	@(
		@("Windows 11", "windows11"),
		@(
			"26H2 (Build 26300.9457 - 2026.09)",
			# a comment, with "quotes" and @(parens)
			@("Windows 11 Home/Pro/Edu", @(3813, 3816)),
			@("Windows 11 Home China ", @(3814, 3817))
		)
	),
	@(
		@("Windows 10", "Windows10ISO"),
		@(
			"22H2 v1 (Build 19045.2965 - 2023.05)",
			@("Windows 10 Home/Pro/Edu", 2618)
		)
	)
	@(
		@("UEFI Shell 2.2", "UEFI_SHELL 2.2"),
		@(
			"26H1 (edk2-stable202602)",
			@("Release", 0),
			@("Debug", 1)
		)
	)
)
#endregion
"#;
        let v = parse_versions(ps1).unwrap();
        assert_eq!(v.len(), 3);
        assert_eq!(v[0].name, "Windows 11");
        assert_eq!(
            v[0].releases[0].editions[0],
            ("Windows 11 Home/Pro/Edu".into(), vec![3813, 3816])
        );
        assert_eq!(v[0].releases[0].editions[1].0, "Windows 11 Home China");
        assert_eq!(v[1].releases[0].editions[0].1, vec![2618]);
        assert_eq!(v[2].page, "UEFI_SHELL 2.2");
        assert_eq!(v[2].releases[0].editions[1], ("Debug".into(), vec![1]));
        assert!(parse_versions("$WindowsVersions = @( @( @(\"a\", \"b\") ").is_err());
    }
}
