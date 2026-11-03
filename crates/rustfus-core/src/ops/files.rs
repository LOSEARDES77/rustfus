// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! Copying the image tree and Rustfus' extras onto the new filesystem.

use super::{AUTORUN_ICO, Boot, Ctx, FREEDOS, Job, MB, MD5_LOADERS, Report, e, sanitize_label};
use crate::{image, iso, syslinux, wue};
use md5::{Digest, Md5};
use std::fs::File;
use std::io::{self, Cursor, Read, Write};
use std::path::PathBuf;
#[cfg(unix)]
use std::path::{Component, Path};

#[cfg(unix)]
use std::os::unix::{
    fs::OpenOptionsExt,
    io::{AsRawFd, FromRawFd},
};

/// Where files land: Rustfus' own FAT driver, or a mounted OS filesystem.
pub(super) enum Dest<'a, T: fatfs::ReadWriteSeek> {
    Fat(&'a fatfs::FileSystem<T>),
    Dir(PathBuf),
    #[cfg(unix)]
    GuardedDir(&'a MountedDir<'a>),
}

/// `Dest::Dir` never touches the FAT type parameter.
pub(super) type DirDest = Dest<'static, Cursor<Vec<u8>>>;

/// A mounted directory whose root is held open while a copy is in progress.
/// The callback is checked before every filesystem operation.  On Unix, child
/// paths are opened relative to the retained root descriptor and symlinks are
/// rejected, avoiding path races through the mount tree.
#[cfg(unix)]
pub(super) struct MountedDir<'a> {
    guard: &'a dyn Fn() -> Result<(), String>,
    #[cfg(unix)]
    fd: File,
}

#[cfg(unix)]
impl<'a> MountedDir<'a> {
    pub(super) fn open(
        root: PathBuf,
        guard: &'a dyn Fn() -> Result<(), String>,
    ) -> io::Result<Self> {
        guard().map_err(io::Error::other)?;
        #[cfg(unix)]
        {
            let fd = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW)
                .open(&root)?;
            guard().map_err(io::Error::other)?;
            Ok(Self { guard, fd })
        }

        #[cfg(not(unix))]
        {
            let _ = root;
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "guarded mounted directories require Unix",
            ))
        }
    }

    fn guarded(&self) -> io::Result<()> {
        (self.guard)().map_err(io::Error::other)
    }

    fn check(&self, path: &str) -> io::Result<Vec<String>> {
        self.guarded()?;
        let p = Path::new(path);
        if path.is_empty()
            || p.is_absolute()
            || path.contains(['\\', '\0'])
            || path
                .split('/')
                .any(|s| s.is_empty() || s == "." || s == ".." || s.contains(':'))
            || p.components().any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsafe destination path",
            ));
        }
        Ok(path.split('/').map(str::to_owned).collect())
    }

    #[cfg(unix)]
    fn directory(&self, parts: &[String], create: bool) -> io::Result<File> {
        let mut dir = self.fd.try_clone()?;
        for part in parts {
            let name = std::ffi::CString::new(part.as_str()).unwrap();
            self.guarded()?;
            let mut next = unsafe {
                libc::openat(
                    dir.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                )
            };
            if next < 0 && create && io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT)
            {
                self.guarded()?;
                if unsafe { libc::mkdirat(dir.as_raw_fd(), name.as_ptr(), 0o755) } < 0
                    && io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST)
                {
                    return Err(io::Error::last_os_error());
                }
                self.guarded()?;
                next = unsafe {
                    libc::openat(
                        dir.as_raw_fd(),
                        name.as_ptr(),
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                    )
                };
            }
            if next < 0 {
                return Err(io::Error::last_os_error());
            }
            dir = unsafe { File::from_raw_fd(next) };
        }
        Ok(dir)
    }

    fn mkdir(&self, path: &str) -> io::Result<()> {
        let parts = self.check(path)?;
        #[cfg(unix)]
        {
            self.directory(&parts, true)?;
            self.guarded()
        }
        #[cfg(not(unix))]
        {
            let _ = parts;
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "guarded mounted directories require Unix",
            ))
        }
    }

    fn create(&self, path: &str) -> io::Result<File> {
        let parts = self.check(path)?;
        #[cfg(unix)]
        {
            let dir = self.directory(&parts[..parts.len() - 1], false)?;
            let name = std::ffi::CString::new(parts.last().unwrap().as_str()).unwrap();
            self.guarded()?;
            // Open without truncating first: an existing hard link or special
            // file must be rejected before it can alter anything outside the tree.
            let fd = unsafe {
                libc::openat(
                    dir.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_WRONLY
                        | libc::O_CREAT
                        | libc::O_NONBLOCK
                        | libc::O_CLOEXEC
                        | libc::O_NOFOLLOW,
                    0o644,
                )
            };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            let file = unsafe { File::from_raw_fd(fd) };
            use std::os::unix::fs::MetadataExt;
            let metadata = file.metadata()?;
            if !metadata.is_file() || metadata.nlink() != 1 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "destination is not a regular single-link file",
                ));
            }
            self.guarded()?;
            file.set_len(0)?;
            Ok(file)
        }
        #[cfg(not(unix))]
        {
            let _ = parts;
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "guarded mounted directories require Unix",
            ))
        }
    }
}

