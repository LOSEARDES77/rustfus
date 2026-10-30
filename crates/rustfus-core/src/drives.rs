// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
use std::process::Command;

#[derive(Clone, Debug, PartialEq)]
pub struct Drive {
    pub path: String,
    pub name: String,
    pub size: u64,
    /// Hardware serial, when the OS reports one (not on macOS).
    pub serial: Option<String>,
}

impl Drive {
    /// Rufus style: "Volume label (G:) [8GB]".
    pub fn label(&self) -> String {
        format!("{} [{}]", self.name, marketed_size(self.size))
    }
}

/// Lists the drives again and checks `d` is still the same disk, so a stick
/// swapped after it was picked is never written.
pub fn verify(d: &Drive) -> Result<(), String> {
    let now = list(true)?;
    let Some(cur) = now.iter().find(|c| c.path == d.path) else {
        return Err(format!("{} is no longer connected", d.path));
    };
    if cur.size != d.size || cur.serial != d.serial {
        return Err(format!(
            "{} is now a different drive ({}); select it again",
            d.path,
            cur.label()
        ));
    }
    Ok(())
}

/// Drive capacity the way it is sold: decimal units, rounded.
pub fn marketed_size(bytes: u64) -> String {
    let gb = bytes as f64 / 1e9;
    if gb >= 1.0 {
        format!("{}GB", gb.round())
    } else {
        format!("{}MB", (bytes as f64 / 1e6).round())
    }
}

pub fn human(bytes: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{v:.1} {}", units[i])
}

pub fn run(cmd: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| format!("{cmd}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{cmd} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Lists writable target disks: USB, SD and MMC only. System and internal
/// (SATA/NVMe) disks are never returned.
pub fn list(all: bool) -> Result<Vec<Drive>, String> {
    imp::list(all)
}

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Lsblk {
        blockdevices: Vec<Dev>,
    }

    #[derive(Deserialize)]
    struct Dev {
        path: String,
        size: Option<u64>,
        model: Option<String>,
        tran: Option<String>,
        rm: bool,
        hotplug: bool,
        #[serde(rename = "type")]
        kind: String,
        mountpoints: Option<Vec<Option<String>>>,
        label: Option<String>,
        serial: Option<String>,
        children: Option<Vec<Dev>>,
    }

    fn hosts_system(d: &Dev) -> bool {
        let sys = ["/", "/boot", "/boot/efi", "/efi", "/usr", "/home", "[SWAP]"];
        d.mountpoints
            .iter()
            .flatten()
            .flatten()
            .any(|m| sys.contains(&m.as_str()))
            || d.children.iter().flatten().any(hosts_system)
    }

    // USB hard drives already report TRAN=usb, and SATA/NVMe disks must never
    // be offered, so `all` has nothing extra to add here.
    pub fn list(_all: bool) -> Result<Vec<Drive>, String> {
        let json = run(
            "lsblk",
            &[
                "-J",
                "-b",
                "-o",
                // NAME keeps the output a tree: without it, newer lsblk
                // (2.42) prints partitions flat, with no `children`, which
                // hides the system mounts hosts_system() looks for.
                "NAME,PATH,SIZE,MODEL,TRAN,RM,HOTPLUG,TYPE,MOUNTPOINTS,LABEL,SERIAL",
            ],
        )?;
        let l: Lsblk = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        Ok(l.blockdevices
            .into_iter()
            .filter(|d| d.kind == "disk" && d.size.unwrap_or(0) > 0)
            .filter(|d| !d.path.starts_with("/dev/zram") && !hosts_system(d))
            .filter(|d| d.rm || d.hotplug || d.tran.as_deref() == Some("usb"))
            .map(|d| {
                let label = d
                    .label
                    .clone()
                    .or_else(|| d.children.iter().flatten().find_map(|c| c.label.clone()));
                Drive {
                    name: label
                        .or(d.model)
                        .unwrap_or_else(|| "Unknown".into())
                        .trim()
                        .to_string(),
                    size: d.size.unwrap_or(0),
                    path: d.path,
                    serial: d.serial,
                }
            })
            .collect())
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;

    // `diskutil list external physical` already includes USB hard drives and
    // never lists internal disks, so `all` has nothing extra to add here.
    pub fn list(_all: bool) -> Result<Vec<Drive>, String> {
        let out = run("diskutil", &["list", "external", "physical"])?;
        let mut drives = Vec::new();
        for line in out.lines().filter(|l| l.starts_with("/dev/disk")) {
            let path = line.split_whitespace().next().unwrap_or_default();
            let info = run("diskutil", &["info", path])?;
            let field = |key: &str| {
                info.lines()
                    .find_map(|l| l.trim().strip_prefix(key))
                    .map(|v| v.trim().to_string())
            };
            // "Disk Size: 16.0 GB (16008609792 Bytes) (exactly ...)"
            let size = field("Disk Size:")
                .and_then(|s| s.split('(').nth(1)?.split_whitespace().next()?.parse().ok())
                .unwrap_or(0);
            drives.push(Drive {
                path: path.to_string(),
                name: field("Device / Media Name:").unwrap_or_else(|| "Unknown".into()),
                size,
                serial: None,
            });
        }
        Ok(drives)
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct Disk {
        number: u32,
        friendly_name: Option<String>,
        size: u64,
        bus_type: u16,
        is_system: bool,
        is_boot: bool,
        label: Option<String>,
        letter: Option<String>,
        serial_number: Option<String>,
    }

    // BusType 7 already covers USB hard drives, and internal SATA/NVMe disks
    // must never be offered, so `all` has nothing extra to add here.
    pub fn list(_all: bool) -> Result<Vec<Drive>, String> {
        let json = run(
            "powershell",
            &[
                "-NoProfile",
                "-Command",
                "ConvertTo-Json -InputObject @(Get-Disk | Select-Object Number,FriendlyName,Size,BusType,IsSystem,IsBoot,SerialNumber,\
                 @{n='Label';e={($_ | Get-Partition -ErrorAction SilentlyContinue | Get-Volume -ErrorAction SilentlyContinue | Select-Object -First 1).FileSystemLabel}},\
                 @{n='Letter';e={[string](($_ | Get-Partition -ErrorAction SilentlyContinue | Where-Object DriveLetter | Select-Object -First 1).DriveLetter)}})",
            ],
        )?;
        let disks: Vec<Disk> = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        // BusType: 7 = USB, 12 = SD, 13 = MMC
        Ok(disks
            .into_iter()
            .filter(|d| !d.is_system && !d.is_boot && d.size > 0)
            .filter(|d| matches!(d.bus_type, 7 | 12 | 13))
            .map(|d| {
                let name = d
                    .label
                    .filter(|l| !l.is_empty())
                    .or(d.friendly_name)
                    .unwrap_or_else(|| "Unknown".into());
                let letter = d
                    .letter
                    .filter(|l| l.len() == 1)
                    .map(|l| format!(" ({l}:)"))
                    .unwrap_or_default();
                Drive {
                    path: format!(r"\\.\PhysicalDrive{}", d.number),
                    name: name + &letter,
                    size: d.size,
                    serial: d.serial_number.map(|s| s.trim().to_string()),
                }
            })
            .collect())
    }
}
