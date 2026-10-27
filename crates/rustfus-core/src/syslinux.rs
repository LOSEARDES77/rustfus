// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Ported from Syslinux libinstaller (via Rufus): Copyright © 1998-2008 H. Peter Anvin,
// Copyright © 2009-2014 Intel Corporation, Copyright © 2003 Lars Munch Christensen,
// Copyright © 2012-2024 Pete Batard (GPL-2.0-or-later).
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! Syslinux support for ISO mode on FAT and NTFS, ported from Rufus' syslinux.c/iso.c
//! and Syslinux' libinstaller (syslxmod.c, syslxcom.c, fs.c, setadv.c).

use crate::bytes::{get16, get32, put16, put32, put64};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

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
/// Configs Knoppix' EFI Syslinux looks for in /boot/syslinux.
const KNOPPIX_EFI_CFGS: [&str; 2] = ["syslnx32.cfg", "syslnx64.cfg"];

/// Volume types the Syslinux boot sector can load ldlinux.sys from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VolumeKind {
    Fat,
    Ntfs,
}

/// ldlinux.sys written through the OS filesystem driver, waiting to be patched
/// in place once the volume is unmounted.
#[derive(Debug)]
pub struct Staged {
    /// Sector-padded ldlinux.sys, without the ADV.
    img: Vec<u8>,
    bss: Vec<u8>,
    /// Volume-relative 512-byte sectors of ldlinux.sys, ADV included.
    sectors: Vec<u64>,
}

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

/// Rufus' Knoppix workaround: Knoppix ships /boot/syslinux as a Rock Ridge
/// symlink to `isolinux`, which FAT can't store, yet its EFI Syslinux reads
/// syslnx32.cfg/syslnx64.cfg from there. Returns None when `link` isn't such a
/// symlink, else the configs to write in a real `link` directory, each loading
/// the ISO's own one. `find` looks a path up case-insensitively in the ISO.
pub fn knoppix_redirects(
    link: &str,
    target: &str,
    find: impl Fn(&str) -> Option<String>,
) -> Option<Vec<(String, Vec<u8>)>> {
    if !base(link).eq_ignore_ascii_case("syslinux") || !target.eq_ignore_ascii_case("isolinux") {
        return None;
    }
    let isolinux_dir = format!("{}isolinux", &link[..link.len() - "syslinux".len()]);
    let mut out = Vec::new();
    for name in KNOPPIX_EFI_CFGS {
        let Some(cfg) = find(&format!("{isolinux_dir}/{name}")) else {
            continue;
        };
        let dir = cfg.rsplit_once('/').map_or("", |(dir, _)| dir);
        let data =
            format!("DEFAULT loadconfig\n\nLABEL loadconfig\n  CONFIG /{cfg}\n  APPEND /{dir}\n");
        out.push((format!("{link}/{name}"), data.into_bytes()));
    }
    Some(out)
}

/// The embedded Syslinux 5+ loader's version ("6.04/pre1"), the one images of
/// other versions need a download to replace.
pub fn embedded_version() -> String {
    let (v, ext) = embedded(true);
    format!("{}.{:02}{ext}", v >> 8, v & 0xff)
}

/// Whether `info` is exactly the embedded Syslinux 5+ loader's version.
fn is_embedded_v6(info: &Info) -> bool {
    let emb = embedded(true);
    let version = ((info.major as u16) << 8) | info.minor as u16;
    (version, info.ext.as_str()) == (emb.0, emb.1.as_str())
}

/// Server/cache path of `file` for Syslinux `version` + `ext`.
fn loader_rel(version: &str, ext: &str, file: &str) -> String {
    format!("syslinux-{version}{ext}/{file}")
}

/// Whether [`loader`] would have to download ldlinux.sys and ldlinux.bss for
/// `info`, so the user must be asked first like Rufus' "Download required".
pub fn needs_download(info: &Info) -> bool {
    if info.major < 5 || is_embedded_v6(info) {
        return false;
    }
    let cached = |ext: &str| {
        ["ldlinux.sys", "ldlinux.bss"]
            .iter()
            .all(|f| crate::net::cached_signed(&loader_rel(&info.version, ext, f)).is_some())
    };
    !cached(&info.ext) && (info.ext.is_empty() || !cached(""))
}

