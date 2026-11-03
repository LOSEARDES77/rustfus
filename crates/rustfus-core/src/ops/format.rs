// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! ISO mode / plain format onto FAT, ext or an OS-created filesystem.

use super::files::{Dest, DirDest, copy_files, copy_files_replacing};
use super::layout::{ext_kind, fat_label, layout, write_tables, zero_range};
use super::{
    Boot, Ctx, Fs, Job, MB, Report, Scheme, Target, analyze_file, e, open_dev, open_image,
    sanitize_label,
};
use crate::blockdev::BlockDev;
use crate::{bootrec, fsos, iso, part, syslinux};
use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

type Replacement<'a> = Option<(&'a str, &'a [(String, std::path::PathBuf)])>;

/// FAT/FAT32 path: Rustfus formats and fills the partition itself.
#[cfg(test)]
pub(super) fn write_fat(
    job: &Job,
    rep: &Report,
    iso: Option<&iso::Iso>,
    sl: Option<&syslinux::Info>,
    main: &part::Part,
    dev: &File,
    ctx: &Ctx,
) -> Result<(), String> {
    write_fat_replacing(job, rep, iso, sl, main, dev, ctx, None)
}

#[allow(clippy::too_many_arguments)]
fn write_fat_replacing(
    job: &Job,
    rep: &Report,
    iso: Option<&iso::Iso>,
    sl: Option<&syslinux::Info>,
    main: &part::Part,
    dev: &File,
    ctx: &Ctx,
    replacement: Replacement<'_>,
) -> Result<(), String> {
    ctx.progress(0.0, format!("Formatting ({})...", job.fs.name()));
    let mut vol = BlockDev::new(dev, main.start, main.size);
    let label = fat_label(&sanitize_label(&job.label, job.fs));
    let hidden = (main.start / part::SECTOR) as u32;
    crate::fat::format(
        &mut vol,
        main.size,
        job.fs == Fs::Fat32,
        job.cluster,
        label,
        hidden,
    )
    .map_err(|err| format!("Format failed: {err}"))?;
    {
        let fat = fatfs::FileSystem::new(&mut vol, fatfs::FsOptions::new()).map_err(e)?;
        let dest = Dest::Fat(&fat);
        copy_files_replacing(job, rep, iso, &dest, replacement, ctx)?;
        if let (Some(iso), Some(sl)) = (iso, sl) {
            for (path, data) in syslinux::extra_files(iso, sl, &job.replace_c32, &|m| ctx.log(m))? {
                dest.put(&path, &data).map_err(e)?;
            }
        }
        drop(dest);
        fat.unmount().map_err(e)?;
    }
    crate::fat::fix_dot_entries(&mut vol).map_err(e)?;
    vol.seek(SeekFrom::Start(0)).map_err(e)?;
    if let Some(sl) = sl {
        ctx.progress(1.0, "Installing Syslinux...");
        syslinux::install(&mut vol, sl, job.allow_downloads, &|m| ctx.log(m))?;
    } else if job.fs == Fs::Fat32 && job.target != Target::Uefi {
        if job.boot == Boot::FreeDos {
            bootrec::write_fat32_freedos_pbr(&mut vol).map_err(e)?;
        } else if rep.windows {
            bootrec::write_fat32_pe_pbr(&mut vol).map_err(e)?;
        }
    }
    vol.flush().map_err(e)
}

/// Whether Syslinux, detected as `sl`, gets installed for BIOS boot from
/// `fs`. Like Rufus' BootCheck, an image only Syslinux can BIOS-boot is
/// refused on a filesystem Syslinux can't read: exFAT (no Syslinux driver) or
/// NTFS before Syslinux 6. GRUB 2 images fall back to GRUB instead.
pub(super) fn syslinux_for(
    rep: &Report,
    fs: Fs,
    scheme: Scheme,
    target: Target,
    sl: Option<syslinux::Info>,
) -> Result<Option<syslinux::Info>, String> {
    let Some(sl) = sl else {
        return Ok(None);
    };
    if rep.windows || target == Target::Uefi || scheme != Scheme::Mbr {
        return Ok(None);
    }
    let ntfs_ok = sl.major >= 6;
    // Rustfus writes NTFS through the OS driver and needs it to map ldlinux.sys.
    let ntfs_here = cfg!(target_os = "linux");
    if fs.is_fat() || (fs == Fs::Ntfs && ntfs_ok && ntfs_here) {
        return Ok(Some(sl));
    }
    if rep.grub2 || !matches!(fs, Fs::Ntfs | Fs::Exfat) {
        return Ok(None);
    }
    let why = match fs {
        Fs::Ntfs if ntfs_ok => "Installing Syslinux on NTFS needs Linux".to_string(),
        Fs::Ntfs => format!(
            "Syslinux {} can't boot from NTFS (6.00 or later can)",
            sl.version
        ),
        _ => "Syslinux can't boot from exFAT".to_string(),
    };
    let alternatives = if ntfs_ok && ntfs_here {
        "FAT32 or NTFS"
    } else {
        "FAT32"
    };
    Err(format!(
        "{why}. Use {alternatives}, or select UEFI as the target system."
    ))
}

