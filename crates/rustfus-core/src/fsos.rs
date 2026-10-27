// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! Filesystems Rustfus can't write itself (NTFS, exFAT, UDF, ReFS, ext*),
//! created and mounted with the host OS tools.

use crate::drives::{Drive, run};
use crate::ops::Fs;
use std::path::PathBuf;

#[cfg(not(target_os = "macos"))]
fn cluster_arg(cluster: u32) -> Option<String> {
    (cluster != 0).then(|| cluster.to_string())
}

#[cfg(target_os = "linux")]
mod imp {
    use super::*;

    pub fn partition(d: &Drive, n: u32) -> String {
        if d.path.ends_with(|c: char| c.is_ascii_digit()) {
            format!("{}p{n}", d.path)
        } else {
            format!("{}{n}", d.path)
        }
    }

    pub fn rescan(d: &Drive) -> Result<(), String> {
        run("blockdev", &["--rereadpt", &d.path]).or_else(|_| run("partprobe", &[&d.path]))?;
        run("udevadm", &["settle"]).ok();
        Ok(())
    }

    fn mkfs_tool(fs: Fs) -> Option<(&'static str, &'static str)> {
        match fs {
            Fs::Ntfs => Some(("mkfs.ntfs", "ntfs-3g or ntfsprogs")),
            Fs::Exfat => Some(("mkfs.exfat", "exfatprogs")),
            Fs::Udf => Some(("mkudffs", "udftools")),
            Fs::Ext2 => Some(("mkfs.ext2", "e2fsprogs")),
            Fs::Ext3 => Some(("mkfs.ext3", "e2fsprogs")),
            Fs::Ext4 => Some(("mkfs.ext4", "e2fsprogs")),
            _ => None,
        }
    }

    /// The tool `mkfs` would run for `fs` when it isn't installed, checked
    /// before the drive is touched.
    pub fn missing_tool(fs: Fs) -> Option<String> {
        let (tool, package) = mkfs_tool(fs)?;
        let path = std::env::var_os("PATH").unwrap_or_default();
        let found = std::env::split_paths(&path)
            .chain(["/sbin".into(), "/usr/sbin".into()])
            .any(|d| d.join(tool).is_file());
        (!found).then(|| format!("{tool} (package {package})"))
    }

    pub fn mkfs(d: &Drive, n: u32, fs: Fs, label: &str, cluster: u32) -> Result<(), String> {
        let node = partition(d, n);
        let c = cluster_arg(cluster);
        let mut args: Vec<&str> = Vec::new();
        let (tool, _) =
            mkfs_tool(fs).ok_or_else(|| format!("{} is not supported on Linux", fs.name()))?;
        match fs {
            Fs::Ntfs => {
                args.extend(["-Q", "-F", "-L", label]);
                if let Some(c) = &c {
                    args.extend(["-c", c]);
                }
            }
            Fs::Exfat => {
                args.extend(["-n", label]);
                if let Some(c) = &c {
                    args.extend(["-c", c]);
                }
            }
            Fs::Udf => args.extend(["--utf8", "--label", label]),
            _ => {
                args.extend(["-F", "-L", label]);
                if let Some(c) = &c {
                    args.extend(["-b", c]);
                }
            }
        }
        args.push(&node);
        run(tool, &args).map(|_| ())
    }

