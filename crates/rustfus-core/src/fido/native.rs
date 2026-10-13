// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Fido, Copyright © 2019-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! Fido's Microsoft download API queries reimplemented over curl, for systems
//! without PowerShell.

use super::table::{Release, Version};
use super::{FIELDS, versions};
use crate::net::{fetch, fetch_with};
use std::sync::Mutex;

// Fido's constants for Microsoft's download "protection".
const ORG_ID: &str = "y6jn8c31";
const PROFILE_ID: &str = "606624d44113";
const INSTANCE_ID: &str = "560dc9f3-1aa5-4a2f-b63c-9e18f8d0e175";
const LOCALE: &str = "en-US";

/// Language name and its (SKU ID, session ID) pairs.
type Langs = Vec<(String, Vec<(String, String)>)>;

/// SKUs per language and download links per arch from the last queries, since
/// Microsoft only serves links for the session that listed the SKUs.
#[derive(Default)]
struct Session {
    langs: Langs,
    links: Vec<(String, String)>,
}
static SESSION: Mutex<Session> = Mutex::new(Session {
    langs: Vec::new(),
    links: Vec::new(),
});

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

pub(super) fn list(chosen: &[String]) -> Result<Vec<String>, String> {
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
    if chosen.len() >= FIELDS.len() {
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

/// Download URL from the links the arch list fetched.
pub(super) fn url(chosen: &[String]) -> Result<String, String> {
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
    fn makes_uuid() {
        let u = uuid();
        assert_eq!(u.len(), 36);
        assert_eq!(u.as_bytes()[14], b'4');
        assert_ne!(u, uuid());
    }
}
