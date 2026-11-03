// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
//! OS-managed Windows UEFI media. This path never opens a raw disk or elevates.

use super::files::{Dest, MountedDir, copy_files_replacing};
use super::{Boot, Ctx, Fs, Job, MB, Msg, Report, Scheme, Target};
use crate::{
    drives::{self, Drive},
    iso::{Entry, Iso},
};
use plist::Value;
use std::{
    collections::HashSet,
    ffi::{CStr, CString},
    fs::{self, File},
    io::{Cursor, Read, Seek, SeekFrom, Write},
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{atomic::AtomicBool, mpsc::Sender},
    time::Duration,
};

const FAT_MAX_FILE: u64 = (4 << 30) - 1;
const DISKUTIL: &str = "/usr/sbin/diskutil";
type SplitFiles = Vec<(String, PathBuf)>;

/// Routing only: unsupported options on this path fail preflight, never fall
/// through to the raw writer after partial formatting.
pub fn supports(job: &Job, report: &Report) -> bool {
    matches!(job.boot, Boot::Image(_))
        && !job.dd
        && report.is_iso
        && report.windows
        && report.efi
        && job.scheme == Scheme::Gpt
        && job.target == Target::Uefi
        && job.fs == Fs::Fat32
        && job.wtg.is_none()
}

fn whole_disk(path: &str) -> Result<&str, String> {
    let name = path
        .strip_prefix("/dev/")
        .ok_or("Invalid whole-disk path")?;
    let digits = name.strip_prefix("disk").ok_or("Invalid whole-disk path")?;
    if digits.is_empty()
        || !digits.bytes().all(|b| b.is_ascii_digit())
        || digits.parse::<u32>().ok() == Some(0)
    {
        return Err("Select an external physical whole disk".into());
    }
    Ok(name)
}

fn safe_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(['\\', '\0'])
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != ".." && !part.contains(':'))
}

fn preflight<'a>(
    job: &Job,
    report: &Report,
    entries: impl IntoIterator<Item = (&'a str, bool, u64, bool)>,
) -> Result<Option<&'a str>, String> {
    if !supports(job, report) {
        return Err(
            "The native macOS path requires Windows UEFI/GPT/FAT32 installation media".into(),
        );
    }
    whole_disk(&job.drive.path)?;
    if !job.quick
        || job.bad_blocks.is_some()
        || job.persistence != 0
        || job.old_bios
        || job.cluster != 0
    {
        return Err("macOS native Windows media requires Quick format, default cluster size, and no bad-block, persistence, or legacy BIOS options. No disk was erased.".into());
    }
    let mut paths = HashSet::new();
    let mut large_wim = None;
    let mut total = 0u64;
    let mut boot_loader = false;
    for (path, is_dir, size, symlink) in entries {
        if !safe_path(path) || symlink {
            return Err(format!("Unsafe or unsupported ISO path: {path}"));
        }
        let lower = path.to_ascii_lowercase();
        if !paths.insert(lower.clone()) {
            return Err(format!("Duplicate case-insensitive ISO path: {}", path));
        }
        if is_dir {
            continue;
        }
        total = total.checked_add(size).ok_or("Image file sizes overflow")?;
        boot_loader |= lower.starts_with("efi/boot/boot") && lower.ends_with(".efi");
        if size > FAT_MAX_FILE {
            if lower != "sources/install.wim" {
                return Err(format!(
                    "{} exceeds FAT32's 4 GB file limit. Only sources/install.wim can be split; no disk was erased.",
                    path
                ));
            }
            large_wim = Some(path);
        }
    }
    if !boot_loader {
        return Err("The ISO has no Windows UEFI removable-media boot loader".into());
    }
    // Reserve room for GPT/EFI partitions, filesystem metadata and setup extras.
    if total
        .checked_add(256 * MB)
        .is_none_or(|needed| needed > job.drive.size)
    {
        return Err(
            "The Windows image is larger than the selected USB drive; no disk was erased".into(),
        );
    }
    Ok(large_wim)
}

