// SPDX-License-Identifier: GPL-3.0-or-later
//! FUSE-free NTFS preparation in a private image before opening the target disk.
use super::{
    Boot, Ctx, Fs, Job, MB, Report, Target,
    files::{Dest, MountedDir, copy_files},
    layout::{layout, write_tables, zero_range},
};
use crate::{
    blockdev::{self, BlockDev},
    iso::Iso,
    native_tools::{Tool, Tools},
    part,
};
use std::{
    collections::HashMap,
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    io::{Cursor, Read, Seek, Write},
    path::{Path, PathBuf},
};

fn safe_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(['\\', '\0'])
        && path.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part.starts_with('$')
                && !part.ends_with([' ', '.'])
                && !part
                    .chars()
                    .any(|c| c.is_control() || "<>:\"|?*".contains(c))
                && part.encode_utf16().count() <= 255
        })
}

fn probe_staging_paths<'a>(
    root: &Path,
    entries: impl IntoIterator<Item = (&'a str, bool)>,
) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let mut identities = HashMap::new();
    for (path, directory) in entries {
        if !safe_path(path) {
            return Err(format!("Unsafe staging path: {path}"));
        }
        let components: Vec<_> = path.split('/').collect();
        let mut target = root.to_path_buf();
        for (index, component) in components.iter().enumerate() {
            target.push(component);
            let logical = components[..=index].join("/");
            let is_dir = index + 1 < components.len() || directory;
            if is_dir {
                match fs::create_dir(&target) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => {
                        return Err(format!("Probing staged directory {logical}: {error}"));
                    }
                }
            } else {
                OpenOptions::new().write(true).create_new(true).open(&target)
                    .map_err(|error| format!("Staged ISO paths alias or conflict at {logical}: {error}; no disk was erased"))?;
            }
            let metadata = fs::symlink_metadata(&target).map_err(super::e)?;
            if metadata.is_dir() != is_dir {
                return Err(format!("Staged file/directory conflict at {logical}"));
            }
            let key = (metadata.dev(), metadata.ino());
            if let Some(previous) = identities.insert(key, logical.clone())
                && previous != logical
            {
                return Err(format!(
                    "ISO paths {previous} and {logical} alias on the staging filesystem; no disk was erased"
                ));
            }
        }
    }
    Ok(())
}

fn validate_entries(iso: &Iso) -> Result<u64, String> {
    let entries = iso.entries().map_err(super::e)?;
    let mut paths = HashMap::new();
    let mut total = 0u64;
    for entry in entries {
        if !safe_path(&entry.path) {
            return Err(format!(
                "Unsafe NTFS destination path: {}; no disk was erased",
                entry.path
            ));
        }
        if paths
            .insert(entry.path.to_lowercase(), entry.is_dir)
            .is_some()
        {
            return Err(format!(
                "Duplicate case-insensitive NTFS path: {}; no disk was erased",
                entry.path
            ));
        }
        if !entry.is_dir {
            total = total
                .checked_add(entry.size)
                .ok_or("Image sizes overflow")?;
        }
    }
    for path in paths.keys() {
        let mut parent = path.as_str();
        while let Some((prefix, _)) = parent.rsplit_once('/') {
            if paths.get(prefix) == Some(&false) {
                return Err(format!("NTFS file/directory conflict: {prefix}"));
            }
            parent = prefix;
        }
    }
    Ok(total)
}

