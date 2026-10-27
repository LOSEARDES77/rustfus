// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Ported from Syslinux libinstaller (via Rufus): Copyright © 1998-2008 H. Peter Anvin,
// Copyright © 2009-2014 Intel Corporation, Copyright © 2003 Lars Munch Christensen,
// Copyright © 2012-2024 Pete Batard (GPL-2.0-or-later).
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! Syslinux support for ISO mode on FAT, ported from Rufus' syslinux.c/iso.c
//! and Syslinux' libinstaller (syslxmod.c, fs.c, setadv.c).

use crate::bytes::{get16, get32, put16, put32, put64};
use std::io::{Read, Seek, SeekFrom, Write};

const V4_SYS: &[u8] = include_bytes!("../../../res/syslinux/ldlinux_v4.sys");
const V4_BSS: &[u8] = include_bytes!("../../../res/syslinux/ldlinux_v4.bss");
const V6_SYS: &[u8] = include_bytes!("../../../res/syslinux/ldlinux_v6.sys");
const V6_BSS: &[u8] = include_bytes!("../../../res/syslinux/ldlinux_v6.bss");
const MBOOT_C32: &[u8] = include_bytes!("../../../res/syslinux/mboot.c32");

const SECTOR: usize = 512;
const ADV_SIZE: usize = 512;
const LDLINUX_MAGIC: u32 = 0x3eb2_02fe;
const CFG_NAMES: [&str; 3] = ["isolinux.cfg", "syslinux.cfg", "extlinux.conf"];
const OLD_C32: [(&str, u64); 2] = [("menu.c32", 53500), ("vesamenu.c32", 148000)];

#[derive(Debug, Clone)]
pub struct Info {
    pub major: u8,
    pub minor: u8,
    /// "6.03"
    pub version: String,
    /// Extra version, slash-prefixed ("/2014-10-06") or empty.
    pub ext: String,
    /// Config file Syslinux should use ("/isolinux/isolinux.cfg"), empty for ReactOS.
    pub cfg_path: String,
    /// The ISO's own /syslinux.cfg is broken (openSUSE) and must be replaced.
    pub overwrite_cfg: bool,
    /// ReactOS loader path when the ISO is ReactOS rather than Syslinux-based.
    pub reactos: Option<String>,
    /// Obsolete menu.c32/vesamenu.c32 to replace (Syslinux < 5 only).
    pub old_c32: Vec<String>,
}

/// C strtoul on a NUL-terminated buffer: (value, end index).
fn strtoul(b: &[u8], mut i: usize) -> (u64, usize) {
    let start = i;
    while matches!(b[i], b' ' | b'\t'..=b'\r') {
        i += 1;
    }
    if matches!(b[i], b'+' | b'-') {
        i += 1;
    }
    let digits = i;
    let mut v = 0u64;
    while b[i].is_ascii_digit() {
        v = v.saturating_mul(10).saturating_add((b[i] - b'0') as u64);
        i += 1;
    }
    if i == digits { (0, start) } else { (v, i) }
}