trait DiskAccess {
    fn verify(&self, drive: &Drive) -> Result<(), String>;
    fn erase(&self, drive: &Drive, label: &str) -> Result<(), String>;
    fn mount(&self, drive: &Drive, label: &str) -> Result<MountedVolume, String>;
    fn unmount(&self, partition: &str) -> Result<(), String>;
}

struct SystemDisk;
#[derive(Debug)]
struct MountedVolume {
    partition: String,
    root: PathBuf,
}

fn diskutil(args: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new(DISKUTIL)
        .args(args)
        .output()
        .map_err(|e| format!("diskutil: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "diskutil {} failed: {} {}. macOS may require permission to manage removable storage; no raw-write fallback was attempted.",
            args.first().copied().unwrap_or(""),
            String::from_utf8_lossy(&output.stderr).trim(),
            String::from_utf8_lossy(&output.stdout).trim()
        ));
    }
    Ok(output.stdout)
}

fn plist_value(bytes: &[u8]) -> Result<Value, String> {
    Value::from_reader(Cursor::new(bytes)).map_err(|e| format!("Invalid diskutil response: {e}"))
}
fn field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.as_dictionary()?.get(key)
}
fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    field(value, key)?.as_string()
}

fn fat_partition(list: &Value, drive: &Drive, label: &str) -> Result<String, String> {
    let name = whole_disk(&drive.path)?;
    let disks = field(list, "AllDisksAndPartitions")
        .and_then(Value::as_array)
        .ok_or("diskutil did not return disk partitions")?;
    let disk = disks
        .iter()
        .find(|disk| text(disk, "DeviceIdentifier") == Some(name))
        .ok_or("The selected USB disk disappeared after formatting")?;
    if text(disk, "Content") != Some("GUID_partition_scheme") {
        return Err("diskutil did not create the requested GPT partition map".into());
    }
    let partitions = field(disk, "Partitions")
        .and_then(Value::as_array)
        .ok_or("No partitions were created")?;
    let candidates: Vec<_> = partitions
        .iter()
        .filter(|part| {
            text(part, "VolumeName") == Some(label)
                && text(part, "Content") == Some("Microsoft Basic Data")
        })
        .collect();
    if candidates.len() != 1 {
        return Err("Could not identify exactly one formatted USB data partition".into());
    }
    let partition =
        text(candidates[0], "DeviceIdentifier").ok_or("USB partition has no identifier")?;
    let suffix = partition
        .strip_prefix(&format!("{name}s"))
        .ok_or("Partition does not belong to the selected USB disk")?;
    if suffix.is_empty() || !suffix.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Invalid USB partition identifier".into());
    }
    Ok(format!("/dev/{partition}"))
}

fn volume_info(
    info: &Value,
    drive: &Drive,
    partition: &str,
    label: &str,
) -> Result<PathBuf, String> {
    let whole = whole_disk(&drive.path)?;
    let fat32 = text(info, "FilesystemType") == Some("msdos")
        || text(info, "Type") == Some("msdos")
        || text(info, "FilesystemName") == Some("MS-DOS FAT32")
        || text(info, "FileSystemPersonality") == Some("MS-DOS FAT32");
    if text(info, "DeviceIdentifier") != partition.strip_prefix("/dev/")
        || text(info, "ParentWholeDisk").or_else(|| text(info, "PartOfWhole")) != Some(whole)
        || !fat32
        || text(info, "VolumeName") != Some(label)
        || field(info, "Mounted").and_then(Value::as_boolean) != Some(true)
    {
        return Err("Mounted volume does not match the selected FAT32 USB partition".into());
    }
    let root = PathBuf::from(text(info, "MountPoint").ok_or("USB volume has no mount point")?);
    if !root.is_absolute() || !root.starts_with("/Volumes") || root == Path::new("/Volumes") {
        return Err("Unexpected USB mount point".into());
    }
    Ok(root)
}