fn available_space(path: &Path) -> Result<u64, String> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let path = CString::new(path.as_os_str().as_bytes()).map_err(super::e)?;
    let mut status = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: path is NUL-terminated, and status is initialized on success.
    if unsafe { libc::statvfs(path.as_ptr(), status.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    // SAFETY: statvfs returned success.
    let status = unsafe { status.assume_init() };
    #[allow(clippy::unnecessary_cast)]
    Ok((status.f_bavail as u64).saturating_mul(status.f_frsize as u64))
}

#[allow(clippy::too_many_arguments)]
fn make_image(
    tools: &Tools,
    image: &Path,
    tree: &Path,
    size: u64,
    label: &str,
    cluster: u32,
    hidden: u64,
    ctx: &Ctx,
) -> Result<(), String> {
    ctx.check()?;
    File::create(image)
        .and_then(|file| file.set_len(size))
        .map_err(super::e)?;
    let cluster = cluster.to_string();
    let hidden = hidden.to_string();
    let mut args = vec![
        OsStr::new("-Q"),
        OsStr::new("-F"),
        OsStr::new("-L"),
        OsStr::new(label),
        OsStr::new("-s"),
        OsStr::new("512"),
        OsStr::new("-p"),
        OsStr::new(&hidden),
        OsStr::new("-H"),
        OsStr::new("255"),
        OsStr::new("-S"),
        OsStr::new("63"),
    ];
    if cluster != "0" {
        args.extend([OsStr::new("-c"), OsStr::new(&cluster)]);
    }
    args.push(image.as_os_str());
    ctx.progress(0., "Formatting private NTFS image before erasing USB…");
    tools.run(Tool::Mkntfs, &args, ctx.cancel)?;
    ctx.progress(0., "Importing prepared files into NTFS before erasing USB…");
    tools.run(
        Tool::NtfsImport,
        &[OsStr::new("import"), image.as_os_str(), tree.as_os_str()],
        ctx.cancel,
    )?;
    ctx.progress(0., "Verifying prepared NTFS files before erasing USB…");
    tools.run(
        Tool::NtfsImport,
        &[OsStr::new("verify"), image.as_os_str(), tree.as_os_str()],
        ctx.cancel,
    )?;
    ctx.check()
}

struct Prepared {
    _directory: tempfile::TempDir,
    image: PathBuf,
    size: u64,
}

fn prepare(
    job: &Job,
    report: &Report,
    iso: Option<&Iso>,
    size: u64,
    hidden: u64,
    ctx: &Ctx,
) -> Result<Prepared, String> {
    let tools = Tools::new()?;
    let total = iso.map(validate_entries).transpose()?.unwrap_or(0);
    if total.checked_add(256 * MB).is_none_or(|bytes| bytes > size) {
        return Err("The image does not fit the NTFS partition; no disk was erased".into());
    }
    if job.cluster != 0
        && (!job.cluster.is_power_of_two() || !(512..=2 << 20).contains(&job.cluster))
    {
        return Err("Invalid NTFS cluster size; no disk was erased".into());
    }
    let directory = tempfile::Builder::new()
        .prefix("rustfus-ntfs-")
        .tempdir()
        .map_err(super::e)?;
    // The staged tree and the filesystem image coexist; reserve for both and NTFS metadata.
    if available_space(directory.path())? < size.saturating_add(total).saturating_add(256 * MB) {
        return Err(format!(
            "NTFS preparation needs about {} of temporary free space (partition image plus staged files); no disk was erased",
            crate::drives::human(size.saturating_add(total).saturating_add(256 * MB))
        ));
    }
    let tree = directory.path().join("files");
    let probe = directory.path().join("path-probe");
    fs::create_dir(&probe).map_err(super::e)?;
    if let Some(iso) = iso {
        let entries = iso.entries().map_err(super::e)?;
        probe_staging_paths(
            &probe,
            entries
                .iter()
                .map(|entry| (entry.path.as_str(), entry.is_dir)),
        )?;
    }
    fs::remove_dir_all(probe).map_err(super::e)?;
    fs::create_dir(&tree).map_err(super::e)?;
    let guard = || ctx.check();
    let mounted = MountedDir::open(tree.clone(), &guard).map_err(super::e)?;
    let destination: Dest<'_, Cursor<Vec<u8>>> = Dest::GuardedDir(&mounted);
    copy_files(job, report, iso, &destination, ctx)?;
    drop(destination);
    drop(mounted);
    let image = directory.path().join("partition.ntfs");
    make_image(
        &tools,
        &image,
        &tree,
        size,
        &super::sanitize_label(&job.label, Fs::Ntfs),
        job.cluster,
        hidden,
        ctx,
    )?;
    if report.windows && job.target != Target::Uefi {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&image)
            .map_err(super::e)?;
        let mut volume = BlockDev::new(&file, 0, size);
        crate::bootrec::write_ntfs_pbr(&mut volume).map_err(super::e)?;
        volume.flush().map_err(super::e)?;
    }
    Ok(Prepared {
        _directory: directory,
        image,
        size,
    })
}

