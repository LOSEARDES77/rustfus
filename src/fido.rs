// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
// Based on Fido, Copyright © 2019-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! Official Windows ISO URLs, as Rufus' Fido PowerShell script gets them, without
//! PowerShell: the version table is read from the signed Fido.ps1 and Microsoft's
//! download API is queried through curl.

use crate::net::{fetch, fetch_signed, fetch_with};
use std::io::Read;
use std::sync::{Mutex, OnceLock};

pub const FIELDS: [&str; 5] = ["Version", "Release", "Edition", "Language", "Architecture"];
/// Rufus' `FIDO_VERSION`: the `Fido.ver` key this Rufus version reads.
const FIDO_VERSION: &str = "z1";
// Fido's constants for Microsoft's download "protection".
const ORG_ID: &str = "y6jn8c31";
const PROFILE_ID: &str = "606624d44113";
const INSTANCE_ID: &str = "560dc9f3-1aa5-4a2f-b63c-9e18f8d0e175";
const LOCALE: &str = "en-US";

/// One `$WindowsVersions` entry of Fido.ps1.
#[derive(Debug, PartialEq)]
struct Version {
    name: String,
    page: String,
    releases: Vec<Release>,
}

#[derive(Debug, PartialEq)]
struct Release {
    name: String,
    /// Edition name and its product edition IDs (one per session: x64, ARM64).
    editions: Vec<(String, Vec<u32>)>,
}

/// SKUs per language and download links per arch from the last queries, since
/// Microsoft only serves links for the session that listed the SKUs.
/// Language name and its (SKU ID, session ID) pairs.
type Langs = Vec<(String, Vec<(String, String)>)>;

#[derive(Default)]
struct Session {
    langs: Langs,
    links: Vec<(String, String)>,
}
static SESSION: Mutex<Session> = Mutex::new(Session {
    langs: Vec::new(),
    links: Vec::new(),
});

/// `token = value` lookup, like Rufus' `get_token_data_buffer`.
fn token<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        (k.trim() == key).then(|| v.trim())
    })
}

/// Downloads + verifies Fido.ps1 and returns its Windows version table.
fn versions() -> Result<&'static [Version], String> {
    static VERSIONS: OnceLock<Vec<Version>> = OnceLock::new();
    if let Some(v) = VERSIONS.get() {
        return Ok(v);
    }
    // Rufus reads the URL from rufus.ie/Fido.ver so each Rufus version can pin its Fido.
    let ver = String::from_utf8_lossy(&fetch("https://rufus.ie/Fido.ver")?).into_owned();
    let url = token(&ver, FIDO_VERSION)
        .filter(|u| u.starts_with("https://github.com/pbatard/Fido/"))
        .ok_or("Fido.ver: no valid download script URL")?;
    // Cache per release tag so a new Fido.ver entry isn't shadowed by an old cached copy.
    let tag = url.rsplit('/').nth(1).unwrap_or("fido");
    let lzma = fetch_signed(url, &format!("fido/{tag}/Fido.ps1.lzma"))?;
    // LZMA "alone" (.lzma) format, as decoded by Rufus' bled; size is at offset 5.
    let size = lzma
        .get(5..13)
        .map(|b| u64::from_le_bytes(b.try_into().unwrap()))
        .unwrap_or(u64::MAX);
    if size >= 1 << 20 {
        return Err("Fido.ps1.lzma: bad uncompressed size".into());
    }
    let mut ps1 = Vec::new();
    lzma_rust2::LzmaReader::new_mem_limit(&lzma[..], u32::MAX, None)
        .and_then(|mut r| r.read_to_end(&mut ps1))
        .map_err(|e| format!("Fido.ps1.lzma: {e}"))?;
    if ps1.len() as u64 != size {
        return Err("Fido.ps1.lzma: could not decompress".into());
    }
    let v = parse_versions(&String::from_utf8_lossy(&ps1))?;
    Ok(VERSIONS.get_or_init(|| v))
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