/// Rufus' GetSyslinuxVersion: finds "ISOLINUX x.yz ..."/"SYSLINUX x.yz ..."
/// and returns ((major << 8) | minor, extra version).
fn sl_version(buf: &[u8]) -> Option<(u16, String)> {
    if buf.len() < 256 {
        return None;
    }
    let mut b = buf.to_vec();
    b.push(0);
    for start in 64..buf.len() - 64 {
        if &b[start..start + 6] != b"LINUX " || !matches!(&b[start - 3..start], b"ISO" | b"SYS") {
            continue;
        }
        let i = start + 6;
        let (major, p) = strtoul(&b, i);
        if major >= 256 {
            continue;
        }
        let (minor, mut p) = strtoul(&b, p + 1);
        let version = if minor < 256 {
            ((major << 8) + minor) as u16
        } else {
            0
        };
        if version == 0 {
            continue;
        }
        b[p] = b'/';
        let mut j = 0;
        while p + 1 + j < b.len() && b[i + j] == b[p + 1 + j] && b[i + j] != b' ' && b[i + j] != 0 {
            j += 1;
        }
        if b[p + j + 1] == b'-' {
            j += 1;
        }
        if j >= 4 {
            b[p + j] = b'/';
            p += j;
        }
        let end = p + b[p..]
            .iter()
            .position(|&c| c == 0)
            .expect("`b` ends with a pushed NUL");
        let mut s = b[p..end].to_vec();
        while s.len() > 1 && matches!(s.last(), Some(b' ' | b'*')) {
            s.pop();
        }
        for c in &mut s[1..] {
            if b"<>:|*?\\/".contains(c) {
                *c = b'_';
            }
        }
        let ext = if s.len() == 1 {
            String::new()
        } else {
            String::from_utf8_lossy(&s).into_owned()
        };
        return Some((version, ext));
    }
    None
}

fn embedded(v6: bool) -> (u16, String) {
    sl_version(if v6 { V6_SYS } else { V4_SYS }).expect("embedded ldlinux.sys has a version")
}

fn info(version: u16, ext: String) -> Info {
    let (major, minor) = ((version >> 8) as u8, version as u8);
    Info {
        major,
        minor,
        version: format!("{major}.{minor:02}"),
        ext,
        cfg_path: String::new(),
        overwrite_cfg: false,
        reactos: None,
        old_c32: Vec::new(),
    }
}

