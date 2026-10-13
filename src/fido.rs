// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! Official Windows ISO URLs through Rufus' Fido PowerShell script, driven in
//! its command-line mode (`powershell.exe` on Windows, `pwsh` elsewhere).

use crate::net::{cache_dir, fetch, fetch_signed};
use std::io::Read;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

pub const FIELDS: [&str; 5] = ["Version", "Release", "Edition", "Language", "Architecture"];
const FLAGS: [&str; 5] = ["-Win", "-Rel", "-Ed", "-Lang", "-Arch"];
/// Rufus' `FIDO_VERSION`: the `Fido.ver` key this Rufus version reads.
const FIDO_VERSION: &str = "z1";

/// PowerShell executable to use, or an error explaining pwsh must be installed (Linux/macOS).
pub fn shell() -> Result<String, String> {
    if cfg!(windows) {
        // Like Rufus: the system copy, not whatever is first in PATH.
        let root = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let p = root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
        return Ok(if p.is_file() {
            p.to_string_lossy().into_owned()
        } else {
            "powershell.exe".into()
        });
    }
    // GUI launches (macOS Finder) get a minimal PATH, so also try usual install dirs.
    let extra = [
        "/usr/local/bin",
        "/opt/homebrew/bin",
        "/snap/bin",
        "/opt/microsoft/powershell/7",
    ]
    .map(PathBuf::from);
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .chain(extra)
        .map(|d| d.join("pwsh"))
        .find(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned())
        .ok_or_else(|| "Downloading Windows ISOs needs PowerShell 7 (pwsh) installed: https://aka.ms/powershell".into())
}

/// `token = value` lookup, like Rufus' `get_token_data_buffer`.
fn token<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        (k.trim() == key).then(|| v.trim())
    })
}

/// Downloads + verifies + caches Fido.ps1, returns its path.
pub fn script() -> Result<PathBuf, String> {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    if let Some(p) = PATH.get() {
        return Ok(p.clone());
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
    // Fido's CLI refuses non-Windows hosts ($winver is 0.0 there); relax only that
    // check (the first one, in the CLI block) so pwsh on Linux/macOS works.
    let ps1 = String::from_utf8_lossy(&ps1).replacen(
        "if ($winver -le 6.1) {",
        "if ($winver -le 6.1 -and $winver -ne 0.0) {",
        1,
    );
    let path = cache_dir().join("fido").join(tag).join("Fido.ps1");
    std::fs::create_dir_all(path.parent().unwrap()).ok();
    std::fs::write(&path, ps1).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(PATH.get_or_init(|| path).clone())
}

fn valid(v: &str) -> bool {
    !v.is_empty()
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || " ._()/,-".contains(c))
}

/// Fido arguments for the chosen values. Fido matches `-Rel` with StartsWith (so
/// we pass the first word) and the rest as regexes (so we anchor and escape them;
/// Fido already escapes parentheses in `-Lang` itself).
fn args(chosen: &[String]) -> Result<Vec<String>, String> {
    if chosen.len() > FLAGS.len() {
        return Err("too many Fido selections".into());
    }
    let mut out = Vec::new();
    for (i, v) in chosen.iter().enumerate() {
        let v = v.trim();
        if !valid(v) {
            return Err(format!("invalid {} value: {v:?}", FIELDS[i]));
        }
        let arg = match i {
            1 => v.split(' ').next().unwrap_or(v).to_string(),
            _ => {
                let meta: &[char] = if i == 3 { &['.'] } else { &['.', '(', ')'] };
                let esc: String = v
                    .chars()
                    .flat_map(|c| (meta.contains(&c).then_some('\\')).into_iter().chain([c]))
                    .collect();
                format!("^{esc}\\s*$")
            }
        };
        out.extend([FLAGS[i].to_string(), arg]);
    }
    Ok(out)
}

fn run(extra: &[String]) -> Result<(bool, String), String> {
    let arch = match std::env::consts::ARCH {
        "aarch64" => "ARM64",
        "x86" => "x86",
        _ => "x64",
    };
    let out = Command::new(shell()?)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(script()?)
        // Passing the arch also skips Fido's Windows-only WMI query.
        .args(["-PlatformArch", arch])
        .args(extra)
        .output()
        .map_err(|e| format!("PowerShell: {e}"))?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok((out.status.success(), text))
}

/// Most useful message from Fido's output on failure.
fn failure(text: &str) -> String {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    lines
        .iter()
        .find_map(|l| l.strip_prefix("Error:"))
        .map(|e| e.trim().to_string())
        .or_else(|| lines.first().map(|l| l.to_string()))
        .unwrap_or_else(|| "Fido returned nothing".into())
}