impl DiskAccess for SystemDisk {
    fn verify(&self, drive: &Drive) -> Result<(), String> {
        drives::verify(drive)
    }
    fn erase(&self, drive: &Drive, label: &str) -> Result<(), String> {
        diskutil(&["eraseDisk", "FAT32", label, "GPT", &drive.path]).map(|_| ())
    }
    fn mount(&self, drive: &Drive, label: &str) -> Result<MountedVolume, String> {
        self.verify(drive)?;
        let partition = fat_partition(
            &plist_value(&diskutil(&["list", "-plist", &drive.path])?)?,
            drive,
            label,
        )?;
        diskutil(&["mount", &partition])?;
        let root = volume_info(
            &plist_value(&diskutil(&["info", "-plist", &partition])?)?,
            drive,
            &partition,
            label,
        )?;
        Ok(MountedVolume { partition, root })
    }
    fn unmount(&self, partition: &str) -> Result<(), String> {
        diskutil(&["unmount", partition]).map(|_| ())
    }
}

fn format_volume(
    disk: &impl DiskAccess,
    job: &Job,
    label: &str,
    ctx: &Ctx,
) -> Result<MountedVolume, String> {
    ctx.check()?;
    disk.verify(&job.drive)?;
    ctx.check()?;
    ctx.progress(0., "Formatting USB with macOS diskutil…");
    ctx.log("Using macOS storage management; Rustfus will not open /dev/rdisk or request root for this job");
    // Do not kill diskutil mid-partitioning. Cancellation is honored before
    // erase, and again when that critical OS operation has completed.
    disk.erase(&job.drive, label)?;
    let volume = disk.mount(&job.drive, label)?;
    if let Err(error) = ctx.check() {
        let _ = disk.unmount(&volume.partition);
        return Err(error);
    }
    Ok(volume)
}