fn base(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

fn read_entry(iso: &crate::iso::Iso, e: &crate::iso::Entry) -> std::io::Result<Vec<u8>> {
    let mut v = Vec::new();
    iso.reader(e)?.read_to_end(&mut v)?;
    Ok(v)
}

/// Detects Syslinux (or ReactOS, which Rufus boots through Syslinux v4) like Rufus' ISO scan.
pub fn detect(iso: &crate::iso::Iso) -> Option<Info> {
    let entries = iso.entries().ok()?;
    let files: Vec<_> = entries
        .iter()
        .filter(|e| !e.is_dir && e.symlink().is_none())
        .collect();
    let is = |e: &crate::iso::Entry, names: &[&str]| {
        names.iter().any(|n| base(&e.path).eq_ignore_ascii_case(n))
    };
    let has_ldlinux_c32 = files.iter().any(|e| is(e, &["ldlinux.c32"]));
    let configs: Vec<String> = files
        .iter()
        .filter(|e| is(e, &CFG_NAMES))
        .map(|e| format!("/{}", e.path))
        .collect();
    let old_c32 = files
        .iter()
        .filter(|e| {
            OLD_C32
                .iter()
                .any(|(n, max)| base(&e.path).eq_ignore_ascii_case(n) && e.size <= *max)
        })
        .map(|e| e.path.clone())
        .collect();

    if configs.is_empty() {
        let reactos = files
            .iter()
            .find(|e| is(e, &["setupldr.sys", "freeldr.sys"]))?;
        let (v, ext) = embedded(false);
        return Some(Info {
            reactos: Some(format!("/{}", reactos.path)),
            ..info(v, ext)
        });
    }
    let (mut cfg, mut overwrite) = (None::<&String>, false);
    for c in &configs {
        if c.eq_ignore_ascii_case("/boot/i386/loader/isolinux.cfg")
            || c.eq_ignore_ascii_case("/boot/x86_64/loader/isolinux.cfg")
        {
            (cfg, overwrite) = (Some(c), true);
            break;
        }
        if cfg.is_none_or(|p| p.len() >= c.len()) {
            cfg = Some(c);
        }
    }
    let cfg = cfg.expect("`configs` is not empty").clone();
    let cfg_dir = &cfg[..cfg.rfind('/').unwrap_or(0)];

    let mut found = (0u16, String::new());
    for e in files
        .iter()
        .filter(|e| is(e, &["isolinux.bin", "boot.bin"]))
    {
        let Ok(buf) = read_entry(iso, e) else {
            continue;
        };
        let v = sl_version(&buf).unwrap_or((0, String::new()));
        // On conflicting versions, prefer the one next to the config file (Antergos).
        let same_dir = format!("/{}", e.path)
            .as_bytes()
            .get(..cfg_dir.len())
            .is_some_and(|p| p.eq_ignore_ascii_case(cfg_dir.as_bytes()));
        if found.0 == 0 || (found != v && same_dir) {
            found = v;
        }
    }
    if found.0 == 0 {
        found = (embedded(has_ldlinux_c32).0, String::new());
    }
    let (v, ext) = found;
    Some(Info {
        cfg_path: cfg,
        overwrite_cfg: overwrite,
        old_c32,
        ..info(v, ext)
    })
}

/// Files to write to the FAT root after copying the ISO (overwriting same-named copies).
/// Of `info.old_c32`, only the paths in `replace` (the ones the user agreed to download
/// newer copies of, as with Rufus' "Replace menu.c32?" prompt) are replaced.
pub fn extra_files(
    iso: &crate::iso::Iso,
    info: &Info,
    replace: &[String],
    log: &dyn Fn(String),
) -> Result<Vec<(String, Vec<u8>)>, String> {
    if let Some(r) = &info.reactos {
        let cfg = format!("DEFAULT ReactOS\nLABEL ReactOS\n  KERNEL mboot.c32\n  APPEND {r}");
        return Ok(vec![
            ("mboot.c32".into(), MBOOT_C32.to_vec()),
            ("syslinux.cfg".into(), cfg.into_bytes()),
        ]);
    }
    let mut out = Vec::new();
    let entries = iso.entries().map_err(|e| e.to_string())?;
    let existing = entries
        .iter()
        .find(|e| !e.is_dir && e.path.eq_ignore_ascii_case("syslinux.cfg"));
    if let Some(e) = existing
        && info.overwrite_cfg
    {
        log("Renaming: syslinux.cfg ➔ syslinux.org".into());
        out.push((
            "syslinux.org".into(),
            read_entry(iso, e).map_err(|e| e.to_string())?,
        ));
    }
    if existing.is_none() || info.overwrite_cfg {
        let mut cfg = format!(
            "DEFAULT loadconfig\n\nLABEL loadconfig\n  CONFIG {}\n",
            info.cfg_path
        );
        if let Some(i) = info.cfg_path.rfind('/').filter(|&i| i > 0) {
            cfg += &format!("  APPEND {}/\n", &info.cfg_path[..i]);
        }
        log(format!("Created: syslinux.cfg → {}", info.cfg_path));
        out.push(("syslinux.cfg".into(), cfg.into_bytes()));
    }
    if info.major < 5 {
        let ver = embedded(false).0;
        let ver = format!("{}.{:02}", ver >> 8, ver & 0xff);
        for path in &info.old_c32 {
            if !replace.contains(path) {
                log(format!("Keeping the obsolete '{path}' from the ISO"));
                continue;
            }
            let name = base(path).to_ascii_lowercase();
            let rel = format!("syslinux-{ver}/{name}");
            match crate::net::fetch_signed(&format!("{}/{rel}", crate::net::FILES_URL), &rel) {
                Ok(data) => {
                    log(format!(
                        "Replacing obsolete '{path}' with the one from Syslinux {ver}"
                    ));
                    out.push((path.clone(), data));
                }
                Err(e) => log(format!(
                    "Could not download {name} ({e}); keeping the obsolete one from the ISO"
                )),
            }
        }
    }
    Ok(out)
}

/// ldlinux.sys and ldlinux.bss for `info`: embedded v4 for Syslinux < 5, embedded v6
/// for an exact version match, otherwise downloaded from Rufus' server (cached).
pub fn loader(info: &Info, log: &dyn Fn(String)) -> Result<(Vec<u8>, Vec<u8>), String> {
    if info.major < 5 {
        return Ok((V4_SYS.to_vec(), V4_BSS.to_vec()));
    }
    let emb = embedded(true);
    let version = ((info.major as u16) << 8) | info.minor as u16;
    if (version, info.ext.as_str()) == (emb.0, emb.1.as_str()) {
        return Ok((V6_SYS.to_vec(), V6_BSS.to_vec()));
    }
    let get = |ext: &str, file: &str| {
        let rel = format!("syslinux-{}{ext}/{file}", info.version);
        crate::net::fetch_signed(&format!("{}/{rel}", crate::net::FILES_URL), &rel)
    };
    let mut files = Vec::new();
    for file in ["ldlinux.sys", "ldlinux.bss"] {
        let mut r = get(&info.ext, file);
        if r.is_err() && !info.ext.is_empty() {
            log("Extended version was not found, trying main version...".into());
            r = get("", file);
        }
        match r {
            Ok(d) => files.push(d),
            Err(e) if version == emb.0 => {
                log(format!(
                    "Could not download the file ({e}) - will try to use embedded {} version instead",
                    info.version
                ));
                return Ok((V6_SYS.to_vec(), V6_BSS.to_vec()));
            }
            Err(e) => return Err(format!("Syslinux {}{}: {e}", info.version, info.ext)),
        }
    }
    let [sys, bss]: [Vec<u8>; 2] = files
        .try_into()
        .expect("the loop downloads exactly ldlinux.sys and ldlinux.bss");
    Ok((sys, bss))
}

/// libinstaller check_fat_bootsect + the media/sector-size checks of syslinux_check_bootsect.
fn check_bootsect(b: &[u8]) -> Result<(), &'static str> {
    if b[21] != 0xf0 && b[21] < 0xf8 {
        return Err("invalid media signature (not an FAT/NTFS volume?)");
    }
    let sectorsize = get16(b, 11) as i64;
    if sectorsize != SECTOR as i64 {
        return Err(
            if (512..=4096).contains(&sectorsize) && sectorsize & (sectorsize - 1) == 0 {
                "unsupported sectors size"
            } else {
                "impossible sector size"
            },
        );
    }
    let clustersize = b[13] as i64;
    if clustersize == 0 || clustersize & (clustersize - 1) != 0 {
        return Err("impossible cluster size on an FAT volume");
    }
    let sectors = match get16(b, 19) {
        0 => get32(b, 32) as i64,
        s => s as i64,
    };
    let fatsectors = match get16(b, 22) {
        0 => get32(b, 36) as i64,
        s => s as i64,
    } * b[16] as i64;
    let rootdirents = get16(b, 17) as i64;
    let dsectors = sectors
        - get16(b, 14) as i64
        - fatsectors
        - (rootdirents + sectorsize / 32 - 1) / sectorsize;
    if dsectors < 0 {
        return Err("negative number of data sectors on an FAT volume");
    }
    if fatsectors == 0 {
        return Err("zero FAT sectors");
    }
    let clusters = dsectors / clustersize;
    let fstype = |at: usize| &b[at..at + 8];
    if clusters < 0xfff5 {
        if get16(b, 22) == 0 {
            return Err("zero FAT sectors (FAT12/16)");
        }
        if b[38] == 0x29 {
            match fstype(54) {
                b"FAT12   " if clusters >= 0xff5 => {
                    return Err("more than 4084 clusters but claims FAT12");
                }
                b"FAT16   " if clusters < 0xff5 => {
                    return Err("less than 4084 clusters but claims FAT16");
                }
                b"FAT32   " => return Err("less than 65525 clusters but claims FAT32"),
                b"FAT12   " | b"FAT16   " | b"FAT     " => {}
                _ => return Err("filesystem type not supported"),
            }
        }
    } else if clusters < 0x0fff_fff5 {
        if b[66] != 0x29 || fstype(82) != b"FAT32   " {
            return Err("missing FAT32 signature");
        }
    } else {
        return Err("impossibly large number of clusters on an FAT volume");
    }
    Ok(())
}