fn write_image(
    prepared: &Prepared,
    device: &File,
    offset: u64,
    verify: bool,
    ctx: &Ctx,
) -> Result<(), String> {
    let mut source = File::open(&prepared.image).map_err(super::e)?;
    let mut buffer = vec![0; 4 << 20];
    let mut position = 0;
    while position < prepared.size {
        ctx.check()?;
        let count = (prepared.size - position).min(buffer.len() as u64) as usize;
        source.read_exact(&mut buffer[..count]).map_err(super::e)?;
        blockdev::write_at(device, &buffer[..count], offset + position).map_err(|error| {
            format!("Writing prepared NTFS partition at byte {position}: {error}")
        })?;
        position += count as u64;
        ctx.progress(
            position as f32 / prepared.size as f32,
            "Writing prepared NTFS partition…",
        );
    }
    blockdev::sync_storage(device).map_err(|error| format!("Flushing NTFS partition: {error}"))?;
    if verify {
        source.rewind().map_err(super::e)?;
        let mut actual = vec![0; buffer.len()];
        let mut position = 0;
        while position < prepared.size {
            ctx.check()?;
            let count = (prepared.size - position).min(buffer.len() as u64) as usize;
            source.read_exact(&mut buffer[..count]).map_err(super::e)?;
            blockdev::read_at(device, &mut actual[..count], offset + position).map_err(super::e)?;
            if buffer[..count] != actual[..count] {
                return Err(format!("NTFS readback mismatch at byte {position}"));
            }
            position += count as u64;
            ctx.progress(
                position as f32 / prepared.size as f32,
                "Verifying NTFS partition…",
            );
        }
    }
    Ok(())
}

