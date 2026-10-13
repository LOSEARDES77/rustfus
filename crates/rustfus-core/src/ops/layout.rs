// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! Partition layout, partition tables and boot records.

use super::*;

/// core.img matching the image's GRUB, fetched (signed) from rufus.ie like
/// Rufus does, trimming the version at '.'/'-' until the server has it.
pub(super) fn grub_core(rep: &Report, allow: bool, ctx: &Ctx) -> std::borrow::Cow<'static, [u8]> {
    let v = &rep.grub2_version;
    if v.is_empty() || v == GRUB2_VERSION {
        return GRUB2_CORE.into();
    }
    if allow {
        let cuts = (1..=v.len())
            .rev()
            .filter(|&i| i == v.len() || matches!(v.as_bytes()[i], b'.' | b'-'));
        for i in cuts {
            let ver = &v[..i];
            let url = format!("{}/grub-{ver}/core.img", crate::net::FILES_URL);
            match crate::net::fetch_signed(&url, &format!("grub-{ver}/core.img")) {
                Ok(data) => {
                    ctx.log(format!("Using GRUB {ver} core.img from {url}"));
                    return data.into();
                }
                Err(err) => ctx.log(format!("{url}: {err}")),
            }
        }
    }
    ctx.log(format!("WARNING: image uses GRUB {v} but the embedded core.img is {GRUB2_VERSION}; BIOS boot may fail"));
    GRUB2_CORE.into()
}

pub(super) fn layout(job: &Job, uefi_ntfs: bool) -> Vec<part::Part> {
    let gpt = job.scheme == Scheme::Gpt;
    let start = if job.old_bios && !gpt {
        63 * part::SECTOR
    } else {
        MB
    };
    let mut end = (job.drive.size - if gpt { 34 * part::SECTOR } else { 0 }) / MB * MB;
    let mut extra = Vec::new();
    if job.persistence > 0 {
        let size = job.persistence / MB * MB;
        end -= size;
        extra.push(part::Part {
            start: end,
            size,
            mbr_type: 0x83,
            gpt_type: part::GPT_LINUX,
            name: "persistence",
            active: false,
        });
    }
    if uefi_ntfs {
        let size = (UEFI_NTFS.len() as u64).next_multiple_of(MB);
        end -= size;
        for p in &mut extra {
            p.start -= size;
        }
        extra.push(part::Part {
            start: end + job.persistence / MB * MB,
            size,
            mbr_type: 0xEF,
            gpt_type: part::GPT_ESP,
            name: "UEFI:NTFS",
            active: false,
        });
        if job.persistence == 0 {
            extra.last_mut().unwrap().start = end;
        }
    }
    let bootable = job.boot != Boot::NonBootable && job.target != Target::Uefi;
    let main = part::Part {
        start,
        size: end - start,
        mbr_type: job.fs.mbr_type(),
        gpt_type: part::GPT_BASIC_DATA,
        name: "Main Data Partition",
        active: bootable && !gpt,
    };
    std::iter::once(main).chain(extra).collect()
}

pub(super) fn mbr_kind(job: &Job, rep: &Report, sl: Option<&syslinux::Info>) -> bootrec::Mbr {
    if job.boot == Boot::NonBootable || job.target == Target::Uefi {
        return bootrec::Mbr::Zero;
    }
    if rep.windows {
        return bootrec::Mbr::Win7;
    }
    // Rufus checks Syslinux before GRUB.
    if sl.is_some() {
        return bootrec::Mbr::Syslinux;
    }
    if rep.grub2 {
        return bootrec::Mbr::Grub2;
    }
    bootrec::Mbr::Rufus { bios_id: None }
}

pub(super) fn ext_kind(fs: Fs) -> Option<crate::ext::Kind> {
    match fs {
        Fs::Ext2 => Some(crate::ext::Kind::Ext2),
        Fs::Ext3 => Some(crate::ext::Kind::Ext3),
        Fs::Ext4 => Some(crate::ext::Kind::Ext4),
        _ => None,
    }
}

pub(super) fn fat_label(label: &str) -> [u8; 11] {
    let mut l = [b' '; 11];
    for (d, s) in l.iter_mut().zip(label.bytes()) {
        *d = s;
    }
    l
}

pub(super) fn zero_range(
    dev: &File,
    start: u64,
    len: u64,
    ctx: Option<&Ctx>,
) -> Result<(), String> {
    let buf = vec![0u8; CHUNK];
    let mut off = 0;
    while off < len {
        if let Some(c) = ctx {
            c.check()?;
            let pct = off as f32 / len as f32;
            c.progress(pct, format!("Zeroing drive: {:.1}%", pct * 100.0));
        }
        let n = (len - off).min(CHUNK as u64) as usize;
        write_at(dev, &buf[..n], start + off).map_err(e)?;
        off += n as u64;
    }
    Ok(())
}

/// Partition table, MBR boot code and the raw UEFI:NTFS image.
pub(super) fn write_tables(
    job: &Job,
    rep: &Report,
    sl: Option<&syslinux::Info>,
    parts: &[part::Part],
    dev: &File,
    ctx: &Ctx,
) -> Result<(), String> {
    let mut disk = BlockDev::new(dev, 0, job.drive.size / part::SECTOR * part::SECTOR);
    match job.scheme {
        Scheme::Mbr => part::write_mbr(&mut disk, job.drive.size, parts),
        Scheme::Gpt => part::write_gpt(&mut disk, job.drive.size, parts),
    }
    .map_err(e)?;
    if job.scheme == Scheme::Mbr {
        let kind = mbr_kind(job, rep, sl);
        bootrec::write_mbr(&mut disk, kind).map_err(e)?;
        // GRUB 2 core.img goes in the gap between the MBR and the first partition.
        if kind == bootrec::Mbr::Grub2 {
            let core = grub_core(rep, job.allow_downloads, ctx);
            if part::SECTOR + core.len() as u64 > parts[0].start {
                ctx.log("WARNING: no room for GRUB core.img; uncheck 'Add fixes for old BIOSes'");
            } else {
                disk.seek(SeekFrom::Start(part::SECTOR)).map_err(e)?;
                disk.write_all(&core).map_err(e)?;
            }
        }
    } else if job.boot != Boot::NonBootable {
        // Like Rufus: BIOS boot of GPT media prints why it can't work.
        bootrec::write_mbr(&mut disk, bootrec::Mbr::RufusMsg).map_err(e)?;
        let mut msg = bootrec::MSG_TEXT.to_vec();
        msg.resize(4096, 0);
        disk.seek(SeekFrom::Start(34 * part::SECTOR)).map_err(e)?;
        disk.write_all(&msg).map_err(e)?;
    }
    if let Some(p) = parts.iter().find(|p| p.name == "UEFI:NTFS") {
        ctx.log("Writing UEFI:NTFS partition");
        disk.seek(SeekFrom::Start(p.start)).map_err(e)?;
        disk.write_all(UEFI_NTFS).map_err(e)?;
    }
    disk.flush().map_err(e)
}
