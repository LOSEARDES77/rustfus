// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! ISO mode / plain format onto FAT, ext or an OS-created filesystem.

use super::files::{Dest, DirDest, copy_files};
use super::layout::{ext_kind, fat_label, layout, write_tables, zero_range};
use super::{
    Boot, Ctx, Fs, Job, MB, Report, Scheme, Target, analyze, e, open_dev, sanitize_label, unmount,
};
use crate::blockdev::BlockDev;
use crate::{bootrec, fsos, iso, part, syslinux};
use std::fs::File;
use std::io::{Seek, SeekFrom, Write};

/// FAT/FAT32 path: Rustfus formats and fills the partition itself.
pub(super) fn write_fat(
    job: &Job,
    rep: &Report,
    iso: Option<&iso::Iso>,
    sl: Option<&syslinux::Info>,
    main: &part::Part,
    dev: &File,
    ctx: &Ctx,
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
        copy_files(job, rep, iso, &dest, ctx)?;
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
        syslinux::install(&mut vol, sl, &|m| ctx.log(m))?;
    } else if job.fs == Fs::Fat32 && job.target != Target::Uefi {
        if job.boot == Boot::FreeDos {
            bootrec::write_fat32_freedos_pbr(&mut vol).map_err(e)?;
        } else if rep.windows {
            bootrec::write_fat32_pe_pbr(&mut vol).map_err(e)?;
        }
    }
    vol.flush().map_err(e)
}

/// ISO mode / format: partition, create the filesystem, copy files, install boot code.
pub(super) fn write_fs(job: &Job, ctx: &Ctx) -> Result<String, String> {
    let (iso, rep) = match &job.boot {
        Boot::Image(p) => (Some(iso::Iso::open(p).map_err(e)?), analyze(p)?),
        _ => (None, Report::default()),
    };
    let fs = job.fs;
    if fs.is_fat() && rep.large_file {
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
        && let Some(tool) = fsos::missing_tool(fs)
    {
        return Err(format!(
            "Formatting {} needs {tool}, which is not installed",
            fs.name()
        ));
    }
    let label = sanitize_label(&job.label, fs);
    // Like Rufus: Syslinux only for FAT, non-Windows, BIOS-capable targets.
    let sl = iso
        .as_ref()
        .filter(|_| {
            fs.is_fat() && !rep.windows && job.target != Target::Uefi && job.scheme == Scheme::Mbr
        })
        .and_then(syslinux::detect);
    if let Some(sl) = &sl {
        ctx.log(format!("Syslinux {}{} detected", sl.version, sl.ext));
        // Fetch a non-embedded loader now so a download failure leaves the drive untouched.
        syslinux::loader(sl, &|m| ctx.log(m))?;
    }
    let parts = layout(
        job,
        !fs.is_fat() && iso.is_some() && job.target != Target::Bios,
    );

    ctx.progress(0.0, "Unmounting...");
    unmount(&job.drive)?;
    let dev = open_dev(&job.drive)?;
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
        zero_range(&dev, p.start, p.size.min(MB), None)?;
    }
    ctx.progress(0.0, format!("Partitioning ({:?})...", job.scheme));
    write_tables(job, &rep, sl.as_ref(), &parts, &dev, ctx)?;

    let needs_files = iso.is_some() || job.ext_label || job.boot == Boot::FreeDos;
    if fs.is_fat() {
        write_fat(job, &rep, iso.as_ref(), sl.as_ref(), &parts[0], &dev, ctx)?;
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
        if needs_files {
            let mnt = fsos::mount(&job.drive, 1, fs)?;
            let r = copy_files(job, &rep, iso.as_ref(), &DirDest::Dir(mnt.clone()), ctx);
            fsos::unmount(&job.drive, 1, &mnt)?;
            r?;
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