pub(super) fn run(
    job: &Job,
    report: &Report,
    iso: Option<&Iso>,
    ctx: &Ctx,
) -> Result<String, String> {
    if job.boot == Boot::FreeDos || job.wtg.is_some() {
        return Err("This boot mode does not support bundled NTFS creation".into());
    }
    let minimum = report
        .size
        .checked_add(512 * MB)
        .ok_or("Image sizes overflow")?;
    if job.drive.size < minimum {
        return Err("The image does not fit on the selected drive; no disk was erased".into());
    }
    let parts = layout(job, iso.is_some() && job.target != Target::Bios);
    let prepared = prepare(
        job,
        report,
        iso,
        parts[0].size,
        parts[0].start / part::SECTOR,
        ctx,
    )?;
    ctx.bad_blocks(job)?;
    let device = ctx.open_unmounted(&job.drive)?;
    if !job.quick {
        zero_range(
            &device,
            0,
            job.drive.size / part::SECTOR * part::SECTOR,
            Some(ctx),
        )?;
    }
    write_tables(job, report, None, &parts, &device, ctx)?;
    write_image(&prepared, &device, parts[0].start, job.verify, ctx)?;
    if let Some(partition) = parts.iter().find(|p| p.name == "persistence") {
        let (label, contents): (&str, &[(&str, &[u8])]) = if report.casper {
            ("casper-rw", &[])
        } else {
            ("persistence", &[("persistence.conf", b"/ union\n")])
        };
        let mut volume = BlockDev::new(&device, partition.start, partition.size);
        crate::ext::format(
            &mut volume,
            partition.size,
            crate::ext::Kind::Ext3,
            label,
            0,
            contents,
        )
        .map_err(super::e)?;
        volume.flush().map_err(super::e)?;
    }
    drop(device);
    crate::fsos::rescan(&job.drive)?;
    Ok(format!(
        "{} ready (bundled NTFS, no FUSE driver)",
        job.drive.path
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{atomic::AtomicBool, mpsc};

    #[test]
    #[ignore = "writes and verifies more than 4 GB in disposable regular files, no devices"]
    fn bundled_ntfs_preserves_files_larger_than_fat32_limit() {
        let tools = Tools::new().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let tree = directory.path().join("files");
        fs::create_dir_all(tree.join("sources")).unwrap();
        let size = (4u64 << 30) + 65536;
        let mut source = File::create(tree.join("sources/install.wim")).unwrap();
        source.set_len(size).unwrap();
        source.write_all(b"MSWIM fixture").unwrap();
        source.seek(std::io::SeekFrom::End(-4)).unwrap();
        source.write_all(b"tail").unwrap();
        drop(source);
        let (tx, _rx) = mpsc::channel();
        let cancel = AtomicBool::new(false);
        let ctx = Ctx {
            tx: &tx,
            cancel: &cancel,
        };
        make_image(
            &tools,
            &directory.path().join("large.ntfs"),
            &tree,
            5u64 << 30,
            "LARGE",
            4096,
            2048,
            &ctx,
        )
        .unwrap();
    }
    #[test]
    fn staging_probe_rejects_host_aliases_before_copying() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("caf\u{e9}.txt");
        fs::write(&first, []).unwrap();
        let aliases = directory.path().join("cafe\u{301}.txt").exists();
        fs::remove_file(first).unwrap();
        let result = probe_staging_paths(
            directory.path(),
            [("caf\u{e9}.txt", false), ("cafe\u{301}.txt", false)],
        );
        assert_eq!(result.is_err(), aliases);
        let directory = tempfile::tempdir().unwrap();
        assert!(
            probe_staging_paths(
                directory.path(),
                [("dir", true), ("dir/a", false), ("dir", true)]
            )
            .is_ok()
        );
        assert!(probe_staging_paths(directory.path(), [("dir/a", true)]).is_err());
    }
    #[test]
    fn rejects_paths_that_cannot_be_safely_staged_for_ntfs() {
        for path in [
            "/root",
            "../x",
            "a/../b",
            "x:y",
            "x\\y",
            "$MFT",
            "a/",
            "trailing.",
            "question?",
            "a\0b",
        ] {
            assert!(!safe_path(path), "{path:?}");
        }
        assert!(safe_path("EFI/BOOT/BOOTX64.EFI"));
        assert!(safe_path("sources/日本語.txt"));
    }
    #[test]
    fn bundled_ntfs_image_import_and_readback_without_a_mount() {
        let tools = Tools::new().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let tree = directory.path().join("files");
        fs::create_dir_all(tree.join("EFI/BOOT")).unwrap();
        fs::create_dir_all(tree.join("sources")).unwrap();
        fs::write(tree.join("EFI/BOOT/BOOTX64.EFI"), b"fixture EFI").unwrap();
        fs::write(tree.join("sources/日本語.txt"), b"unicode filename").unwrap();
        fs::write(tree.join("large.bin"), vec![0x6b; 3 << 20]).unwrap();
        let image = directory.path().join("image.ntfs");
        let (tx, _rx) = mpsc::channel();
        let cancel = AtomicBool::new(false);
        let ctx = Ctx {
            tx: &tx,
            cancel: &cancel,
        };
        let size = 64 * MB;
        make_image(&tools, &image, &tree, size, "TEST", 4096, 2048, &ctx).unwrap();
        let hostile = directory.path().join("hostile");
        fs::create_dir(&hostile).unwrap();
        std::os::unix::fs::symlink(tree.join("large.bin"), hostile.join("link")).unwrap();
        assert!(
            tools
                .run(
                    Tool::NtfsImport,
                    &[OsStr::new("import"), image.as_os_str(), hostile.as_os_str()],
                    &cancel
                )
                .is_err()
        );
        let file = File::open(&image).unwrap();
        let mut header = [0; 512];
        blockdev::read_at(&file, &mut header, 0).unwrap();
        assert_eq!(&header[3..11], b"NTFS    ");
        assert_eq!(u32::from_le_bytes(header[28..32].try_into().unwrap()), 2048);
        drop(file);
        let prepared = Prepared {
            _directory: directory,
            image,
            size,
        };
        let destination = tempfile::tempfile().unwrap();
        destination.set_len(size + MB).unwrap();
        write_image(&prepared, &destination, MB, true, &ctx).unwrap();
        fs::write(tree.join("large.bin"), b"changed").unwrap();
        assert!(
            tools
                .run(
                    Tool::NtfsImport,
                    &[
                        OsStr::new("verify"),
                        prepared.image.as_os_str(),
                        tree.as_os_str()
                    ],
                    &cancel
                )
                .is_err()
        );
    }
}