    pub fn mount(d: &Drive, n: u32, fs: Fs) -> Result<PathBuf, String> {
        let node = partition(d, n);
        let dir = std::env::temp_dir().join(format!("rustfus-mnt-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let target = dir.to_string_lossy().into_owned();
        let r = if fs == Fs::Ntfs {
            run("mount", &["-t", "ntfs3", &node, &target])
                .or_else(|_| run("ntfs-3g", &[&node, &target]))
        } else {
            // Explicit type: right after mkfs the probe can miss a filesystem
            // whose module isn't loaded yet (seen with exFAT).
            let kind = match fs {
                Fs::Exfat => "exfat",
                Fs::Udf => "udf",
                Fs::Ext2 => "ext2",
                Fs::Ext3 => "ext3",
                Fs::Ext4 => "ext4",
                _ => "auto",
            };
            run("mount", &["-t", kind, &node, &target])
        };
        r.map(|_| dir)
    }

    pub fn unmount(_d: &Drive, _n: u32, dir: &std::path::Path) -> Result<(), String> {
        run("umount", &[&dir.to_string_lossy()])?;
        std::fs::remove_dir(dir).ok();
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;

    pub fn partition(d: &Drive, n: u32) -> String {
        format!("{}s{n}", d.path)
    }

    pub fn rescan(d: &Drive) -> Result<(), String> {
        // diskutil re-reads the partition table when the whole disk is
        // unmounted after our raw writes.
        run("diskutil", &["unmountDisk", "force", &d.path]).map(|_| ())
    }

    pub fn mkfs(d: &Drive, n: u32, fs: Fs, label: &str, _cluster: u32) -> Result<(), String> {
        let raw = partition(d, n).replacen("/dev/disk", "/dev/rdisk", 1);
        match fs {
            Fs::Exfat => run("newfs_exfat", &["-v", label, &raw]),
            Fs::Udf => run("newfs_udf", &["-v", label, &raw]),
            _ => return Err(format!("{} is not supported on macOS", fs.name())),
        }
        .map(|_| ())
    }

    pub fn mount(d: &Drive, n: u32, _fs: Fs) -> Result<PathBuf, String> {
        let node = partition(d, n);
        run("diskutil", &["mount", &node])?;
        let info = run("diskutil", &["info", &node])?;
        info.lines()
            .find_map(|l| l.trim().strip_prefix("Mount Point:"))
            .map(|m| PathBuf::from(m.trim()))
            .filter(|m| !m.as_os_str().is_empty())
            .ok_or_else(|| format!("{node} did not mount"))
    }

    pub fn unmount(_d: &Drive, n: u32, _dir: &std::path::Path) -> Result<(), String> {
        run("diskutil", &["unmount", &partition(_d, n)]).map(|_| ())
    }
}

#[cfg(windows)]
mod imp {
    use super::*;

    fn disk(d: &Drive) -> &str {
        d.path.trim_start_matches(r"\\.\PhysicalDrive")
    }

    pub fn powershell(script: &str) -> Result<String, String> {
        run(
            "powershell",
            &[
                "-NoProfile",
                "-Command",
                &format!("$ErrorActionPreference='Stop'; {script}"),
            ],
        )
    }

    pub fn rescan(d: &Drive) -> Result<(), String> {
        powershell(&format!("Update-Disk -Number {}", disk(d))).map(|_| ())
    }

    pub fn mkfs(d: &Drive, n: u32, fs: Fs, label: &str, cluster: u32) -> Result<(), String> {
        let name = match fs {
            Fs::Ntfs => "NTFS",
            Fs::Exfat => "exFAT",
            Fs::Udf => "UDF",
            Fs::Refs => "ReFS",
            _ => return Err(format!("{} is not supported on Windows", fs.name())),
        };
        let unit = cluster_arg(cluster)
            .map(|c| format!(" -AllocationUnitSize {c}"))
            .unwrap_or_default();
        powershell(&format!(
            "Get-Partition -DiskNumber {} -PartitionNumber {n} | Format-Volume -FileSystem {name} \
             -NewFileSystemLabel '{label}'{unit} -Confirm:$false",
            disk(d)
        ))
        .map(|_| ())
    }

    pub fn mount(d: &Drive, n: u32, _fs: Fs) -> Result<PathBuf, String> {
        let get = format!("Get-Partition -DiskNumber {} -PartitionNumber {n}", disk(d));
        let letter = powershell(&format!(
            "$p = {get}; if (-not $p.DriveLetter -or $p.DriveLetter -eq [char]0) {{ $p | Add-PartitionAccessPath -AssignDriveLetter }}; ({get}).DriveLetter"
        ))?;
        let letter = letter.trim();
        if letter.len() != 1 {
            return Err("No drive letter could be assigned".into());
        }
        Ok(PathBuf::from(format!("{letter}:\\")))
    }

    pub fn unmount(_d: &Drive, _n: u32, _dir: &std::path::Path) -> Result<(), String> {
        Ok(())
    }
}

/// macOS and Windows format with built-in system tools.
#[cfg(not(target_os = "linux"))]
pub fn missing_tool(_fs: Fs) -> Option<String> {
    None
}

pub use imp::*;
