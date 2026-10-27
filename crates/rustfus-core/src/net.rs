// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! Downloads through the system `curl` (shipped with Linux distros, macOS
//! and Windows 10+), plus verification of Rufus' signed files.

use rsa::{BigUint, Pkcs1v15Sign, RsaPublicKey};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub const FILES_URL: &str = "https://rufus.ie/files";
static RUFUS_PUBKEY: &[u8] = include_bytes!("../../../res/rufus_pubkey.bin");

/// Per-user cache, like Rufus' `%LOCALAPPDATA%\Rufus`.
pub fn cache_dir() -> PathBuf {
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Caches"))
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
    };
    base.unwrap_or_else(std::env::temp_dir).join("rustfus")
}

fn curl() -> Command {
    let mut c = Command::new(if cfg!(windows) { "curl.exe" } else { "curl" });
    c.args([
        "-fsSL",
        "--proto",
        "=https",
        "--retry",
        "2",
        "--connect-timeout",
        "15",
    ]);
    c
}

pub fn fetch(url: &str) -> Result<Vec<u8>, String> {
    fetch_with(url, &[])
}

/// `fetch` with extra curl arguments, such as `-H` headers.
pub fn fetch_with(url: &str, args: &[&str]) -> Result<Vec<u8>, String> {
    let out = curl()
        .args(args)
        .arg(url)
        .output()
        .map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "download of {url} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(out.stdout)
}

/// RSA-2048 PKCS#1 v1.5 / SHA-256 check against Rufus' public key.
pub fn verify(data: &[u8], sig: &[u8]) -> bool {
    // DER DigestInfo prefix for SHA-256.
    const PREFIX: [u8; 19] = [
        0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01,
        0x05, 0x00, 0x04, 0x20,
    ];
    let Ok(key) = RsaPublicKey::new(
        BigUint::from_bytes_be(RUFUS_PUBKEY),
        BigUint::from(65537u32),
    ) else {
        return false;
    };
    let scheme = Pkcs1v15Sign {
        hash_len: Some(32),
        prefix: Box::new(PREFIX),
    };
    key.verify(scheme, &Sha256::digest(data), sig).is_ok()
}

/// Where [`fetch_signed`] caches `rel` and its signature.
fn cache_paths(rel: &str) -> (PathBuf, PathBuf) {
    let path = cache_dir().join(rel);
    let sig_path = path.with_extension(format!(
        "{}sig",
        path.extension()
            .map(|e| format!("{}.", e.to_string_lossy()))
            .unwrap_or_default()
    ));
    (path, sig_path)
}

/// The copy of `rel` an earlier [`fetch_signed`] cached, if its signature still checks out.
pub fn cached_signed(rel: &str) -> Option<Vec<u8>> {
    let (path, sig_path) = cache_paths(rel);
    let data = std::fs::read(&path).ok()?;
    let sig = std::fs::read(&sig_path).ok()?;
    verify(&data, &sig).then_some(data)
}

/// Fetches a file signed by Rufus (`url` + `.sig`), caching it under
/// `cache_dir()/rel`. Cached copies are re-verified on every use.
pub fn fetch_signed(url: &str, rel: &str) -> Result<Vec<u8>, String> {
    if let Some(data) = cached_signed(rel) {
        return Ok(data);
    }
    let (path, sig_path) = cache_paths(rel);
    let data = fetch(url)?;
    let sig = fetch(&format!("{url}.sig"))?;
    if !verify(&data, &sig) {
        return Err(format!("{url}: signature check failed"));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    std::fs::write(&path, &data).ok();
    std::fs::write(&sig_path, &sig).ok();
    Ok(data)
}

/// Downloads a large file to `dest`, reporting progress from the growing
/// file size against the server's Content-Length.
pub fn download(
    url: &str,
    dest: &Path,
    progress: &dyn Fn(f32, u64),
    cancel: &AtomicBool,
) -> Result<(), String> {
    let total = curl()
        .args(["-I", url])
        .output()
        .ok()
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|v| v.trim().parse::<u64>().ok())
                })
                .next_back()
        })
        .unwrap_or(0);
    let mut child = curl()
        .arg("-o")
        .arg(dest)
        .arg(url)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("curl: {e}"))?;
    loop {
        if cancel.load(Ordering::Relaxed) {
            child.kill().ok();
            child.wait().ok();
            std::fs::remove_file(dest).ok();
            return Err("Cancelled".into());
        }
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            if !status.success() {
                let mut err = String::new();
                if let Some(mut s) = child.stderr.take() {
                    std::io::Read::read_to_string(&mut s, &mut err).ok();
                }
                std::fs::remove_file(dest).ok();
                return Err(format!("download failed: {}", err.trim()));
            }
            return Ok(());
        }
        let got = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
        progress(
            if total > 0 {
                got as f32 / total as f32
            } else {
                0.0
            },
            got,
        );
        std::thread::sleep(Duration::from_millis(250));
    }
}

#[cfg(test)]
#[path = "../tests/unit/net.rs"]
mod tests;