#[derive(Clone, PartialEq, Eq)]
struct MountIdentity {
    source: Vec<u8>,
    kind: Vec<u8>,
    device: u64,
}
fn mount_identity(root: &Path) -> Result<MountIdentity, String> {
    let root_c = CString::new(root.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    let mut status = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: statfs initializes status on success; root_c is a NUL-terminated path.
    if unsafe { libc::statfs(root_c.as_ptr(), status.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    // SAFETY: statfs succeeded and its fixed string fields are NUL-terminated by macOS.
    let status = unsafe { status.assume_init() };
    let source = unsafe { CStr::from_ptr(status.f_mntfromname.as_ptr()) }
        .to_bytes()
        .to_vec();
    let kind = unsafe { CStr::from_ptr(status.f_fstypename.as_ptr()) }
        .to_bytes()
        .to_vec();
    Ok(MountIdentity {
        source,
        kind,
        device: fs::symlink_metadata(root).map_err(|e| e.to_string())?.dev(),
    })
}

fn copy_iso_entry(iso: &Iso, entry: &Entry, destination: &Path, ctx: &Ctx) -> Result<(), String> {
    let mut reader = iso.reader(entry).map_err(super::e)?;
    let mut output = File::create(destination).map_err(super::e)?;
    let mut buffer = vec![0; 4 << 20];
    let mut done = 0u64;
    loop {
        ctx.check()?;
        let read = reader.read(&mut buffer).map_err(super::e)?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read]).map_err(super::e)?;
        done += read as u64;
        ctx.progress(
            done as f32 / entry.size.max(1) as f32,
            "Staging install.wim before erasing USB…",
        );
    }
    if done != entry.size {
        return Err("Unexpected end of install.wim; no disk was erased".into());
    }
    output.sync_all().map_err(super::e)
}

fn wimlib() -> Result<PathBuf, String> {
    let search = std::env::var_os("PATH").unwrap_or_default();
    for path in std::env::split_paths(&search).chain([
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ]) {
        let candidate = path.join("wimlib-imagex");
        if candidate.is_file()
            && Command::new(&candidate)
                .arg("--version")
                .output()
                .is_ok_and(|out| out.status.success())
        {
            return candidate.canonicalize().map_err(super::e);
        }
    }
    Err("Windows install.wim exceeds FAT32's 4 GB limit. Install wimlib (brew install wimlib), then retry. No disk was erased.".into())
}

struct StagedWim {
    _directory: tempfile::TempDir,
    original: String,
    files: SplitFiles,
}
fn stage_wim(iso: &Iso, entry: &Entry, ctx: &Ctx) -> Result<StagedWim, String> {
    let tool = wimlib()?;
    let directory = tempfile::Builder::new()
        .prefix("rustfus-wim-")
        .tempdir()
        .map_err(super::e)?;
    let source = directory.path().join("input.wim");
    copy_iso_entry(iso, entry, &source, ctx)?;
    let output = directory.path().join("install.swm");
    let mut diagnostics = tempfile::tempfile().map_err(super::e)?;
    let stdout = diagnostics.try_clone().map_err(super::e)?;
    let stderr = diagnostics.try_clone().map_err(super::e)?;
    ctx.progress(0., "Splitting install.wim for FAT32 before erasing USB…");
    let mut child = Command::new(tool)
        .arg("split")
        .arg(&source)
        .arg(&output)
        .arg("3800")
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr)
        .spawn()
        .map_err(super::e)?;
    let status = loop {
        if let Err(error) = ctx.check() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.to_string());
            }
        }
    };
    if !status.success() {
        diagnostics.seek(SeekFrom::Start(0)).map_err(super::e)?;
        let mut message = String::new();
        diagnostics
            .take(16384)
            .read_to_string(&mut message)
            .map_err(super::e)?;
        return Err(format!(
            "Could not split install.wim; no disk was erased. {}",
            message.trim()
        ));
    }
    let parent = entry
        .path
        .rsplit_once('/')
        .ok_or("Invalid install.wim path")?
        .0;
    let mut files = Vec::new();
    for item in fs::read_dir(directory.path()).map_err(super::e)? {
        let path = item.map_err(super::e)?.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !name.ends_with(".swm") {
            continue;
        }
        let metadata = fs::symlink_metadata(&path).map_err(super::e)?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > FAT_MAX_FILE {
            return Err(
                "A split WIM part exceeds FAT32 limits or is invalid; no disk was erased".into(),
            );
        }
        let mut header = [0; 8];
        File::open(&path)
            .and_then(|mut file| file.read_exact(&mut header))
            .map_err(super::e)?;
        if &header != b"MSWIM\0\0\0" {
            return Err("Invalid split WIM output; no disk was erased".into());
        }
        files.push((format!("{parent}/{name}"), path));
    }
    if !files
        .iter()
        .any(|(_, file)| file.file_name().is_some_and(|name| name == "install.swm"))
    {
        return Err("wimlib produced no install.swm; no disk was erased".into());
    }
    files.sort();
    Ok(StagedWim {
        _directory: directory,
        original: entry.path.clone(),
        files,
    })
}

fn validate_split_paths<'a>(
    existing: impl IntoIterator<Item = &'a str>,
    files: &SplitFiles,
) -> Result<(), String> {
    let existing: HashSet<_> = existing.into_iter().map(str::to_ascii_lowercase).collect();
    let mut seen = HashSet::new();
    for (path, _) in files {
        let lower = path.to_ascii_lowercase();
        if !safe_path(path) || existing.contains(&lower) || !seen.insert(lower) {
            return Err(format!(
                "Split WIM output conflicts with ISO content: {path}. No disk was erased."
            ));
        }
    }
    Ok(())
}