/// Writes Syslinux' extra files under the mounted `root` and stages ldlinux.sys there.
fn stage_syslinux(
    job: &Job,
    iso: &iso::Iso,
    sl: &syslinux::Info,
    root: &Path,
    ctx: &Ctx,
) -> Result<syslinux::Staged, String> {
    let dest = DirDest::Dir(root.to_path_buf());
    for (path, data) in syslinux::extra_files(iso, sl, &job.replace_c32, &|m| ctx.log(m))? {
        dest.put(&path, &data).map_err(e)?;
    }
    ctx.progress(1.0, "Installing Syslinux...");
    syslinux::stage(root, sl, job.allow_downloads, &|m| ctx.log(m))
}

/// ISO mode / format: partition, create the filesystem, copy files, install boot code.
pub(super) fn write_fs(job: &Job, pinned: Option<&File>, ctx: &Ctx) -> Result<String, String> {
    let (iso, rep) = match &job.boot {
        Boot::Image(p) => {
            ctx.log(format!("Reading source ISO {}", p.display()));
            let file = open_image(p, pinned)?;
            let rep = analyze_file(file.try_clone().map_err(e)?)
                .map_err(|error| format!("Analyzing source ISO {}: {error}", p.display()))?;
            (
                Some(
                    iso::Iso::from_file(file)
                        .map_err(|error| format!("Opening ISO {}: {error}", p.display()))?,
                ),
                rep,
            )
        }
        _ => (None, Report::default()),
    };
    let fs = job.fs;
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    let staged_wim = if fs == Fs::Fat32 && rep.windows && rep.large_file {
        let iso = iso.as_ref().ok_or("Select a Windows ISO")?;
        let entries = iso.entries().map_err(e)?;
        let large: Vec<_> = entries
            .iter()
            .filter(|entry| !entry.is_dir && entry.size > super::FAT_MAX_FILE)
            .collect();
        if large.len() != 1 || !large[0].path.eq_ignore_ascii_case("sources/install.wim") {
            return Err("FAT32 can split sources/install.wim only; use NTFS for oversized ESD or other files. No disk was erased.".into());
        }
        Some(super::wim::stage_wim(iso, large[0], ctx)?)
    } else {
        None
    };
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    let replacement = staged_wim
        .as_ref()
        .map(|staged| (staged.original.as_str(), staged.files.as_slice()));
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let replacement: Replacement<'_> = None;
    if fs.is_fat() && rep.large_file && replacement.is_none() {
        return Err("This image contains a file larger than 4 GB, which FAT32 cannot store. Use NTFS or exFAT.".into());
    }
    if !fs.supported() {
        return Err(format!("{} cannot be created on this OS", fs.name()));
    }
    // UEFI:NTFS can load bootmgfw.efi from exFAT, but the Windows boot manager
    // then reads the BCD with its own drivers, which lack exFAT (0xc0000272).
    if fs == Fs::Exfat && rep.windows {
        return Err("Windows install media can't boot from exFAT. Use NTFS.".into());
    }
    // Ext is formatted by Rustfus itself; the others need the OS tool.
    if ext_kind(fs).is_none()
        && !(cfg!(unix) && fs == Fs::Ntfs)
        && let Some(tool) = fsos::missing_tool(fs)
    {
        return Err(format!(
            "Formatting {} needs {tool}, which is not installed",
            fs.name()
        ));
    }
    let label = sanitize_label(&job.label, fs);
    // Like Rufus: Syslinux only for non-Windows, BIOS-capable targets.
    let sl = syslinux_for(
        &rep,
        fs,
        job.scheme,
        job.target,
        iso.as_ref().and_then(syslinux::detect),
    )?;
    #[cfg(target_os = "linux")]
    if fs == Fs::Ntfs && sl.is_some() {
        if let Some(tool) = fsos::missing_tool(fs) {
            return Err(format!("Syslinux on NTFS needs {tool}; no disk was erased"));
        }
        fsos::preflight_ntfs_mount()?;
    }
    if let Some(sl) = &sl {
        ctx.log(format!("Syslinux {}{} detected", sl.version, sl.ext));
        // Fetch a non-embedded loader now so a download failure leaves the drive untouched.
        syslinux::loader(sl, job.allow_downloads, &|m| ctx.log(m))?;
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    if fs == Fs::Ntfs && sl.is_none() {
        return super::ntfs::run(job, &rep, iso.as_ref(), ctx);
    }
    let parts = layout(
        job,
        !fs.is_fat() && iso.is_some() && job.target != Target::Bios,
    );
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    if let (Some(staged), Some(source)) = (&staged_wim, &iso) {
        super::wim::validate_fat_capacity(source, staged, parts[0].size)?;
    }

    ctx.bad_blocks(job)?;
    ctx.progress(0.0, "Unmounting...");
    let dev = ctx.open_unmounted(&job.drive)?;
    if !job.quick {
        zero_range(
            &dev,
            0,
            job.drive.size / part::SECTOR * part::SECTOR,
            Some(ctx),
        )?;
    }
    // Clear stale filesystem signatures at the start of every partition.
    for p in &parts {
        zero_range(&dev, p.start, p.size.min(MB), None).map_err(|error| {
            format!(
                "Clearing partition signatures on {}: {error}",
                job.drive.path
            )
        })?;
    }
    ctx.progress(0.0, format!("Partitioning ({:?})...", job.scheme));
    write_tables(job, &rep, sl.as_ref(), &parts, &dev, ctx)
        .map_err(|error| format!("Writing partition map on {}: {error}", job.drive.path))?;

    let needs_files = iso.is_some() || job.ext_label || job.boot == Boot::FreeDos;
    if fs.is_fat() {
        write_fat_replacing(
            job,
            &rep,
            iso.as_ref(),
            sl.as_ref(),
            &parts[0],
            &dev,
            ctx,
            replacement,
        )
        .map_err(|error| format!("Formatting/copying FAT on {}: {error}", job.drive.path))?;
    } else if let Some(kind) = ext_kind(fs) {
        if needs_files && !cfg!(target_os = "linux") {
            return Err(format!(
                "Copying files to {} needs Linux; use FAT32/NTFS/exFAT",
                fs.name()
            ));
        }
        ctx.progress(0.0, format!("Formatting ({})...", fs.name()));
        let mut vol = BlockDev::new(&dev, parts[0].start, parts[0].size);
        crate::ext::format(&mut vol, parts[0].size, kind, &label, job.cluster, &[])
            .map_err(|err| format!("Format failed: {err}"))?;
        vol.flush().map_err(e)?;
    }
    // Persistence is created by Rustfus' own ext formatter, so it works on every OS.
    if let Some(p) = parts.iter().find(|p| p.name == "persistence") {
        let (plabel, conf): (&str, &[(&str, &[u8])]) = if rep.casper {
            ("casper-rw", &[])
        } else {
            ("persistence", &[("persistence.conf", b"/ union\n")])
        };
        ctx.progress(0.0, "Creating persistent partition...");
        let mut vol = BlockDev::new(&dev, p.start, p.size);
        crate::ext::format(&mut vol, p.size, crate::ext::Kind::Ext3, plabel, 0, conf).map_err(e)?;
        vol.flush().map_err(e)?;
    }
    drop(dev);
    if fs.is_fat() {
        fsos::rescan(&job.drive).ok();
    } else {
        fsos::rescan(&job.drive)?;
        if ext_kind(fs).is_none() {
            ctx.progress(0.0, format!("Formatting ({})...", fs.name()));
            fsos::mkfs(&job.drive, 1, fs, &label, job.cluster)?;
        }
        let mut staged = None;
        if needs_files {
            let mnt = fsos::mount(&job.drive, 1, fs)?;
            let r = copy_files(job, &rep, iso.as_ref(), &DirDest::Dir(mnt.clone()), ctx).and_then(
                |()| match (iso.as_ref(), sl.as_ref()) {
                    (Some(iso), Some(sl)) => stage_syslinux(job, iso, sl, &mnt, ctx).map(Some),
                    _ => Ok(None),
                },
            );
            fsos::unmount(&job.drive, 1, &mnt)?;
            staged = r?;
        }
        if let (Some(sl), Some(staged)) = (&sl, staged) {
            // Patched through the raw device, now that the driver's cache is gone.
            let dev = open_dev(&job.drive)?;
            let mut vol = BlockDev::new(&dev, parts[0].start, parts[0].size);
            syslinux::install_staged(&mut vol, sl, staged, &|m| ctx.log(m))?;
            vol.flush().map_err(e)?;
        }
        if fs == Fs::Ntfs && rep.windows && job.target != Target::Uefi && !cfg!(windows) {
            // mkntfs boot code can't load BOOTMGR; Windows' own format already can.
            let dev = open_dev(&job.drive)?;
            bootrec::write_ntfs_pbr(&mut BlockDev::new(&dev, parts[0].start, parts[0].size))
                .map_err(e)?;
        }
    }

    Ok(format!(
        "{} ready ({} \"{label}\")",
        job.drive.path,
        fs.name()
    ))
}