#[cfg(unix)]
struct GuardedWriter<'a> {
    file: File,
    guard: &'a dyn Fn() -> Result<(), String>,
}

#[cfg(unix)]
impl Write for GuardedWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        (self.guard)().map_err(io::Error::other)?;
        self.file.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        (self.guard)().map_err(io::Error::other)?;
        self.file.flush()?;
        (self.guard)().map_err(io::Error::other)?;
        self.file.sync_all()
    }
}

impl<T: fatfs::ReadWriteSeek> Dest<'_, T> {
    pub(super) fn mkdir(&self, p: &str) -> io::Result<()> {
        match self {
            Dest::Fat(fs) => fs.root_dir().create_dir(p).map(|_| ()),
            Dest::Dir(root) => std::fs::create_dir_all(root.join(p)),
            #[cfg(unix)]
            Dest::GuardedDir(dir) => dir.mkdir(p),
        }
    }

    pub(super) fn create(&self, p: &str) -> io::Result<Box<dyn Write + '_>> {
        match self {
            Dest::Fat(fs) => {
                let mut f = fs.root_dir().create_file(p)?;
                f.truncate()?;
                Ok(Box::new(f))
            }
            Dest::Dir(root) => Ok(Box::new(File::create(root.join(p))?)),
            #[cfg(unix)]
            Dest::GuardedDir(dir) => Ok(Box::new(GuardedWriter {
                file: dir.create(p)?,
                guard: dir.guard,
            })),
        }
    }

    pub(super) fn put(&self, p: &str, data: &[u8]) -> io::Result<()> {
        let mut out = self
            .create(p)
            .map_err(|e| io::Error::new(e.kind(), format!("Creating {p}: {e}")))?;
        out.write_all(data)
            .map_err(|e| io::Error::new(e.kind(), format!("Writing {p}: {e}")))?;
        out.flush()
            .map_err(|e| io::Error::new(e.kind(), format!("Flushing {p}: {e}")))
    }
}

pub(super) fn is_boot_cfg(p: &str) -> bool {
    let p = p.to_ascii_lowercase();
    p.ends_with(".cfg")
        && (p.starts_with("boot/grub") || p.starts_with("isolinux") || p.starts_with("syslinux"))
}

/// Adds the persistence kernel option with Rufus' rules, first match wins:
/// older Ubuntu's preseed, Mint's `boot=casper`, Ubuntu 23.04+'s bare
/// `/casper/vmlinuz`, then Debian's `boot=live`. None when nothing changed.
pub(super) fn patch_persistence(cfg: &str) -> Option<String> {
    const RULES: [(&str, &str); 4] = [
        ("file=/cdrom/preseed", "persistent file=/cdrom/preseed"),
        ("boot=casper", "boot=casper persistent"),
        ("/casper/vmlinuz ", "/casper/vmlinuz persistent "),
        ("boot=live", "boot=live persistence"),
    ];
    let (from, to) = RULES.iter().find(|(from, _)| cfg.contains(from))?;
    // Undo an earlier patch first so a second run changes nothing.
    let s = cfg.replace(to, from).replace(from, to);
    (s != cfg).then_some(s)
}