pub(super) fn run(job: &Job, tx: &Sender<Msg>, cancel: &AtomicBool) -> Result<String, String> {
    let ctx = Ctx { tx, cancel };
    ctx.check()?;
    let Boot::Image(image) = &job.boot else {
        return Err("Select a Windows installation ISO".into());
    };
    let iso = Iso::open(image).map_err(|e| {
        format!(
            "Cannot read Windows ISO {}: {e}; no disk was erased",
            image.display()
        )
    })?;
    let report = super::analyze(image)?;
    let entries = iso
        .entries()
        .map_err(|e| format!("Cannot list Windows ISO files: {e}; no disk was erased"))?;
    let large_wim = preflight(
        job,
        &report,
        entries.iter().map(|entry| {
            (
                entry.path.as_str(),
                entry.is_dir,
                entry.size,
                entry.symlink().is_some(),
            )
        }),
    )?;
    let staged = large_wim
        .map(|path| {
            stage_wim(
                &iso,
                entries
                    .iter()
                    .find(|entry| entry.path == path)
                    .expect("preflight path comes from entries"),
                &ctx,
            )
        })
        .transpose()?;
    if let Some(staged) = &staged {
        validate_split_paths(
            entries.iter().map(|entry| entry.path.as_str()),
            &staged.files,
        )?;
        let staged_bytes = staged.files.iter().try_fold(0u64, |total, (_, path)| {
            total
                .checked_add(fs::metadata(path).map_err(super::e)?.len())
                .ok_or_else(|| "Split WIM sizes overflow".to_string())
        })?;
        let other_bytes = entries
            .iter()
            .filter(|entry| !entry.is_dir && !entry.path.eq_ignore_ascii_case(&staged.original))
            .map(|entry| entry.size)
            .sum::<u64>();
        if other_bytes
            .checked_add(staged_bytes)
            .and_then(|size| size.checked_add(256 * MB))
            .is_none_or(|size| size > job.drive.size)
        {
            return Err(
                "The split Windows image does not fit on the selected USB; no disk was erased"
                    .into(),
            );
        }
    }
    let label = super::sanitize_label(&job.label, Fs::Fat32);
    let disk = SystemDisk;
    let volume = format_volume(&disk, job, &label, &ctx)?;
    let result = (|| {
        disk.verify(&job.drive)?;
        let root = volume.root.canonicalize().map_err(super::e)?;
        if root != volume.root {
            return Err("USB mount point resolves through a symlink".into());
        }
        let identity = mount_identity(&root)?;
        if identity.source != volume.partition.as_bytes() || identity.kind != b"msdos" {
            return Err("USB mount source or filesystem changed before copying".into());
        }
        let guard = || {
            ctx.check()?;
            if mount_identity(&root)? != identity {
                return Err("The USB was removed or its mount changed; copying stopped".into());
            }
            Ok(())
        };
        let mounted = MountedDir::open(root.clone(), &guard)
            .map_err(|e| format!("Cannot open mounted USB directory {}: {e}", root.display()))?;
        let destination: Dest<'_, Cursor<Vec<u8>>> = Dest::GuardedDir(&mounted);
        copy_files_replacing(
            job,
            &report,
            Some(&iso),
            &destination,
            staged
                .as_ref()
                .map(|staged| (staged.original.as_str(), staged.files.as_slice())),
            &ctx,
        )
    })();
    // Flush mounted writes via diskutil and report completion only after unmount.
    // Failure cleanup never falls through to raw writes or retries formatting.
    let unmount_result = disk.unmount(&volume.partition);
    result?;
    unmount_result?;
    ctx.progress(1., "Windows UEFI USB is ready");
    Ok(format!(
        "{} ready (macOS-managed GPT/FAT32 \"{label}\")",
        job.drive.path
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, sync::atomic::Ordering};

    fn job() -> Job {
        Job {
            drive: Drive {
                path: "/dev/disk12".into(),
                name: "USB".into(),
                size: 16 << 30,
                serial: None,
            },
            boot: Boot::Image("/tmp/windows.iso".into()),
            dd: false,
            scheme: Scheme::Gpt,
            target: Target::Uefi,
            fs: Fs::Fat32,
            cluster: 0,
            label: "WIN".into(),
            quick: true,
            ext_label: true,
            bad_blocks: None,
            verify: false,
            old_bios: false,
            uefi_validation: false,
            persistence: 0,
            wue: Default::default(),
            allow_downloads: false,
            replace_c32: vec![],
            wtg: None,
        }
    }
    fn report() -> Report {
        Report {
            is_iso: true,
            windows: true,
            efi: true,
            ..Default::default()
        }
    }
    struct FakeDisk<'a> {
        calls: RefCell<Vec<&'static str>>,
        valid: bool,
        cancel: &'a AtomicBool,
        cancel_on_erase: bool,
    }
    impl DiskAccess for FakeDisk<'_> {
        fn verify(&self, _: &Drive) -> Result<(), String> {
            self.calls.borrow_mut().push("verify");
            if self.valid {
                Ok(())
            } else {
                Err("drive changed".into())
            }
        }
        fn erase(&self, _: &Drive, _: &str) -> Result<(), String> {
            self.calls.borrow_mut().push("erase");
            if self.cancel_on_erase {
                self.cancel.store(true, Ordering::Relaxed);
            }
            Ok(())
        }
        fn mount(&self, _: &Drive, _: &str) -> Result<MountedVolume, String> {
            self.calls.borrow_mut().push("mount");
            Ok(MountedVolume {
                partition: "/dev/disk12s2".into(),
                root: "/Volumes/WIN".into(),
            })
        }
        fn unmount(&self, _: &str) -> Result<(), String> {
            self.calls.borrow_mut().push("unmount");
            Ok(())
        }
    }
    #[test]
    fn preflight_rejects_raw_only_options_large_non_wim_and_unsafe_paths() {
        let entries = [
            ("efi/boot/bootx64.efi", false, 100, false),
            ("sources/install.wim", false, FAT_MAX_FILE + 1, false),
        ];
        assert_eq!(
            preflight(&job(), &report(), entries).unwrap(),
            Some("sources/install.wim")
        );
        let mut invalid = job();
        invalid.cluster = 4096;
        assert!(preflight(&invalid, &report(), entries).is_err());
        invalid = job();
        invalid.bad_blocks = Some(1);
        assert!(preflight(&invalid, &report(), entries).is_err());
        invalid = job();
        invalid.quick = false;
        assert!(preflight(&invalid, &report(), entries).is_err());
        assert!(
            preflight(
                &job(),
                &report(),
                [("sources/install.esd", false, FAT_MAX_FILE + 1, false)]
            )
            .is_err()
        );
        assert!(preflight(&job(), &report(), [("../escape", false, 1, false)]).is_err());
        assert!(
            preflight(
                &job(),
                &report(),
                [("efi/boot/bootx64.efi", false, 1, true)]
            )
            .is_err()
        );
        assert!(
            preflight(
                &job(),
                &report(),
                [("sources/install.wim", false, 1, false)]
            )
            .is_err()
        );
        invalid = job();
        invalid.drive.size = 100;
        assert!(preflight(&invalid, &report(), entries).is_err());
    }

    #[test]
    fn split_paths_cannot_replace_existing_iso_files_or_each_other() {
        let files = vec![
            ("sources/install.swm".into(), PathBuf::from("part1")),
            ("sources/install2.swm".into(), PathBuf::from("part2")),
        ];
        assert!(validate_split_paths(["sources/install.wim"], &files).is_ok());
        assert!(validate_split_paths(["SOURCES/INSTALL2.SWM"], &files).is_err());
        let duplicate = vec![files[0].clone(), files[0].clone()];
        assert!(validate_split_paths([], &duplicate).is_err());
    }

    #[test]
    fn routing_excludes_non_windows_and_raw_jobs() {
        let job = job();
        assert!(supports(&job, &report()));
        assert!(!supports(&job, &Report::default()));
        let mut raw = job.clone();
        raw.dd = true;
        assert!(!supports(&raw, &report()));
        let mut bios = job;
        bios.target = Target::Bios;
        assert!(!supports(&bios, &report()));
    }
    #[test]
    fn verifies_before_erase_and_finishes_critical_format_before_cancel() {
        let cancel = AtomicBool::new(false);
        let (tx, _) = std::sync::mpsc::channel();
        let ctx = Ctx {
            tx: &tx,
            cancel: &cancel,
        };
        let disk = FakeDisk {
            calls: RefCell::new(vec![]),
            valid: true,
            cancel: &cancel,
            cancel_on_erase: true,
        };
        assert_eq!(
            format_volume(&disk, &job(), "WIN", &ctx).unwrap_err(),
            "Cancelled"
        );
        assert_eq!(
            *disk.calls.borrow(),
            ["verify", "erase", "mount", "unmount"]
        );
    }
    #[test]
    fn denial_and_pre_cancel_never_erase() {
        let cancel = AtomicBool::new(false);
        let (tx, _) = std::sync::mpsc::channel();
        let ctx = Ctx {
            tx: &tx,
            cancel: &cancel,
        };
        let disk = FakeDisk {
            calls: RefCell::new(vec![]),
            valid: false,
            cancel: &cancel,
            cancel_on_erase: false,
        };
        assert!(format_volume(&disk, &job(), "WIN", &ctx).is_err());
        assert_eq!(*disk.calls.borrow(), ["verify"]);
        cancel.store(true, Ordering::Relaxed);
        disk.calls.borrow_mut().clear();
        assert!(format_volume(&disk, &job(), "WIN", &ctx).is_err());
        assert!(disk.calls.borrow().is_empty());
    }
    fn plist(xml: &str) -> Value {
        plist_value(
            format!("<?xml version=\"1.0\"?><plist version=\"1.0\">{xml}</plist>").as_bytes(),
        )
        .unwrap()
    }
    #[test]
    fn gpt_data_partition_is_discovered_not_assumed_to_be_s1() {
        let list = plist(
            "<dict><key>AllDisksAndPartitions</key><array><dict><key>DeviceIdentifier</key><string>disk12</string><key>Content</key><string>GUID_partition_scheme</string><key>Partitions</key><array><dict><key>DeviceIdentifier</key><string>disk12s1</string><key>Content</key><string>EFI</string></dict><dict><key>DeviceIdentifier</key><string>disk12s2</string><key>Content</key><string>Microsoft Basic Data</string><key>VolumeName</key><string>WIN</string></dict></array></dict></array></dict>",
        );
        assert_eq!(
            fat_partition(&list, &job().drive, "WIN").unwrap(),
            "/dev/disk12s2"
        );
        assert!(fat_partition(&list, &job().drive, "OTHER").is_err());
        let mut changed = job().drive;
        changed.path = "/dev/disk13".into();
        assert!(fat_partition(&list, &changed, "WIN").is_err());
    }
    #[test]
    fn mounted_volume_must_match_disk_partition_and_fat_filesystem() {
        let xml = "<dict><key>DeviceIdentifier</key><string>disk12s2</string><key>ParentWholeDisk</key><string>disk12</string><key>FilesystemType</key><string>msdos</string><key>Mounted</key><true/><key>VolumeName</key><string>WIN</string><key>MountPoint</key><string>/Volumes/WIN</string></dict>";
        assert_eq!(
            volume_info(&plist(xml), &job().drive, "/dev/disk12s2", "WIN").unwrap(),
            Path::new("/Volumes/WIN")
        );
        assert!(
            volume_info(
                &plist(&xml.replace("msdos", "apfs")),
                &job().drive,
                "/dev/disk12s2",
                "WIN"
            )
            .is_err()
        );
        assert!(
            volume_info(
                &plist(&xml.replace("/Volumes/WIN", "/tmp/WIN")),
                &job().drive,
                "/dev/disk12s2",
                "WIN"
            )
            .is_err()
        );
    }
    #[test]
    fn disk_paths_are_whole_and_external_candidates_only() {
        assert!(whole_disk("/dev/disk12").is_ok());
        for path in [
            "/dev/disk0",
            "/dev/disk00",
            "/dev/disk12s2",
            "/dev/rdisk12",
            "disk12",
            "/dev/disk12;rm",
        ] {
            assert!(whole_disk(path).is_err());
        }
    }
}
