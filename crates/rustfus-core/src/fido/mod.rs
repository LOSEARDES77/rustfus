// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! Official Windows ISO URLs through Rufus' Fido script. Fido.ps1 is fetched from
//! the URL in rufus.ie/Fido.ver and checked against Rufus' signature; PowerShell
//! runs it when installed, else its queries are made natively.

use crate::net::{fetch, fetch_signed};
use std::io::Read;
use std::sync::OnceLock;

mod native;
mod pwsh;
mod table;

pub const FIELDS: [&str; 5] = ["Version", "Release", "Edition", "Language", "Architecture"];
/// Rufus' `FIDO_VERSION`: the `Fido.ver` key this Rufus version reads.
const FIDO_VERSION: &str = "z1";

/// Signature-checked Fido.ps1 and its release tag.
struct Script {
    text: String,
    tag: String,
}

/// `token = value` lookup, like Rufus' `get_token_data_buffer`.
fn token<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        (k.trim() == key).then(|| v.trim())
    })
}

/// Downloads Fido.ps1.lzma, checks it against Rufus' signature (cached copies too)
/// and decompresses it.
fn script() -> Result<&'static Script, String> {
    static SCRIPT: OnceLock<Script> = OnceLock::new();
    if let Some(s) = SCRIPT.get() {
        return Ok(s);
    }
    // Rufus reads the URL from rufus.ie/Fido.ver so each Rufus version can pin its Fido.
    let ver = String::from_utf8_lossy(&fetch("https://rufus.ie/Fido.ver")?).into_owned();
    let url = token(&ver, FIDO_VERSION)
        .filter(|u| u.starts_with("https://github.com/pbatard/Fido/"))
        .ok_or("Fido.ver: no valid download script URL")?;
    // Cache per release tag so a new Fido.ver entry isn't shadowed by an old cached copy.
    let tag = url.rsplit('/').nth(1).unwrap_or("fido").to_string();
    let lzma = fetch_signed(url, &format!("fido/{tag}/Fido.ps1.lzma"))?;
    // LZMA "alone" (.lzma) format, as decoded by Rufus' bled; size is at offset 5.
    let size = lzma
        .get(5..13)
        .map(|b| u64::from_le_bytes(b.try_into().expect("8-byte slice")))
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
    let text = String::from_utf8(ps1).map_err(|_| "Fido.ps1: not UTF-8")?;
    Ok(SCRIPT.get_or_init(|| Script { text, tag }))
}

/// Fido's Windows version table, for the native queries.
fn versions() -> Result<&'static [table::Version], String> {
    static VERSIONS: OnceLock<Vec<table::Version>> = OnceLock::new();
    if let Some(v) = VERSIONS.get() {
        return Ok(v);
    }
    let v = table::parse_versions(&script()?.text)?;
    Ok(VERSIONS.get_or_init(|| v))
}

/// Lists choices for the next unset field, given the already chosen ones in order (Win, Rel, Ed, Lang, Arch).
pub fn list(chosen: &[String]) -> Result<Vec<String>, String> {
    match pwsh::shell() {
        Some(sh) => pwsh::list(&sh, chosen),
        None => native::list(chosen),
    }
}

/// Final download URL for a full selection.
pub fn url(chosen: &[String]) -> Result<String, String> {
    if chosen.len() != FIELDS.len() {
        return Err("incomplete Fido selection".into());
    }
    match pwsh::shell() {
        Some(sh) => pwsh::url(&sh, chosen),
        None => native::url(chosen),
    }
}

/// ISO file name from a download URL, for the save dialog or `-o` default.
pub fn file_name(url: &str) -> String {
    url.rsplit('/')
        .next()
        .and_then(|n| n.split('?').next())
        .filter(|n| !n.is_empty())
        .unwrap_or("windows.iso")
        .to_string()
}

#[cfg(test)]
#[path = "../../tests/unit/fido.rs"]
mod tests;