/// `Write-Host " -" $item` lines after the "Please select ..." header.
fn parse_list(text: &str) -> Result<Vec<String>, String> {
    let items: Vec<String> = text
        .lines()
        .skip_while(|l| !l.trim_start().starts_with("Please select"))
        .filter_map(|l| l.trim_start().strip_prefix("- "))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if items.is_empty() {
        Err(failure(text))
    } else {
        Ok(items)
    }
}

/// `-GetUrl` prints the URL as pipeline output (verbosity is 0 in that mode).
fn parse_url(text: &str) -> Result<String, String> {
    text.lines()
        .map(str::trim)
        .rfind(|l| l.starts_with("https://"))
        .map(String::from)
        .ok_or_else(|| failure(text))
}

/// Lists choices for the next unset field, given the already chosen ones in order (Win, Rel, Ed, Lang, Arch).
pub fn list(chosen: &[String]) -> Result<Vec<String>, String> {
    let flag = FLAGS
        .get(chosen.len())
        .ok_or("all Fido fields already chosen")?;
    let mut a = args(chosen)?;
    a.extend([flag.to_string(), "List".into()]);
    // Fido exits 1 after listing, so the status is meaningless here.
    parse_list(&run(&a)?.1)
}

/// Final download URL for a full selection (uses -GetUrl).
pub fn url(chosen: &[String]) -> Result<String, String> {
    if chosen.len() != FLAGS.len() {
        return Err("incomplete Fido selection".into());
    }
    let mut a = args(chosen)?;
    a.push("-GetUrl".into());
    let (ok, text) = run(&a)?;
    parse_url(&text).and_then(|u| {
        if ok || !text.contains("Error:") {
            Ok(u)
        } else {
            Err(failure(&text))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn parses_lists() {
        let out = "Please select a Windows Version (-Win):\n - Windows 11\n - Windows 10\n - UEFI Shell 2.2\n";
        assert_eq!(
            parse_list(out).unwrap(),
            s(&["Windows 11", "Windows 10", "UEFI Shell 2.2"])
        );
        let out = "Please select a Windows Release (-Rel) for Windows 11 (or use 'Latest' for most recent):\r\n - 26H2 (Build 26300.9457 - 2026.09)\r\n";
        assert_eq!(
            parse_list(out).unwrap(),
            s(&["26H2 (Build 26300.9457 - 2026.09)"])
        );
        let out = "Please select an Architecture (-Arch) for UEFI Shell 2.2 26H1,Release, en-us:\n - x64, ia32, aa64\n";
        assert_eq!(parse_list(out).unwrap(), s(&["x64, ia32, aa64"]));
        assert_eq!(
            parse_list("Error: This feature is not available on this platform.\n").unwrap_err(),
            "This feature is not available on this platform."
        );
        assert_eq!(
            parse_list("Invalid Windows version provided.\nUse '-Win List'...\n").unwrap_err(),
            "Invalid Windows version provided."
        );
    }

    #[test]
    fn parses_url() {
        let u = "https://software.download.prss.microsoft.com/dbazure/Win11_26H2_English_x64.iso?t=abc&e=1";
        assert_eq!(parse_url(&format!("{u}\n")).unwrap(), u);
        assert_eq!(
            parse_url("Error: Your IP address has been banned\n").unwrap_err(),
            "Your IP address has been banned"
        );
    }

    #[test]
    fn builds_args() {
        let a = args(&s(&[
            "Windows 11",
            "26H2 (Build 26300.9457 - 2026.09)",
            "Windows 11 Home/Pro/Edu",
            "English (United States)",
            "x64",
        ]))
        .unwrap();
        assert_eq!(
            a,
            s(&[
                "-Win",
                r"^Windows 11\s*$",
                "-Rel",
                "26H2",
                "-Ed",
                r"^Windows 11 Home/Pro/Edu\s*$",
                "-Lang",
                r"^English (United States)\s*$",
                "-Arch",
                r"^x64\s*$"
            ])
        );
        assert_eq!(
            args(&s(&["UEFI Shell 2.2"])).unwrap()[1],
            r"^UEFI Shell 2\.2\s*$"
        );
        for bad in ["", "a;b", "$(x)", "a\"b", "a`b", "a|b", "a\nb", "a[0]", "*"] {
            assert!(args(&s(&[bad])).is_err(), "{bad:?}");
        }
        assert!(args(&s(&["a"; 6])).is_err());
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
        let p = script().unwrap();
        assert!(std::fs::read_to_string(&p).unwrap().contains("$GetUrl"));
        if shell().is_ok() {
            let v = list(&[]).unwrap();
            assert!(v.iter().any(|x| x.starts_with("Windows")), "{v:?}");
        }
    }
}