/// Replaces the ISO label with the USB one on kernel command lines, as Rufus'
/// fix_config does, since the new filesystem can't keep the ISO label
/// (`root=live:CDLABEL=`, `coreos.liveiso=`, ...). `path` picks the keyword:
/// `linux` in grub.cfg, `options` in .conf (systemd-boot), `append` otherwise.
pub(super) fn patch_label(
    path: &str,
    cfg: &str,
    iso_label: &str,
    usb_label: &str,
) -> Option<String> {
    let lower = path.to_ascii_lowercase();
    let keys: &[&str] = if lower.ends_with("grub.cfg") {
        &["linux", "linuxefi", "$linux"]
    } else if lower.ends_with(".conf") {
        &["options"]
    } else if lower.ends_with(".cfg") {
        &["append"]
    } else {
        return None;
    };
    let (from, to) = (
        iso_label.replace(' ', "\\x20"),
        usb_label.replace(' ', "\\x20"),
    );
    if from.is_empty() || from == to || !cfg.contains(&from) {
        return None;
    }
    let mut changed = false;
    let out: Vec<String> = cfg
        .split_inclusive('\n')
        .map(|line| {
            let first = line.split_whitespace().next().unwrap_or("");
            if keys.iter().any(|k| first.eq_ignore_ascii_case(k)) && line.contains(&from) {
                changed = true;
                line.replace(&from, &to)
            } else {
                line.to_string()
            }
        })
        .collect();
    changed.then(|| out.concat())
}

/// Copies the ISO tree plus Rufus' extras (validation loaders, unattend,
/// FreeDOS, autorun) to `dest`.
pub(super) fn copy_files<T: fatfs::ReadWriteSeek>(
    job: &Job,
    rep: &Report,
    src: Option<&iso::Iso>,
    dest: &Dest<T>,
    ctx: &Ctx,
) -> Result<(), String> {
    copy_files_replacing(job, rep, src, dest, None, ctx)
}

