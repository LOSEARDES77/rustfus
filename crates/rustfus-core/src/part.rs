// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
//! MBR and GPT partition table writers.

use crate::bytes::{put, put32, put64};
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Seek, SeekFrom, Write};

pub const SECTOR: u64 = 512;

pub const GPT_BASIC_DATA: [u8; 16] = guid(
    0xebd0a0a2,
    0xb9e5,
    0x4433,
    [0x87, 0xc0, 0x68, 0xb6, 0xb7, 0x26, 0x99, 0xc7],
);
pub const GPT_ESP: [u8; 16] = guid(
    0xc12a7328,
    0xf81f,
    0x11d2,
    [0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e, 0xc9, 0x3b],
);
pub const GPT_LINUX: [u8; 16] = guid(
    0x0fc63daf,
    0x8483,
    0x4772,
    [0x8e, 0x79, 0x3d, 0x69, 0xd8, 0x47, 0x7d, 0xe4],
);

/// GPT_BASIC_DATA_ATTRIBUTE_NO_DRIVE_LETTER
const GPT_NO_DRIVE_LETTER: u64 = 1 << 63;

/// GUIDs are stored mixed-endian: first three fields little endian.
const fn guid(a: u32, b: u16, c: u16, d: [u8; 8]) -> [u8; 16] {
    let (a, b, c) = (a.to_le_bytes(), b.to_le_bytes(), c.to_le_bytes());
    [
        a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d[0], d[1], d[2], d[3], d[4], d[5], d[6],
        d[7],
    ]
}

pub fn random_u64() -> u64 {
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
    );
    h.finish()
}

pub(crate) fn random_guid() -> [u8; 16] {
    let mut g = [0u8; 16];
    g[..8].copy_from_slice(&random_u64().to_le_bytes());
    g[8..].copy_from_slice(&random_u64().to_le_bytes());
    g[7] = (g[7] & 0x0f) | 0x40; // version 4
    g[8] = (g[8] & 0x3f) | 0x80;
    g
}

#[derive(Clone, Debug)]
pub struct Part {
    pub start: u64, // bytes, sector aligned
    pub size: u64,
    pub mbr_type: u8,
    pub gpt_type: [u8; 16],
    pub name: &'static str,
    pub active: bool,
}

fn chs(lba: u64) -> [u8; 3] {
    let (heads, spt) = (255u64, 63u64);
    let c = lba / (heads * spt);
    if c > 1023 {
        return [0xfe, 0xff, 0xff];
    }
    let h = (lba / spt) % heads;
    let s = lba % spt + 1;
    [h as u8, (s as u8) | ((c >> 2) as u8 & 0xc0), c as u8]
}

fn mbr_entry(e: &mut [u8], p: &Part) {
    let (lba, n) = (p.start / SECTOR, p.size / SECTOR);
    e[0] = if p.active { 0x80 } else { 0 };
    e[1..4].copy_from_slice(&chs(lba));
    e[4] = p.mbr_type;
    e[5..8].copy_from_slice(&chs(lba + n - 1));
    put32(e, 8, lba as u32);
    put32(e, 12, n as u32);
}

fn write_sector0<D: Write + Seek>(dev: &mut D, entries: &[Part]) -> io::Result<()> {
    let mut s = [0u8; 512];
    put32(&mut s, 440, random_u64() as u32);
    for (i, p) in entries.iter().enumerate().take(4) {
        mbr_entry(&mut s[446 + i * 16..462 + i * 16], p);
    }
    s[510] = 0x55;
    s[511] = 0xaa;
    dev.seek(SeekFrom::Start(0))?;
    dev.write_all(&s)
}

fn zero<D: Write + Seek>(dev: &mut D, at: u64, len: u64) -> io::Result<()> {
    dev.seek(SeekFrom::Start(at))?;
    dev.write_all(&vec![0u8; len as usize])
}

/// Zeroes the gap before the first partition (at least the 33 GPT sectors, at
/// most 1 MiB), so stale signatures there, like an ISO9660 descriptor at 32
/// KiB left by a hybrid ISO, don't make the whole disk look formatted.
fn clear_gap<D: Write + Seek>(dev: &mut D, parts: &[Part]) -> io::Result<()> {
    let first = parts.iter().map(|p| p.start).min().unwrap_or(u64::MAX);
    zero(dev, SECTOR, first.clamp(34 * SECTOR, 1 << 20) - SECTOR)
}

/// Writes an MBR table (boot code left zeroed) and wipes stale GPT
/// structures at both ends of the disk.
pub fn write_mbr<D: Write + Seek>(dev: &mut D, disk_size: u64, parts: &[Part]) -> io::Result<()> {
    clear_gap(dev, parts)?;
    zero(dev, disk_size - 33 * SECTOR, 33 * SECTOR)?;
    write_sector0(dev, parts)
}

pub fn write_gpt<D: Write + Seek>(dev: &mut D, disk_size: u64, parts: &[Part]) -> io::Result<()> {
    let last = disk_size / SECTOR - 1;
    clear_gap(dev, parts)?;
    let protective = Part {
        start: SECTOR,
        size: (disk_size - SECTOR).min(u32::MAX as u64 * SECTOR),
        mbr_type: 0xee,
        gpt_type: [0; 16],
        name: "",
        active: false,
    };
    write_sector0(dev, &[protective])?;

    let mut table = vec![0u8; 128 * 128];
    for (i, p) in parts.iter().enumerate() {
        let e = &mut table[i * 128..(i + 1) * 128];
        put(e, 0, &p.gpt_type);
        put(e, 16, &random_guid());
        put64(e, 32, p.start / SECTOR);
        put64(e, 40, (p.start + p.size) / SECTOR - 1);
        if p.name == "UEFI:NTFS" {
            put64(e, 48, GPT_NO_DRIVE_LETTER);
        }
        for (j, c) in p.name.encode_utf16().take(36).enumerate() {
            put(e, 56 + j * 2, &c.to_le_bytes());
        }
    }
    let table_crc = crc32fast::hash(&table);
    let disk_guid = random_guid();
    let header = |current: u64, backup: u64, entries: u64| {
        let mut h = [0u8; 512];
        put(&mut h, 0, b"EFI PART");
        put32(&mut h, 8, 0x0001_0000);
        put32(&mut h, 12, 92);
        put64(&mut h, 24, current);
        put64(&mut h, 32, backup);
        put64(&mut h, 40, 34);
        put64(&mut h, 48, last - 33);
        put(&mut h, 56, &disk_guid);
        put64(&mut h, 72, entries);
        put32(&mut h, 80, 128);
        put32(&mut h, 84, 128);
        put32(&mut h, 88, table_crc);
        let crc = crc32fast::hash(&h[..92]);
        put32(&mut h, 16, crc);
        h
    };
    dev.seek(SeekFrom::Start(SECTOR))?;
    dev.write_all(&header(1, last, 2))?;
    dev.write_all(&table)?;
    dev.seek(SeekFrom::Start((last - 32) * SECTOR))?;
    dev.write_all(&table)?;
    dev.write_all(&header(last, 1, last - 32))
}

#[cfg(test)]
#[path = "../tests/unit/part.rs"]
mod tests;