/// setadv.c syslinux_reset_adv: two copies of an empty ADV.
fn empty_adv() -> Vec<u8> {
    let mut adv = vec![0u8; 2 * ADV_SIZE];
    put32(&mut adv, 0, 0x5a2d_2fa5);
    put32(&mut adv, 4, 0xa304_1767); // MAGIC2 minus the sum of an all-zero payload
    put32(&mut adv, ADV_SIZE - 4, 0xdd28_bf64);
    adv.copy_within(..ADV_SIZE, ADV_SIZE);
    adv
}

/// syslxmod.c generate_extents.
fn generate_extents(
    img: &mut [u8],
    at: usize,
    nptrs: usize,
    sectors: &[u64],
) -> Result<(), String> {
    img.get_mut(at..at + nptrs * 10)
        .ok_or("bad extent table")?
        .fill(0);
    let mut out = Vec::new();
    let (mut addr, mut base, mut lba, mut len) = (0x8000u32, 0x8000u32, 0u64, 0u32);
    for &sect in sectors {
        if len != 0 {
            let xbytes = (len + 1) * SECTOR as u32;
            if sect == lba + len as u64
                && xbytes < 65536
                && (addr ^ (base + xbytes - 1)) & 0xffff_0000 == 0
            {
                len += 1;
                addr += SECTOR as u32;
                continue;
            }
            out.push((lba, len));
        }
        (base, lba, len) = (addr, sect, 1);
        addr += SECTOR as u32;
    }
    if len != 0 {
        out.push((lba, len));
    }
    if out.len() > nptrs {
        return Err("ldlinux.sys is too fragmented".into());
    }
    for (k, (lba, len)) in out.into_iter().enumerate() {
        put64(img, at + k * 10, lba);
        put16(img, at + k * 10 + 8, len as u16);
    }
    Ok(())
}

