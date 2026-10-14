// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
//! MBR and GPT partition table writers.

use crate::bytes::{put, put32, put64};
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Seek, SeekFrom, Write};

pub const SECTOR: u64 = 512;

pub const GPT_BASIC_DATA: [u8; 16] = guid(
    0xEBD0A0A2,
    0xB9E5,
    0x4433,
    [0x87, 0xC0, 0x68, 0xB6, 0xB7, 0x26, 0x99, 0xC7],
);
pub const GPT_ESP: [u8; 16] = guid(
    0xC12A7328,
    0xF81F,
    0x11D2,
    [0xBA, 0x4B, 0x00, 0xA0, 0xC9, 0x3E, 0xC9, 0x3B],
);
pub const GPT_LINUX: [u8; 16] = guid(
    0x0FC63DAF,
    0x8483,
    0x4772,
    [0x8E, 0x79, 0x3D, 0x69, 0xD8, 0x47, 0x7D, 0xE4],
);

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
    g[7] = (g[7] & 0x0F) | 0x40; // version 4
    g[8] = (g[8] & 0x3F) | 0x80;
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
        return [0xFE, 0xFF, 0xFF];
    }
    let h = (lba / spt) % heads;
    let s = lba % spt + 1;
    [h as u8, (s as u8) | ((c >> 2) as u8 & 0xC0), c as u8]
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
    s[511] = 0xAA;
    dev.seek(SeekFrom::Start(0))?;
    dev.write_all(&s)
}

fn zero<D: Write + Seek>(dev: &mut D, at: u64, len: u64) -> io::Result<()> {
    dev.seek(SeekFrom::Start(at))?;
    dev.write_all(&vec![0u8; len as usize])
}

/// Writes an MBR table (boot code left zeroed) and wipes stale GPT
/// structures at both ends of the disk.
pub fn write_mbr<D: Write + Seek>(dev: &mut D, disk_size: u64, parts: &[Part]) -> io::Result<()> {
    zero(dev, SECTOR, 33 * SECTOR)?;
    zero(dev, disk_size - 33 * SECTOR, 33 * SECTOR)?;
    write_sector0(dev, parts)
}

pub fn write_gpt<D: Write + Seek>(dev: &mut D, disk_size: u64, parts: &[Part]) -> io::Result<()> {
    let last = disk_size / SECTOR - 1;
    let protective = Part {
        start: SECTOR,
        size: (disk_size - SECTOR).min(u32::MAX as u64 * SECTOR),
        mbr_type: 0xEE,
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
mod tests {
    use super::*;
    use std::process::Command;

    fn layout(gpt: bool) -> Option<String> {
        let path = std::env::temp_dir().join(format!("rustfus-part-{}-{gpt}", std::process::id()));
        let size = 64 << 20;
        let mut f = std::fs::File::create(&path).unwrap();
        f.set_len(size).unwrap();
        let parts = [
            Part {
                start: 1 << 20,
                size: 32 << 20,
                mbr_type: 0x0C,
                gpt_type: GPT_BASIC_DATA,
                name: "Main",
                active: true,
            },
            Part {
                start: 33 << 20,
                size: 1 << 20,
                mbr_type: 0xEF,
                gpt_type: GPT_ESP,
                name: "UEFI:NTFS",
                active: false,
            },
        ];
        if gpt {
            write_gpt(&mut f, size, &parts)
        } else {
            write_mbr(&mut f, size, &parts)
        }
        .unwrap();
        let out = Command::new("sfdisk").arg("-d").arg(&path).output().ok()?;
        std::fs::remove_file(&path).ok();
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    }

    #[test]
    fn sfdisk_reads_tables() {
        let Some(mbr) = layout(false) else { return };
        assert!(mbr.contains("label: dos"), "{mbr}");
        assert!(
            mbr.contains("start=        2048, size=       65536, type=c, bootable"),
            "{mbr}"
        );
        assert!(
            mbr.contains("start=       67584, size=        2048, type=ef"),
            "{mbr}"
        );
        let gpt = layout(true).unwrap();
        assert!(gpt.contains("label: gpt"), "{gpt}");
        assert!(
            gpt.contains("type=EBD0A0A2-B9E5-4433-87C0-68B6B72699C7"),
            "{gpt}"
        );
        assert!(
            gpt.contains("type=C12A7328-F81F-11D2-BA4B-00A0C93EC93B, uuid="),
            "{gpt}"
        );
        assert!(gpt.contains("name=\"UEFI:NTFS\""), "{gpt}");
    }
}