fn parse_versions(ps1: &str) -> Result<Vec<Version>, String> {
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
                        .map(|n| {
                            if let Value::Num(n) = n {
                                Ok(*n)
                            } else {
                                Err(bad())
                            }
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

/// Random version 4 UUID for Microsoft's session IDs.
fn uuid() -> String {
    use std::hash::BuildHasher;
    let r =
        || std::collections::hash_map::RandomState::new().hash_one(std::time::SystemTime::now());
    let (a, b) = (r(), r());
    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        a >> 32,
        (a >> 16) & 0xffff,
        a & 0xfff,
        (b >> 48) & 0x3fff | 0x8000,
        b & 0xffff_ffff_ffff
    )
}

fn json(url: &str, args: &[&str]) -> Result<serde_json::Value, String> {
    serde_json::from_slice(&fetch_with(url, args)?).map_err(|e| format!("{url}: {e}"))
}

/// First Microsoft API error, mapping type 9 (banned) to Fido's explanation.
fn api_error(r: &serde_json::Value, session: &str) -> Option<String> {
    let e = r["Errors"].get(0)?;
    Some(if e["Type"] == 9 {
        format!(
            "Your IP address has been banned by Microsoft for issuing too many ISO download requests or for \
             belonging to a region of the world where sanctions currently apply. Please try again later. \
             If you believe this ban to be in error, you can try contacting Microsoft by referring to \
             message code 715-123130 and session ID {session}."
        )
    } else {
        e["Value"]
            .as_str()
            .unwrap_or("Microsoft server error")
            .to_string()
    })
}

/// Fido's `Get-Windows-Languages` for Windows: whitelists a new session per
/// edition ID, then lists that edition's SKUs per language.
fn query_languages(ids: &[u32]) -> Result<Langs, String> {
    let mut langs: Langs = Vec::new();
    for id in ids {
        let session = uuid();
        fetch(&format!(
            "https://vlscppe.microsoft.com/tags?org_id={ORG_ID}&session_id={session}"
        ))?;
        let mdt = String::from_utf8_lossy(&fetch(&format!(
            "https://ov-df.microsoft.com/mdt.js?instanceId={INSTANCE_ID}&PageId=si&session_id={session}"
        ))?)
        .into_owned();
        let after = |key: &str, ok: fn(char) -> bool| {
            let v: String = mdt
                .split(key)
                .nth(1)?
                .trim_start_matches('+')
                .chars()
                .take_while(|&c| ok(c))
                .collect();
            (!v.is_empty()).then_some(v)
        };
        let w = after("&w=", |c| c.is_ascii_hexdigit())
            .or_else(|| after("?w=", |c| c.is_ascii_hexdigit()));
        let rticks = after("rticks=\"", |c| c.is_ascii_digit());
        let (Some(w), Some(rticks)) = (w, rticks) else {
            return Err("Could not extract ov-df data".into());
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        fetch(&format!(
            "https://ov-df.microsoft.com/?session_id={session}&CustomerId={INSTANCE_ID}&PageId=si&w={w}&mdt={now}&rticks={rticks}"
        ))?;
        let url = format!(
            "https://www.microsoft.com/software-download-connector/api/getskuinformationbyproductedition\
             ?profile={PROFILE_ID}&productEditionId={id}&SKU=undefined&friendlyFileName=undefined&Locale={LOCALE}&sessionID={session}"
        );
        // Like Fido: may need a few attempts.
        let mut attempt = 0;
        let r = loop {
            let r = json(&url, &[]);
            let err = match &r {
                Ok(v) => api_error(v, &session).or_else(|| {
                    v["Skus"]
                        .as_array()
                        .is_none_or(Vec::is_empty)
                        .then(|| "Could not parse languages".into())
                }),
                Err(e) => Some(e.clone()),
            };
            match err {
                None => break r?,
                Some(e) if attempt >= 2 => return Err(e),
                Some(_) => {
                    attempt += 1;
                    std::thread::sleep(std::time::Duration::from_secs(2));
                }
            }
        };
        for sku in r["Skus"].as_array().into_iter().flatten() {
            let (Some(lang), Some(sku)) = (sku["Language"].as_str(), sku["Id"].as_str()) else {
                continue;
            };
            let entry = (sku.to_string(), session.clone());
            match langs.iter_mut().find(|(l, _)| l == lang) {
                Some((_, v)) => v.push(entry),
                None => langs.push((lang.to_string(), vec![entry])),
            }
        }
    }
    Ok(langs)
}

/// Fido's `Get-Windows-Download-Links` for Windows: one link per arch.
fn query_links(skus: &[(String, String)]) -> Result<Vec<(String, String)>, String> {
    let mut links = Vec::new();
    for (sku, session) in skus {
        let url = format!(
            "https://www.microsoft.com/software-download-connector/api/GetProductDownloadLinksBySku\
             ?profile={PROFILE_ID}&productEditionId=undefined&SKU={sku}&friendlyFileName=undefined&Locale={LOCALE}&sessionID={session}"
        );
        // Microsoft's servers may deny the request without this referer.
        let r = json(
            &url,
            &[
                "-H",
                "Referer: https://www.microsoft.com/software-download/windows11",
            ],
        )?;
        if let Some(e) = api_error(&r, session) {
            return Err(e);
        }
        for o in r["ProductDownloadOptions"].as_array().into_iter().flatten() {
            let arch = match o["DownloadType"].as_u64() {
                Some(0) => "x86",
                Some(1) => "x64",
                Some(2) => "ARM64",
                _ => "Unknown",
            };
            if let Some(uri) = o["Uri"].as_str() {
                links.push((arch.to_string(), uri.to_string()));
            }
        }
    }
    if links.is_empty() {
        return Err("Could not retrieve ISO download links".into());
    }
    Ok(links)
}

/// UEFI Shell ISOs come from GitHub, with the archs listed in the release's Version.xml.
fn uefi_shell_link(v: &Version, r: &Release, debug: bool) -> Result<(String, String), String> {
    let tag = r.name.split(' ').next().unwrap_or_default();
    let shell = v.page.split(' ').nth(1).unwrap_or_default();
    let base = format!("https://github.com/pbatard/UEFI-Shell/releases/download/{tag}");
    let xml = String::from_utf8_lossy(&fetch(&format!("{base}/Version.xml"))?).into_owned();
    let archs: Vec<&str> = xml
        .split("<arch>")
        .skip(1)
        .filter_map(|a| a.split("</arch>").next())
        .map(str::trim)
        .collect();
    let kind = if debug { "DEBUG" } else { "RELEASE" };
    Ok((
        archs.join(", "),
        format!("{base}/UEFI-Shell-{shell}-{tag}-{kind}.iso"),
    ))
}

/// Lists choices for the next unset field, given the already chosen ones in order (Win, Rel, Ed, Lang, Arch).
pub fn list(chosen: &[String]) -> Result<Vec<String>, String> {
    let vs = versions()?;
    let names = |it: &mut dyn Iterator<Item = &String>| it.cloned().collect::<Vec<_>>();
    let Some(want) = chosen.first() else {
        return Ok(names(&mut vs.iter().map(|v| &v.name)));
    };
    let v = vs
        .iter()
        .find(|v| &v.name == want)
        .ok_or("Invalid Windows version")?;
    let Some(want) = chosen.get(1) else {
        return Ok(names(&mut v.releases.iter().map(|r| &r.name)));
    };
    let r = v
        .releases
        .iter()
        .find(|r| &r.name == want)
        .ok_or("Invalid Windows release")?;
    let Some(want) = chosen.get(2) else {
        // Like Fido, China-only editions only for Chinese users.
        let zh = crate::loc::current().starts_with("zh");
        return Ok(names(
            &mut r
                .editions
                .iter()
                .map(|e| &e.0)
                .filter(|e| zh || !e.contains("China")),
        ));
    };
    let ids = &r
        .editions
        .iter()
        .find(|e| &e.0 == want)
        .ok_or("Invalid Windows edition")?
        .1;
    let uefi = v.page.starts_with("UEFI_SHELL");
    let mut s = SESSION.lock().unwrap();
    let Some(want) = chosen.get(3) else {
        *s = Session::default();
        if uefi {
            return Ok(vec!["English (US)".into()]);
        }
        s.langs = query_languages(ids)?;
        return Ok(names(&mut s.langs.iter().map(|l| &l.0)));
    };
    if chosen.len() > 4 {
        return Err("all Fido fields already chosen".into());
    }
    s.links = if uefi {
        vec![uefi_shell_link(v, r, ids.first() == Some(&1))?]
    } else {
        let skus = &s
            .langs
            .iter()
            .find(|l| &l.0 == want)
            .ok_or("Invalid Windows language")?
            .1;
        query_links(skus)?
    };
    Ok(names(&mut s.links.iter().map(|l| &l.0)))
}

/// Final download URL for a full selection, from the links the arch list fetched.
pub fn url(chosen: &[String]) -> Result<String, String> {
    if chosen.len() != FIELDS.len() {
        return Err("incomplete Fido selection".into());
    }
    let s = SESSION.lock().unwrap();
    s.links
        .iter()
        .find(|l| l.0 == chosen[4])
        .map(|l| l.1.clone())
        .ok_or_else(|| "Download link expired, please go back and select the language again".into())
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

    #[test]
    fn makes_uuid() {
        let u = uuid();
        assert_eq!(u.len(), 36);
        assert_eq!(u.as_bytes()[14], b'4');
        assert_ne!(u, uuid());
    }

    #[test]
    fn finds_token() {
        let ver = "# comment\nz0 = https://old\nz1 = https://github.com/pbatard/Fido/releases/download/v1.71/Fido.ps1.lzma\n";
        assert_eq!(
            token(ver, "z1"),
            Some("https://github.com/pbatard/Fido/releases/download/v1.71/Fido.ps1.lzma")
        );
        assert_eq!(token(ver, "z2"), None);
    }

    /// Network test: run with `cargo test fido_live -- --ignored`.
    #[test]
    #[ignore]
    fn fido_live() {
        let mut chosen = Vec::new();
        for want in [
            "Windows 11",
            "",
            "Windows 11 Home/Pro/Edu",
            "English",
            "x64",
        ] {
            let opts = list(&chosen).unwrap();
            let pick = opts
                .iter()
                .find(|o| want.is_empty() || *o == want)
                .unwrap_or_else(|| panic!("{want:?} not in {opts:?}"));
            chosen.push(pick.clone());
        }
        let u = url(&chosen).unwrap();
        assert!(u.starts_with("https://") && u.contains(".iso"), "{u}");
        chosen.truncate(1);
        chosen[0] = "UEFI Shell 2.2".into();
        for _ in 0..4 {
            chosen.push(list(&chosen).unwrap()[0].clone());
        }
        assert!(url(&chosen).unwrap().ends_with("-RELEASE.iso"));
    }
}