/// syslxmod.c syslinux_patch (stupid = raid = 0, no subvol). `img` must be
/// sector-padded; patches `img` and the sector-1 pointer in `bss`.
fn patch(img: &mut [u8], bss: &mut [u8], sectors: &[u64], subdir: &str) -> Result<(), String> {
    let nsect = img.len().div_ceil(SECTOR) + 2;
    if sectors.len() < nsect {
        return Err("ldlinux.sys is too small for its content".into());
    }
    let pa = (0..=(img.len() - 24) / 4)
        .map(|i| i * 4)
        .find(|&o| get32(img, o) == LDLINUX_MAGIC)
        .ok_or("no LDLINUX_MAGIC in ldlinux.sys")?;
    let epa = get16(img, pa + 22) as usize;
    let field = |at: usize| get16(img, epa + at) as usize;
    let (advptr, diroff, dirlen, secptr, secptrcnt, s1p0, s1p1) = (
        field(0),
        field(2),
        field(4),
        field(10),
        field(12),
        field(14),
        field(16),
    );
    if epa + 20 > img.len()
        || s1p0.max(s1p1) + 4 > bss.len()
        || advptr + 16 > img.len()
        || diroff + dirlen > img.len()
    {
        return Err("corrupt ldlinux.sys patch area".into());
    }
    put32(bss, s1p0, sectors[0] as u32);
    put32(bss, s1p1, (sectors[0] >> 32) as u32);
    let sectors = &sectors[1..];

    let dw = img.len() / 4;
    put16(img, pa + 8, (nsect - 2) as u16);
    put16(img, pa + 10, 2);
    put32(img, pa + 12, dw as u32);
    generate_extents(img, secptr, secptrcnt, &sectors[..nsect - 3])?;
    put64(img, advptr, sectors[nsect - 3]);
    put64(img, advptr + 8, sectors[nsect - 2]);

    let sub = subdir.as_bytes();
    if dirlen < sub.len() + 1 {
        return Err("syslinux_patch: Subdirectory path too long".into());
    }
    img[diroff..diroff + sub.len()].copy_from_slice(sub);
    img[diroff + sub.len()] = 0;

    put32(img, pa + 16, 0);
    let csum = (0..dw).fold(LDLINUX_MAGIC, |c, i| c.wrapping_sub(get32(img, i * 4)));
    put32(img, pa + 16, csum);
    Ok(())
}