/// ldlinux.sys and ldlinux.bss for `info`: embedded v4 for Syslinux < 5, embedded v6
/// for an exact version match, otherwise from Rufus' server (cached). Without
/// `allow_download`, only cached copies are used.
pub fn loader(
    info: &Info,
    allow_download: bool,
    log: &dyn Fn(String),
) -> Result<(Vec<u8>, Vec<u8>), String> {
    if info.major < 5 {
        return Ok((V4_SYS.to_vec(), V4_BSS.to_vec()));
    }
    if is_embedded_v6(info) {
        return Ok((V6_SYS.to_vec(), V6_BSS.to_vec()));
    }
    let emb = embedded(true);
    let version = ((info.major as u16) << 8) | info.minor as u16;
    let get = |ext: &str, file: &str| {
        let rel = loader_rel(&info.version, ext, file);
        if allow_download {
            crate::net::fetch_signed(&format!("{}/{rel}", crate::net::FILES_URL), &rel)
        } else {
            crate::net::cached_signed(&rel)
                .ok_or_else(|| format!("{file} must be downloaded, which was not allowed"))
        }
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

/// libinstaller ntfs_check_sb_fields: fields NTFS keeps zero plus a known OEM name.
fn is_ntfs_bootsect(b: &[u8]) -> bool {
    b[16..19] == [0; 3]
        && get16(b, 19) == 0
        && get16(b, 22) == 0
        && get32(b, 32) == 0
        && matches!(&b[3..11], b"NTFS    " | b"MSWIN4.0" | b"MSWIN4.1")
}

/// libinstaller syslinux_check_bootsect (check_fat_bootsect for FAT).
fn check_bootsect(b: &[u8]) -> Result<VolumeKind, &'static str> {
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
    if is_ntfs_bootsect(b) {
        return Ok(VolumeKind::Ntfs);
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
    Ok(VolumeKind::Fat)
}

/// syslxcom.c syslinux_make_bootsect: jump (+ OEM name on FAT) and the code area
/// come from the .bss, the BPB is kept.
fn make_bootsect(bs: &mut [u8], bss: &[u8], kind: VolumeKind) {
    let (head, code) = match kind {
        VolumeKind::Fat => (11, 90),
        VolumeKind::Ntfs => (3, 80),
    };
    bs[..head].copy_from_slice(&bss[..head]);
    bs[code..510].copy_from_slice(&bss[code..510]);
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

/// ldlinux.sys and .bss for `info`, the image padded to whole sectors so the
/// ADV that follows it is sector-aligned, as upstream's installer does.
fn padded_loader(
    info: &Info,
    allow_download: bool,
    log: &dyn Fn(String),
) -> Result<(Vec<u8>, Vec<u8>), String> {
    let (mut img, bss) = loader(info, allow_download, log)?;
    if bss.len() < SECTOR {
        return Err("ldlinux.bss is too small".into());
    }
    img.resize(img.len().div_ceil(SECTOR) * SECTOR, 0);
    Ok((img, bss))
}

/// Sectors ldlinux.sys spans once the ADV is appended to `img`.
fn ldlinux_sectors(img: &[u8]) -> usize {
    (img.len() + 2 * ADV_SIZE).div_ceil(SECTOR)
}

/// The directory Syslinux searches for its config, patched into ldlinux.sys.
fn subdir(info: &Info) -> &str {
    match info.cfg_path.rfind('/') {
        Some(i) if i > 0 => &info.cfg_path[..i],
        _ => info.cfg_path.as_str(),
    }
}

/// Patches ldlinux.sys for its sectors, writes it there and points the boot
/// sector `bs` (read from `vol`) at it.
fn write_patched<F: Write + Seek>(
    vol: &mut F,
    info: &Info,
    staged: Staged,
    mut bs: [u8; SECTOR],
    kind: VolumeKind,
    log: &dyn Fn(String),
) -> Result<(), String> {
    let e = |err: io::Error| format!("Syslinux install: {err}");
    let Staged {
        mut img,
        mut bss,
        sectors,
    } = staged;
    patch(&mut img, &mut bss, &sectors, subdir(info))?;
    for (chunk, &s) in img.chunks(SECTOR).zip(&sectors) {
        vol.seek(SeekFrom::Start(s * SECTOR as u64)).map_err(e)?;
        vol.write_all(chunk).map_err(e)?;
    }
    make_bootsect(&mut bs, &bss, kind);
    vol.seek(SeekFrom::Start(0)).map_err(e)?;
    vol.write_all(&bs).map_err(e)?;
    vol.flush().map_err(e)?;
    vol.seek(SeekFrom::Start(0)).map_err(e)?;
    log("Successfully wrote Syslinux boot record".into());
    Ok(())
}

fn read_bootsect<F: Read + Seek>(vol: &mut F) -> Result<([u8; SECTOR], VolumeKind), String> {
    let mut bs = [0u8; SECTOR];
    vol.seek(SeekFrom::Start(0))
        .and_then(|_| vol.read_exact(&mut bs))
        .map_err(|err| format!("Syslinux install: {err}"))?;
    let kind = check_bootsect(&bs).map_err(|m| format!("Syslinux install: {m}"))?;
    Ok((bs, kind))
}

/// Installs ldlinux.sys and the Syslinux boot sector on a formatted, populated FAT volume.
/// `allow_download` is as for [`loader`].
pub fn install<F: Read + Write + Seek>(
    vol: &mut F,
    info: &Info,
    allow_download: bool,
    log: &dyn Fn(String),
) -> Result<(), String> {
    let e = |err: io::Error| format!("Syslinux install: {err}");
    log(format!(
        "Installing Syslinux {}{}...",
        info.version, info.ext
    ));
    let (img, bss) = padded_loader(info, allow_download, log)?;
    let (bs, kind) = read_bootsect(vol)?;
    if kind != VolumeKind::Fat {
        return Err("Syslinux install: not a FAT volume".into());
    }
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
    let (entry, sectors) = crate::fat::root_file(vol, b"LDLINUX SYS", ldlinux_sectors(&img))
        .map_err(e)?
        .ok_or("ldlinux.sys not found after writing it")?;

    // Read-only, hidden, system (fatfs can't set attributes).
    let mut attr = [0u8];
    vol.seek(SeekFrom::Start(entry + 11)).map_err(e)?;
    vol.read_exact(&mut attr).map_err(e)?;
    attr[0] |= 0x07;
    vol.seek(SeekFrom::Start(entry + 11)).map_err(e)?;
    vol.write_all(&attr).map_err(e)?;

    let staged = Staged { img, bss, sectors };
    write_patched(vol, info, staged, bs, kind, log)
}

/// First step of installing Syslinux on a volume Rustfus can't write itself
/// (NTFS): writes ldlinux.sys under the mounted `root` and maps its sectors.
/// Finish with [`install_staged`] once the volume is unmounted. `allow_download`
/// is as for [`loader`].
pub fn stage(
    root: &Path,
    info: &Info,
    allow_download: bool,
    log: &dyn Fn(String),
) -> Result<Staged, String> {
    let e = |err: io::Error| format!("Syslinux install: {err}");
    log(format!(
        "Installing Syslinux {}{}...",
        info.version, info.ext
    ));
    let (img, bss) = padded_loader(info, allow_download, log)?;
    let path = root.join("ldlinux.sys");
    // A read-only copy from an earlier run can't be opened for writing.
    std::fs::remove_file(&path).ok();
    let mut f = File::create(&path).map_err(e)?;
    f.write_all(&img).map_err(e)?;
    f.write_all(&empty_adv()).map_err(e)?;
    f.sync_all().map_err(e)?;
    let sectors = file_sectors(&f, ldlinux_sectors(&img))
        .map_err(|err| format!("Syslinux install: can't map ldlinux.sys: {err}"))?;
    drop(f);
    // Like Rufus' read-only attribute: keeps the mapped sectors where they are.
    if let Ok(meta) = std::fs::metadata(&path) {
        let mut perms = meta.permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&path, perms).ok();
    }
    Ok(Staged { img, bss, sectors })
}

/// Patches the ldlinux.sys written by [`stage`] in place and writes the
/// Syslinux boot sector, on the unmounted volume `vol`.
pub fn install_staged<F: Read + Write + Seek>(
    vol: &mut F,
    info: &Info,
    staged: Staged,
    log: &dyn Fn(String),
) -> Result<(), String> {
    let (bs, kind) = read_bootsect(vol)?;
    write_patched(vol, info, staged, bs, kind, log)
}

/// The first `want` 512-byte sectors of `f`, relative to the start of its
/// volume, found like Syslinux' sectmap: FIEMAP, else FIBMAP (ntfs-3g).
#[cfg(target_os = "linux")]
fn file_sectors(f: &File, want: usize) -> io::Result<Vec<u64>> {
    fiemap(f, want).or_else(|fiemap_err| {
        fibmap(f, want)
            .map_err(|fibmap_err| io::Error::other(format!("{fiemap_err}; FIBMAP: {fibmap_err}")))
    })
}

#[cfg(not(target_os = "linux"))]
fn file_sectors(_f: &File, _want: usize) -> io::Result<Vec<u64>> {
    Err(io::ErrorKind::Unsupported.into())
}

#[cfg(target_os = "linux")]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct FiemapExtent {
    logical: u64,
    physical: u64,
    length: u64,
    reserved64: [u64; 2],
    flags: u32,
    reserved: [u32; 3],
}

#[cfg(target_os = "linux")]
const FIEMAP_EXTENTS: usize = 32;

/// `struct fiemap` from linux/fiemap.h with room for FIEMAP_EXTENTS extents.
#[cfg(target_os = "linux")]
#[repr(C)]
struct Fiemap {
    start: u64,
    length: u64,
    flags: u32,
    mapped_extents: u32,
    extent_count: u32,
    reserved: u32,
    extents: [FiemapExtent; FIEMAP_EXTENTS],
}

#[cfg(target_os = "linux")]
fn fiemap(f: &File, want: usize) -> io::Result<Vec<u64>> {
    use std::os::fd::AsRawFd;
    const FS_IOC_FIEMAP: u32 = 0xc020_660b;
    const FIEMAP_FLAG_SYNC: u32 = 0x1;
    const FIEMAP_EXTENT_LAST: u32 = 0x1;
    // UNKNOWN, DELALLOC, ENCODED, NOT_ALIGNED, DATA_INLINE, DATA_TAIL, UNWRITTEN:
    // no stable sectors a boot loader could read.
    const UNMAPPABLE: u32 = 0x2 | 0x4 | 0x8 | 0x100 | 0x200 | 0x400 | 0x800;

    let len = (want * SECTOR) as u64;
    let mut sectors: Vec<u64> = Vec::with_capacity(want);
    while sectors.len() < want {
        let mut map = Fiemap {
            start: (sectors.len() * SECTOR) as u64,
            length: len - (sectors.len() * SECTOR) as u64,
            flags: FIEMAP_FLAG_SYNC,
            mapped_extents: 0,
            extent_count: FIEMAP_EXTENTS as u32,
            reserved: 0,
            extents: [FiemapExtent::default(); FIEMAP_EXTENTS],
        };
        // SAFETY: `map` is a valid, exclusively borrowed `struct fiemap` whose
        // `extent_count` matches its trailing array, and the fd outlives the call.
        if unsafe { libc::ioctl(f.as_raw_fd(), FS_IOC_FIEMAP as _, &mut map as *mut Fiemap) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let mapped = &map.extents[..(map.mapped_extents as usize).min(FIEMAP_EXTENTS)];
        if mapped.is_empty() {
            break;
        }
        for x in mapped {
            if x.flags & UNMAPPABLE != 0 || !x.physical.is_multiple_of(SECTOR as u64) {
                return Err(io::Error::other("ldlinux.sys has an unmappable extent"));
            }
            if x.logical != (sectors.len() * SECTOR) as u64 {
                return Err(io::Error::other("ldlinux.sys has a hole"));
            }
            let count = (x.length / SECTOR as u64).min((want - sectors.len()) as u64);
            sectors.extend((0..count).map(|k| x.physical / SECTOR as u64 + k));
        }
        if mapped
            .last()
            .is_some_and(|x| x.flags & FIEMAP_EXTENT_LAST != 0)
        {
            break;
        }
    }
    if sectors.len() < want {
        return Err(io::Error::other("ldlinux.sys is not fully allocated"));
    }
    Ok(sectors)
}

#[cfg(target_os = "linux")]
fn fibmap(f: &File, want: usize) -> io::Result<Vec<u64>> {
    use std::os::fd::AsRawFd;
    const FIBMAP: u32 = 1;
    const FIGETBSZ: u32 = 2;

    let mut block_size: libc::c_int = 0;
    // SAFETY: FIGETBSZ writes one int through the pointer, which is valid and
    // exclusively borrowed, and the fd outlives the call.
    if unsafe {
        libc::ioctl(
            f.as_raw_fd(),
            FIGETBSZ as _,
            &mut block_size as *mut libc::c_int,
        )
    } < 0
    {
        return Err(io::Error::last_os_error());
    }
    let block_size = block_size as usize;
    if block_size < SECTOR || !block_size.is_multiple_of(SECTOR) {
        return Err(io::Error::other(format!("odd block size {block_size}")));
    }
    let per_block = block_size / SECTOR;
    let mut sectors: Vec<u64> = Vec::with_capacity(want);
    for index in 0..want.div_ceil(per_block) {
        let mut block = libc::c_int::try_from(index).map_err(io::Error::other)?;
        // SAFETY: FIBMAP reads and overwrites one int through the pointer, which
        // is valid and exclusively borrowed, and the fd outlives the call.
        if unsafe { libc::ioctl(f.as_raw_fd(), FIBMAP as _, &mut block as *mut libc::c_int) } < 0 {
            return Err(io::Error::last_os_error());
        }
        if block <= 0 {
            return Err(io::Error::other("ldlinux.sys has a hole"));
        }
        let first = block as u64 * per_block as u64;
        let count = per_block.min(want - sectors.len()) as u64;
        sectors.extend(first..first + count);
    }
    Ok(sectors)
}

#[cfg(test)]
#[path = "../tests/unit/syslinux.rs"]
mod tests;