/// Copies the ISO tree, optionally replacing one ISO path with staged files.
/// Replacement paths are `(destination, source)` pairs and are copied before
/// `md5sum.txt` is finalized so UEFI validation covers the staged content.
pub(super) fn copy_files_replacing<T: fatfs::ReadWriteSeek>(
    job: &Job,
    rep: &Report,
    src: Option<&iso::Iso>,
    dest: &Dest<T>,
    replacement: Option<(&str, &[(String, PathBuf)])>,
    ctx: &Ctx,
) -> Result<(), String> {
    let entries = match src {
        Some(iso) => iso.entries().map_err(e)?,
        None => Vec::new(),
    };
    let replaced = replacement.map(|(p, _)| p.to_ascii_lowercase());
    let skipped_size = replaced.as_deref().map_or(0, |p| {
        entries
            .iter()
            .filter(|x| !x.is_dir && x.path.eq_ignore_ascii_case(p))
            .map(|x| x.size)
            .sum()
    });
    let mut staged_size = 0u64;
    if let Some((_, files)) = replacement {
        for (_, path) in files {
            let metadata =
                std::fs::metadata(path).map_err(|err| format!("{}: {err}", path.display()))?;
            if !metadata.is_file() {
                return Err(format!(
                    "{}: staged source is not a regular file",
                    path.display()
                ));
            }
            staged_size = staged_size.saturating_add(metadata.len());
        }
    }
    let total = entries
        .iter()
        .filter(|x| replacement.is_none() || (!x.is_dir && x.symlink().is_none()))
        .map(|x| x.size)
        .sum::<u64>()
        .saturating_sub(skipped_size)
        .saturating_add(staged_size)
        .max(1);
    let (mut done, mut md5sums, mut loaders) = (0u64, String::new(), Vec::new());
    let find = |p: &str| {
        entries
            .iter()
            .find(|x| !x.is_dir && x.path.eq_ignore_ascii_case(p))
            .map(|x| x.path.clone())
    };
    let mut redirects = Vec::new();
    let mut buf = vec![0u8; 1 << 20];
    for entry in &entries {
        ctx.check()?;
        if entry.is_dir {
            dest.mkdir(&entry.path)
                .map_err(|err| format!("{}: {err}", entry.path))?;
            continue;
        }
        if replaced
            .as_deref()
            .is_some_and(|p| entry.path.eq_ignore_ascii_case(p))
        {
            continue;
        }
        if let Some(target) = entry.symlink() {
            match syslinux::knoppix_redirects(&entry.path, target, find) {
                Some(cfgs) => {
                    ctx.log(format!("Found symlink {} -> {target}", entry.path));
                    redirects.push((entry.path.as_str(), cfgs));
                }
                None => ctx.log(format!("Skipping symlink {} -> {target}", entry.path)),
            }
            continue;
        }
        let iso = src.expect("entries come from an ISO");
        let mut path = entry.path.clone();
        let lower = path.to_ascii_lowercase();
        if job.uefi_validation
            && let Some((_, loader)) = MD5_LOADERS
                .iter()
                .find(|(a, _)| lower == format!("efi/boot/boot{a}.efi"))
        {
            loaders.push((path.clone(), *loader));
            path = format!("{}_original.efi", &path[..path.len() - 4]);
        }
        let mut reader: Box<dyn Read> = Box::new(iso.reader(entry).map_err(e)?);
        if (lower.ends_with(".cfg") || lower.ends_with(".conf")) && entry.size < MB {
            let mut s = String::new();
            if reader.read_to_string(&mut s).is_ok() {
                let usb_label = sanitize_label(&job.label, job.fs);
                if let Some(p) = patch_label(&path, &s, &iso.volume_id(), &usb_label) {
                    ctx.log(format!(
                        "Patched {path}: '{}' → '{usb_label}'",
                        iso.volume_id()
                    ));
                    s = p;
                }
                if job.persistence > 0
                    && is_boot_cfg(&path)
                    && let Some(p) = patch_persistence(&s)
                {
                    ctx.log(format!("Patched {path} for persistence"));
                    s = p;
                }
                reader = Box::new(Cursor::new(s.into_bytes()));
            } else {
                reader = Box::new(iso.reader(entry).map_err(e)?);
            }
        }
        ctx.tx
            .send(super::Msg::CurrentFile(format!(
                "{path} ({})",
                crate::drives::human(entry.size)
            )))
            .ok();
        let mut out = dest.create(&path).map_err(|err| format!("{path}: {err}"))?;
        let mut md5 = Md5::new();
        loop {
            ctx.check()?;
            let n = reader
                .read(&mut buf)
                .map_err(|err| format!("Reading source {path}: {err}"))?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])
                .map_err(|err| format!("{path}: {err}"))?;
            md5.update(&buf[..n]);
            done += n as u64;
            let pct = done as f32 / total as f32;
            ctx.progress(pct, format!("Copying ISO files: {:.1}%", pct * 100.0));
        }
        out.flush()
            .map_err(|err| format!("Flushing destination {path}: {err}"))?;
        md5sums += &format!("{}  ./{path}\n", image::hex(&md5.finalize()));
    }

    if let Some((_, files)) = replacement {
        for (path, source) in files {
            ctx.check()?;
            let mut reader =
                File::open(source).map_err(|err| format!("{}: {err}", source.display()))?;
            let size = std::fs::metadata(source).map_err(e)?.len();
            ctx.tx
                .send(super::Msg::CurrentFile(format!(
                    "{path} ({})",
                    crate::drives::human(size)
                )))
                .ok();
            let mut out = dest.create(path).map_err(|err| format!("{path}: {err}"))?;
            let mut md5 = Md5::new();
            loop {
                ctx.check()?;
                let n = reader
                    .read(&mut buf)
                    .map_err(|err| format!("Reading source {path}: {err}"))?;
                if n == 0 {
                    break;
                }
                out.write_all(&buf[..n])
                    .map_err(|err| format!("{path}: {err}"))?;
                md5.update(&buf[..n]);
                done += n as u64;
                let pct = done as f32 / total as f32;
                ctx.progress(pct, format!("Copying ISO files: {:.1}%", pct * 100.0));
            }
            out.flush()
                .map_err(|err| format!("Flushing destination {path}: {err}"))?;
            md5sums += &format!("{}  ./{path}\n", image::hex(&md5.finalize()));
        }
    }

    let has = |p: &str| entries.iter().any(|x| x.path.eq_ignore_ascii_case(p));
    // Rufus only applies the Knoppix workaround to Syslinux images.
    if !redirects.is_empty() && src.and_then(syslinux::detect).is_some() {
        for (dir, cfgs) in redirects {
            dest.mkdir(dir).map_err(|err| format!("{dir}: {err}"))?;
            for (path, data) in cfgs {
                dest.put(&path, &data)
                    .map_err(|err| format!("{path}: {err}"))?;
                ctx.log(format!("Created: {path}"));
            }
        }
    }
    for (path, loader) in loaders {
        dest.put(&path, loader).map_err(e)?;
    }
    if job.uefi_validation && src.is_some() {
        let header = format!("# md5_totalbytes = 0x{done:x}\n");
        dest.put("md5sum.txt", (header + &md5sums).as_bytes())
            .map_err(e)?;
    }
    if rep.windows && job.wue.any() {
        ctx.log("Writing autounattend.xml");
        dest.put(
            "autounattend.xml",
            wue::unattend_xml(&job.wue, rep.arch).as_bytes(),
        )
        .map_err(e)?;
    }
    if job.boot == Boot::FreeDos {
        dest.mkdir("LOCALE").map_err(e)?;
        for (name, data) in FREEDOS.iter() {
            let p = if matches!(*name, "COMMAND.COM" | "KERNEL.SYS") {
                name.to_string()
            } else {
                format!("LOCALE/{name}")
            };
            dest.put(&p, data).map_err(e)?;
        }
        dest.put("AUTOEXEC.BAT", b"@echo off\r\nSET PATH=\\LOCALE\r\n")
            .map_err(e)?;
    }
    if job.ext_label && !has("autorun.inf") {
        let inf = format!(
            "; Created by Rustfus\r\n[autorun]\r\nicon  = autorun.ico\r\nlabel = {}\r\n",
            job.label
        );
        dest.put("autorun.inf", inf.as_bytes()).map_err(e)?;
        dest.put("autorun.ico", AUTORUN_ICO).map_err(e)?;
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn fixture() -> (PathBuf, MountedDir<'static>) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "rustfus-files-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir(&root).unwrap();
        let guard: &'static dyn Fn() -> Result<(), String> = Box::leak(Box::new(|| Ok(())));
        let mounted = MountedDir::open(root.clone(), guard).unwrap();
        (root, mounted)
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "creates and mounts a disposable local FAT32 disk image, never a physical USB"]
    fn macos_fat32_disk_image_supports_guarded_write_and_flush() {
        use std::process::Command;
        let directory = tempfile::tempdir().unwrap();
        let image = directory.path().join("fixture.dmg");
        let mount = directory.path().join("mounted");
        std::fs::create_dir(&mount).unwrap();
        let create = Command::new("/usr/bin/hdiutil")
            .args([
                "create",
                "-size",
                "96m",
                "-fs",
                "MS-DOS FAT32",
                "-volname",
                "RUSTFUSTEST",
                "-layout",
                "GPTSPUD",
            ])
            .arg(&image)
            .output()
            .unwrap();
        assert!(
            create.status.success(),
            "{}",
            String::from_utf8_lossy(&create.stderr)
        );
        let attach = Command::new("/usr/bin/hdiutil")
            .args(["attach", "-nobrowse", "-mountpoint"])
            .arg(&mount)
            .arg(&image)
            .output()
            .unwrap();
        assert!(
            attach.status.success(),
            "{}",
            String::from_utf8_lossy(&attach.stderr)
        );
        struct Detach(PathBuf);
        impl Drop for Detach {
            fn drop(&mut self) {
                let _ = Command::new("/usr/bin/hdiutil")
                    .arg("detach")
                    .arg(&self.0)
                    .output();
            }
        }
        let detach = Detach(mount.clone());
        let guard = || Ok(());
        let mounted = MountedDir::open(mount.clone(), &guard).unwrap();
        let dest: Dest<Cursor<Vec<u8>>> = Dest::GuardedDir(&mounted);
        dest.mkdir("sources").expect("mkdir on FAT32");
        dest.put("sources/install.swm", b"fixture content")
            .expect("write and sync on FAT32");
        dest.put("sources/install.swm", b"replacement")
            .expect("truncate and sync on FAT32");
        assert_eq!(
            std::fs::read(mount.join("sources/install.swm")).unwrap(),
            b"replacement"
        );
        let info = Command::new("/usr/sbin/diskutil")
            .args(["info", "-plist"])
            .arg(&mount)
            .output()
            .unwrap();
        assert!(info.status.success());
        let info = plist::Value::from_reader(Cursor::new(info.stdout)).unwrap();
        println!(
            "FAT32 plist identity: {:?}",
            info.as_dictionary()
                .unwrap()
                .iter()
                .filter(|(k, _)| [
                    "FilesystemType",
                    "FilesystemName",
                    "FileSystemPersonality",
                    "Type",
                    "DeviceIdentifier",
                    "ParentWholeDisk",
                    "PartOfWhole",
                    "MountPoint"
                ]
                .contains(&k.as_str()))
                .collect::<Vec<_>>()
        );
        drop(dest);
        drop(mounted);
        drop(detach);
    }

    #[test]
    fn guarded_paths_reject_escaping_components() {
        let (root, mounted) = fixture();
        for path in [
            "/absolute",
            "../escape",
            "a/../b",
            "a/./b",
            "a\\b",
            "a\0b",
            "C:x",
        ] {
            assert!(
                mounted.create(path).is_err(),
                "accepted unsafe path {path:?}"
            );
        }
        drop(mounted);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn guarded_paths_reject_symlink_components() {
        let (root, mounted) = fixture();
        std::fs::create_dir(root.join("real")).unwrap();
        std::os::unix::fs::symlink("real", root.join("link")).unwrap();
        assert!(mounted.create("link/file").is_err());
        assert!(mounted.mkdir("link/child").is_err());
        std::fs::write(root.join("real/file"), b"unchanged").unwrap();
        std::os::unix::fs::symlink("real/file", root.join("file-link")).unwrap();
        assert!(mounted.create("file-link").is_err());
        std::fs::hard_link(root.join("real/file"), root.join("hard-link")).unwrap();
        assert!(mounted.create("hard-link").is_err());
        assert_eq!(std::fs::read(root.join("real/file")).unwrap(), b"unchanged");
        drop(mounted);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn guard_is_checked_after_open_for_writes_and_flushes() {
        let root = std::env::temp_dir().join(format!("rustfus-guard-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let valid = std::cell::Cell::new(true);
        let guard = || {
            if valid.get() {
                Ok(())
            } else {
                Err("mount changed".into())
            }
        };
        let mounted = MountedDir::open(root.clone(), &guard).unwrap();
        let dest: Dest<Cursor<Vec<u8>>> = Dest::GuardedDir(&mounted);
        let mut out = dest.create("file").unwrap();
        out.write_all(b"before").unwrap();
        valid.set(false);
        assert!(out.write_all(b"after").is_err());
        assert!(out.flush().is_err());
        assert!(dest.mkdir("new").is_err());
        drop(out);
        assert_eq!(std::fs::read(root.join("file")).unwrap(), b"before");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retained_root_cannot_be_redirected_by_path_replacement() {
        let (root, mounted) = fixture();
        let renamed = root.with_extension("original");
        std::fs::rename(&root, &renamed).unwrap();
        std::fs::create_dir(&root).unwrap();
        let dest: Dest<Cursor<Vec<u8>>> = Dest::GuardedDir(&mounted);
        dest.put("file", b"anchored").unwrap();
        assert!(!root.join("file").exists());
        assert_eq!(std::fs::read(renamed.join("file")).unwrap(), b"anchored");
        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_dir_all(renamed).unwrap();
    }
    fn record(extent: u32, size: u32, flags: u8, name: &[u8]) -> Vec<u8> {
        let mut r = vec![0u8; 33];
        r[2..6].copy_from_slice(&extent.to_le_bytes());
        r[10..14].copy_from_slice(&size.to_le_bytes());
        r[25] = flags;
        r[32] = name.len() as u8;
        r.extend_from_slice(name);
        if !r.len().is_multiple_of(2) {
            r.push(0);
        }
        r[0] = r.len() as u8;
        r
    }

    #[test]
    fn replacement_updates_progress_and_validation_sums() {
        let (root, mounted) = fixture();
        let mut bytes = vec![0u8; 22 * 2048];
        bytes[16 * 2048] = 1;
        bytes[16 * 2048 + 1..16 * 2048 + 6].copy_from_slice(b"CD001");
        bytes[16 * 2048 + 6] = 1;
        bytes[16 * 2048 + 40..16 * 2048 + 72].fill(b' ');
        bytes[16 * 2048 + 156..16 * 2048 + 190].copy_from_slice(&record(18, 2048, 2, &[0]));
        bytes[17 * 2048] = 255;
        bytes[17 * 2048 + 1..17 * 2048 + 6].copy_from_slice(b"CD001");
        for (block, records) in [
            (
                18,
                vec![
                    record(18, 2048, 2, &[0]),
                    record(18, 2048, 2, &[1]),
                    record(19, 2048, 2, b"SOURCES"),
                    record(21, 4, 0, b"KEEP.TXT;1"),
                ],
            ),
            (
                19,
                vec![
                    record(19, 2048, 2, &[0]),
                    record(18, 2048, 2, &[1]),
                    record(20, 500, 0, b"INSTALL.WIM;1"),
                ],
            ),
        ] {
            let data = records.concat();
            bytes[block * 2048..block * 2048 + data.len()].copy_from_slice(&data);
        }
        bytes[20 * 2048..20 * 2048 + 500].fill(0xff);
        bytes[21 * 2048..21 * 2048 + 4].copy_from_slice(b"keep");
        let iso_path = root.join("fixture.iso");
        std::fs::write(&iso_path, bytes).unwrap();
        let iso = iso::Iso::open(&iso_path).unwrap();
        let stage = root.join("piece");
        std::fs::write(&stage, b"split").unwrap();
        let files = vec![("SOURCES/install.swm".to_string(), stage)];
        let job = Job {
            drive: crate::drives::Drive {
                path: "unused".into(),
                name: "unused".into(),
                size: 0,
                serial: None,
            },
            boot: Boot::Image(iso_path),
            dd: false,
            scheme: super::super::Scheme::Gpt,
            target: super::super::Target::Uefi,
            fs: super::super::Fs::Fat32,
            cluster: 0,
            label: "TEST".into(),
            quick: true,
            ext_label: true,
            bad_blocks: None,
            verify: false,
            old_bios: false,
            uefi_validation: true,
            persistence: 0,
            wue: wue::Options {
                bypass_hw_checks: true,
                ..Default::default()
            },
            allow_downloads: false,
            replace_c32: vec![],
            wtg: None,
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let ctx = Ctx {
            tx: &tx,
            cancel: &cancel,
        };
        let dest: Dest<Cursor<Vec<u8>>> = Dest::GuardedDir(&mounted);
        copy_files_replacing(
            &job,
            &Report {
                windows: true,
                arch: "x64",
                ..Default::default()
            },
            Some(&iso),
            &dest,
            Some(("sources/install.wim", &files)),
            &ctx,
        )
        .unwrap();
        assert!(!root.join("SOURCES/INSTALL.WIM").exists());
        assert_eq!(
            std::fs::read(root.join("SOURCES/install.swm")).unwrap(),
            b"split"
        );
        let sums = std::fs::read_to_string(root.join("md5sum.txt")).unwrap();
        assert!(sums.starts_with("# md5_totalbytes = 0x9\n"), "{sums}");
        assert!(sums.contains(&format!(
            "{}  ./SOURCES/install.swm\n",
            image::hex(&Md5::digest(b"split"))
        )));
        assert!(!sums.to_ascii_lowercase().contains("install.wim"));
        let messages: Vec<_> = rx.try_iter().collect();
        let progress: Vec<_> = messages
            .iter()
            .filter_map(|m| {
                if let super::super::Msg::Progress(p, _) = m {
                    Some(*p)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(progress.last(), Some(&1.0));
        assert!(messages.iter().any(|m| matches!(m, super::super::Msg::CurrentFile(p) if p.starts_with("SOURCES/install.swm"))));
        assert!(root.join("autounattend.xml").exists());
        assert!(root.join("autorun.inf").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