/// Installs ldlinux.sys and the Syslinux boot sector on a formatted, populated FAT volume.
pub fn install<F: Read + Write + Seek>(
    vol: &mut F,
    info: &Info,
    log: &dyn Fn(String),
) -> Result<(), String> {
    let e = |err: std::io::Error| format!("Syslinux install: {err}");
    log(format!(
        "Installing Syslinux {}{}...",
        info.version, info.ext
    ));
    let (sys, mut bss) = loader(info, log)?;
    if bss.len() < SECTOR {
        return Err("ldlinux.bss is too small".into());
    }
    let mut bs = [0u8; SECTOR];
    vol.seek(SeekFrom::Start(0)).map_err(e)?;
    vol.read_exact(&mut bs).map_err(e)?;
    check_bootsect(&bs).map_err(|m| format!("Syslinux install: {m}"))?;

    // Upstream's installer pads the image to whole sectors so the ADV is sector-aligned.
    let mut img = sys;
    img.resize(img.len().div_ceil(SECTOR) * SECTOR, 0);
    {
        vol.seek(SeekFrom::Start(0)).map_err(e)?;
        let fs = fatfs::FileSystem::new(&mut *vol, fatfs::FsOptions::new()).map_err(e)?;
        fs.root_dir().remove("ldlinux.sys").ok();
        let mut f = fs.root_dir().create_file("ldlinux.sys").map_err(e)?;
        f.write_all(&img).map_err(e)?;
        f.write_all(&empty_adv()).map_err(e)?;
        f.flush().map_err(e)?;
        drop(f);
        fs.unmount().map_err(e)?;
    }
    let want = (img.len() + 2 * ADV_SIZE).div_ceil(SECTOR);
    let (entry, sectors) = crate::fat::root_file(vol, b"LDLINUX SYS", want)
        .map_err(e)?
        .ok_or("ldlinux.sys not found after writing it")?;

    // Read-only, hidden, system (fatfs can't set attributes).
    let mut attr = [0u8];
    vol.seek(SeekFrom::Start(entry + 11)).map_err(e)?;
    vol.read_exact(&mut attr).map_err(e)?;
    attr[0] |= 0x07;
    vol.seek(SeekFrom::Start(entry + 11)).map_err(e)?;
    vol.write_all(&attr).map_err(e)?;

    let subdir = match info.cfg_path.rfind('/') {
        Some(i) if i > 0 => &info.cfg_path[..i],
        _ => info.cfg_path.as_str(),
    };
    patch(&mut img, &mut bss, &sectors, subdir)?;
    for (chunk, &s) in img.chunks(SECTOR).zip(&sectors) {
        vol.seek(SeekFrom::Start(s * SECTOR as u64)).map_err(e)?;
        vol.write_all(chunk).map_err(e)?;
    }

    // syslinux_make_bootsect: jump + OEM name and the code area from the .bss, BPB kept.
    bs[..11].copy_from_slice(&bss[..11]);
    bs[90..510].copy_from_slice(&bss[90..510]);
    vol.seek(SeekFrom::Start(0)).map_err(e)?;
    vol.write_all(&bs).map_err(e)?;
    vol.flush().map_err(e)?;
    vol.seek(SeekFrom::Start(0)).map_err(e)?;
    log("Successfully wrote Syslinux boot record".into());
    Ok(())
}

#[cfg(test)]
#[path = "../tests/unit/syslinux.rs"]
mod tests;
