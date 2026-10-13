// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! Fido.ps1 itself, driven in its command-line mode (`powershell.exe` on
//! Windows, `pwsh` elsewhere), as Rufus does.

use super::{FIELDS, script};
use crate::net::cache_dir;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

const FLAGS: [&str; 5] = ["-Win", "-Rel", "-Ed", "-Lang", "-Arch"];

/// PowerShell executable to use, if any.
pub(super) fn shell() -> Option<String> {
    if cfg!(windows) {
        // Like Rufus: the system copy, not whatever is first in PATH.
        let root = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let p = root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
        return Some(if p.is_file() {
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
}

/// Writes the signature-checked script where PowerShell runs it from. It is
/// rewritten before every run, so what runs is what was verified.
fn script_file(shell: &str) -> Result<PathBuf, String> {
    let s = script()?;
    let text = if cfg!(windows) {
        s.text.clone()
    } else {
        // Fido's CLI refuses non-Windows hosts ($winver is 0.0 there); relax only that
        // check (the first one, in the CLI block) so pwsh on Linux/macOS works.
        s.text.replacen(
            "if ($winver -le 6.1) {",
            "if ($winver -le 6.1 -and $winver -ne 0.0) {",
            1,
        )
    };
    let path = cache_dir().join("fido").join(&s.tag).join("Fido.ps1");
    std::fs::create_dir_all(path.parent().unwrap()).ok();
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    if cfg!(windows) {
        authenticode(shell, &path)?;
    }
    Ok(path)
}

/// Windows also checks the script's own Authenticode signature (Akeo Consulting,
/// Fido's publisher), on top of Rufus' signature of the download. Once per run of
/// Rustfus: the file always holds the same verified text.
fn authenticode(shell: &str, path: &std::path::Path) -> Result<(), String> {
    static CHECKED: OnceLock<Result<(), String>> = OnceLock::new();
    CHECKED
        .get_or_init(|| {
            let p = path.to_string_lossy().replace('\'', "''");
            let out = Command::new(shell)
                .args(["-NoProfile", "-NonInteractive", "-Command"])
                .arg(format!(
                    "$s = Get-AuthenticodeSignature -LiteralPath '{p}'; \
                     if ($s.Status -eq 'Valid' -and $s.SignerCertificate.Subject -match 'CN=Akeo Consulting') {{ 'OK' }} \
                     else {{ \"$($s.Status) $($s.SignerCertificate.Subject)\" }}"
                ))
                .output()
                .map_err(|e| format!("PowerShell: {e}"))?;
            match String::from_utf8_lossy(&out.stdout).trim() {
                "OK" => Ok(()),
                s => Err(format!("Fido.ps1 Authenticode signature is not valid: {s}")),
            }
        })
        .clone()
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

fn run(shell: &str, extra: &[String]) -> Result<(bool, String), String> {
    let arch = match std::env::consts::ARCH {
        "aarch64" => "ARM64",
        "x86" => "x86",
        _ => "x64",
    };
    let out = Command::new(shell)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(script_file(shell)?)
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

pub(super) fn list(shell: &str, chosen: &[String]) -> Result<Vec<String>, String> {
    let flag = FLAGS
        .get(chosen.len())
        .ok_or("all Fido fields already chosen")?;
    let mut a = args(chosen)?;
    a.extend([flag.to_string(), "List".into()]);
    // Fido exits 1 after listing, so the status is meaningless here.
    parse_list(&run(shell, &a)?.1)
}

pub(super) fn url(shell: &str, chosen: &[String]) -> Result<String, String> {
    let mut a = args(chosen)?;
    a.push("-GetUrl".into());
    let (ok, text) = run(shell, &a)?;
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
}
