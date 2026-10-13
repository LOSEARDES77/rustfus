// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! FAT16/FAT32 formatter laid out like Windows/Rufus (32 reserved sectors,
//! so the ms-sys boot records fit), plus a fix-up for fatfs' directory dot
//! entries. File contents are written with the fatfs crate.

use std::io::{self, Read, Seek, SeekFrom, Write};

const BPS: u64 = 512;

fn put16(b: &mut [u8], at: usize, v: u16) {
    b[at..at + 2].copy_from_slice(&v.to_le_bytes());
}
fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
fn get16(b: &[u8], at: usize) -> u64 {
    u16::from_le_bytes([b[at], b[at + 1]]) as u64
}
fn get32(b: &[u8], at: usize) -> u64 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap()) as u64
}

fn invalid(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, msg)
}

/// Default cluster size, following Microsoft's format tables.
fn default_cluster(size: u64, fat32: bool) -> u32 {
    let gb = 1u64 << 30;
    if fat32 {
        match size {
            s if s <= 64 << 20 => 512,
            s if s <= 128 << 20 => 1024,
            s if s <= 256 << 20 => 2048,
            s if s <= 8 * gb => 4096,
            s if s <= 16 * gb => 8192,
            s if s <= 32 * gb => 16384,
            _ => 32768,
        }
    } else {
        match size {
            s if s <= 128 << 20 => 2048,
            s if s <= 256 << 20 => 4096,
            s if s <= 512 << 20 => 8192,
            s if s <= gb => 16384,
            s if s <= 2 * gb => 32768,
            _ => 65536,
        }
    }
}

fn zero<D: Write + Seek>(dev: &mut D, at: u64, len: u64) -> io::Result<()> {
    dev.seek(SeekFrom::Start(at))?;
    let buf = vec![0u8; 1 << 20];
    let mut left = len;
    while left > 0 {
        let n = left.min(buf.len() as u64) as usize;
        dev.write_all(&buf[..n])?;
        left -= n as u64;
    }
    Ok(())
}

/// Formats `size` bytes of `dev` as FAT32 (or FAT16). `hidden` is the
/// partition's starting LBA, which boot code relies on.
pub fn format<D: Write + Seek>(
    dev: &mut D,
    size: u64,
    fat32: bool,
    cluster: u32,
    label: [u8; 11],
    hidden: u32,
) -> io::Result<()> {
    let cluster = if cluster == 0 {
        default_cluster(size, fat32)
    } else {
        cluster
    };
    let spc = cluster as u64 / BPS;
    if !(1..=128).contains(&spc) || !spc.is_power_of_two() {
        return Err(invalid("invalid cluster size"));
    }
    let total = size / BPS;
    if total > u32::MAX as u64 {
        return Err(invalid("partition too large for FAT"));
    }
    let (reserved, root_sectors) = if fat32 { (32u64, 0u64) } else { (1, 32) };
    // Microsoft's FAT size formula (fatgen103).
    let tmp2 = if fat32 {
        (256 * spc + 2) / 2
    } else {
        256 * spc + 2
    };
    let fatsz = (total - reserved - root_sectors).div_ceil(tmp2);
    let clusters = (total - reserved - 2 * fatsz - root_sectors) / spc;
    let ok = if fat32 {
        (65525..0x0FFF_FFF5).contains(&clusters)
    } else {
        (4085..65525).contains(&clusters)
    };
    if !ok {
        return Err(invalid(if fat32 {
            "drive too small or cluster size too large for FAT32"
        } else {
            "drive or cluster size not suitable for FAT16"
        }));
    }
    let volid = crate::part::random_u64() as u32;

    let mut bs = vec![0u8; 3 * BPS as usize];
    bs[..3].copy_from_slice(&[0xEB, if fat32 { 0x58 } else { 0x3C }, 0x90]);
    bs[3..11].copy_from_slice(b"MSWIN4.1");
    put16(&mut bs, 0x0B, BPS as u16);
    bs[0x0D] = spc as u8;
    put16(&mut bs, 0x0E, reserved as u16);
    bs[0x10] = 2;
    put16(&mut bs, 0x11, if fat32 { 0 } else { 512 });
    if total < 0x10000 {
        put16(&mut bs, 0x13, total as u16);
    } else {
        put32(&mut bs, 0x20, total as u32);
    }
    bs[0x15] = 0xF8;
    put16(&mut bs, 0x18, 63);
    put16(&mut bs, 0x1A, 255);
    put32(&mut bs, 0x1C, hidden);
    let ext = if fat32 {
        put32(&mut bs, 0x24, fatsz as u32);
        put32(&mut bs, 0x2C, 2);
        put16(&mut bs, 0x30, 1);
        put16(&mut bs, 0x32, 6);
        0x40
    } else {
        put16(&mut bs, 0x16, fatsz as u16);
        0x24
    };
    bs[ext] = 0x80;
    bs[ext + 2] = 0x29;
    put32(&mut bs, ext + 3, volid);
    bs[ext + 7..ext + 18].copy_from_slice(&label);
    bs[ext + 18..ext + 26].copy_from_slice(if fat32 { b"FAT32   " } else { b"FAT16   " });
    bs[0x1FE..0x200].copy_from_slice(&[0x55, 0xAA]);
    if fat32 {
        bs[0x5FE..0x600].copy_from_slice(&[0x55, 0xAA]);
        // FSInfo sector
        put32(&mut bs, 0x200, 0x4161_5252);
        put32(&mut bs, 0x3E4, 0x6141_7272);
        put32(&mut bs, 0x3E8, clusters as u32 - 1);
        put32(&mut bs, 0x3EC, 3);
        put32(&mut bs, 0x3FC, 0xAA55_0000);
    }

    let data = (reserved + 2 * fatsz + root_sectors) * BPS;
    zero(dev, 0, data + if fat32 { cluster as u64 } else { 0 })?;
    dev.seek(SeekFrom::Start(0))?;
    dev.write_all(if fat32 { &bs } else { &bs[..512] })?;
    if fat32 {
        dev.seek(SeekFrom::Start(6 * BPS))?;
        dev.write_all(&bs)?;
    }
    let mut fat = [0u8; 12];
    let n = if fat32 {
        put32(&mut fat, 0, 0x0FFF_FFF8);
        put32(&mut fat, 4, 0x0FFF_FFFF);
        put32(&mut fat, 8, 0x0FFF_FFFF);
        12
    } else {
        put16(&mut fat, 0, 0xFFF8);
        put16(&mut fat, 2, 0xFFFF);
        4
    };
    for i in 0..2 {
        dev.seek(SeekFrom::Start((reserved + i * fatsz) * BPS))?;
        dev.write_all(&fat[..n])?;
    }
    // Volume label entry, first slot of the root directory.
    let mut entry = [0u8; 32];
    entry[..11].copy_from_slice(&label);
    entry[11] = 0x08;
    dev.seek(SeekFrom::Start(data - root_sectors * BPS))?;
    dev.write_all(&entry)?;
    // fatfs expects to be handed the volume positioned at its start.
    dev.seek(SeekFrom::Start(0))?;
    dev.flush()
}

struct Layout {
    spc: u64,
    fat_start: u64,
    root_start: u64,
    data_start: u64,
    root_cluster: u64,
    fat32: bool,
}

impl Layout {
    fn read<D: Read + Seek>(dev: &mut D) -> io::Result<Layout> {
        let mut b = [0u8; 512];
        dev.seek(SeekFrom::Start(0))?;
        dev.read_exact(&mut b)?;
        let (spc, reserved) = (b[0x0D] as u64, get16(&b, 0x0E));
        let fat32 = get16(&b, 0x16) == 0;
        let fatsz = if fat32 {
            get32(&b, 0x24)
        } else {
            get16(&b, 0x16)
        };
        let root_sectors = (get16(&b, 0x11) * 32).div_ceil(BPS);
        let fat_start = reserved * BPS;
        let root_start = fat_start + b[0x10] as u64 * fatsz * BPS;
        Ok(Layout {
            spc,
            fat_start,
            root_start,
            data_start: root_start + root_sectors * BPS,
            root_cluster: if fat32 { get32(&b, 0x2C) } else { 0 },
            fat32,
        })
    }

    fn offset(&self, cluster: u64) -> u64 {
        self.data_start + (cluster - 2) * self.spc * BPS
    }

    fn next<D: Read + Seek>(&self, dev: &mut D, c: u64) -> io::Result<Option<u64>> {
        let mut b = [0u8; 4];
        let (n, eoc) = if self.fat32 {
            (4, 0x0FFF_FFF8)
        } else {
            (2, 0xFFF8)
        };
        dev.seek(SeekFrom::Start(self.fat_start + c * n as u64))?;
        dev.read_exact(&mut b[..n])?;
        let v = if self.fat32 {
            get32(&b, 0) & 0x0FFF_FFFF
        } else {
            get16(&b, 0)
        };
        Ok((2..eoc).contains(&v).then_some(v))
    }

    /// Raw entries of a directory (cluster 0 = FAT16 fixed root).
    fn entries<D: Read + Seek>(&self, dev: &mut D, cluster: u64) -> io::Result<Vec<u8>> {
        let mut out = Vec::new();
        if cluster == 0 {
            out.resize((self.data_start - self.root_start) as usize, 0);
            dev.seek(SeekFrom::Start(self.root_start))?;
            dev.read_exact(&mut out)?;
            return Ok(out);
        }
        let mut c = Some(cluster);
        let mut hops = 0;
        while let Some(cl) = c {
            let start = out.len();
            out.resize(start + (self.spc * BPS) as usize, 0);
            dev.seek(SeekFrom::Start(self.offset(cl)))?;
            dev.read_exact(&mut out[start..])?;
            c = self.next(dev, cl)?;
            hops += 1;
            if hops > 1 << 16 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "directory cluster loop",
                ));
            }
        }
        Ok(out)
    }
}

fn entry_cluster(e: &[u8]) -> u64 {
    (get16(e, 20) << 16) | get16(e, 26)
}

fn set_cluster(e: &mut [u8], c: u64) {
    put16(e, 20, (c >> 16) as u16);
    put16(e, 26, c as u16);
}

const DOT: &[u8; 11] = b".          ";
const DOTDOT: &[u8; 11] = b"..         ";

/// fatfs 0.3 writes long-name entries in front of "." and "..", and points
/// ".." of root subdirectories at the root cluster instead of 0. Both break
/// `fsck` and strict firmware, so rewrite the first slots of every directory.
pub fn fix_dot_entries<D: Read + Write + Seek>(dev: &mut D) -> io::Result<()> {
    let l = Layout::read(dev)?;
    let root = if l.fat32 { l.root_cluster } else { 0 };
    let mut queue = vec![(root, 0u64)];
    let mut seen = std::collections::HashSet::new();
    while let Some((dir, parent)) = queue.pop() {
        if !seen.insert(dir) {
            continue;
        }
        let raw = l.entries(dev, dir)?;
        if dir != root {
            let first = &raw[..128];
            let slot = |name: &[u8; 11]| {
                (0..4).find(|i| &first[i * 32..i * 32 + 11] == name && first[i * 32 + 11] != 0x0F)
            };
            if let (Some(d), Some(dd)) = (slot(DOT), slot(DOTDOT)) {
                let mut fixed = first.to_vec();
                for i in 0..4 {
                    let is_lfn = first[i * 32 + 11] == 0x0F;
                    if is_lfn || i == d || i == dd {
                        fixed[i * 32] = 0xE5;
                    }
                }
                let (mut de, mut dde) = (
                    first[d * 32..d * 32 + 32].to_vec(),
                    first[dd * 32..dd * 32 + 32].to_vec(),
                );
                set_cluster(&mut de, dir);
                set_cluster(&mut dde, parent);
                fixed[..32].copy_from_slice(&de);
                fixed[32..64].copy_from_slice(&dde);
                dev.seek(SeekFrom::Start(l.offset(dir)))?;
                dev.write_all(&fixed)?;
            }
        }
        for e in raw.chunks_exact(32) {
            if e[0] == 0 {
                break;
            }
            if e[0] == 0xE5
                || e[0] == b'.'
                || e[11] == 0x0F
                || e[11] & 0x08 != 0
                || e[11] & 0x10 == 0
            {
                continue;
            }
            let child = entry_cluster(e);
            if child >= 2 {
                queue.push((child, if dir == root { 0 } else { dir }));
            }
        }
    }
    dev.seek(SeekFrom::Start(0))?;
    dev.flush()
}

/// Finds the 8.3 entry `name` in the root directory, like libfat's
/// searchdir + nextsector: returns the byte offset of its directory entry
/// and up to `max` volume-relative 512-byte sectors of its cluster chain.
pub fn root_file<D: Read + Seek>(
    dev: &mut D,
    name: &[u8; 11],
    max: usize,
) -> io::Result<Option<(u64, Vec<u64>)>> {
    let l = Layout::read(dev)?;
    let root = if l.fat32 { l.root_cluster } else { 0 };
    let raw = l.entries(dev, root)?;
    let csize = if root == 0 {
        raw.len().max(1)
    } else {
        (l.spc * BPS) as usize
    };
    let mut starts = vec![l.root_start];
    if root != 0 {
        starts.clear();
        let mut c = Some(root);
        while let Some(cl) = c
            && starts.len() < raw.len() / csize
        {
            starts.push(l.offset(cl));
            c = l.next(dev, cl)?;
        }
    }
    for (i, e) in raw.chunks_exact(32).enumerate() {
        if e[0] == 0 {
            break;
        }
        if e[0] == 0xE5 || e[11] == 0x0F || e[11] & 0x18 != 0 || &e[..11] != name {
            continue;
        }
        let at = i * 32;
        let mut sectors = Vec::new();
        let mut c = Some(entry_cluster(e)).filter(|&c| c >= 2);
        while let Some(cl) = c
            && sectors.len() < max
        {
            let first = l.offset(cl) / BPS;
            sectors.extend((first..first + l.spc).take(max - sectors.len()));
            c = l.next(dev, cl)?;
        }
        return Ok(Some((starts[at / csize] + (at % csize) as u64, sectors)));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn check(fat32: bool, size: u64) {
        let path = std::env::temp_dir().join(format!("rustfus-fat-{}-{fat32}", std::process::id()));
        let mut f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        f.set_len(size).unwrap();
        format(&mut f, size, fat32, 0, *b"TESTLABEL  ", 2048).unwrap();
        {
            let fs = fatfs::FileSystem::new(&mut f, fatfs::FsOptions::new()).unwrap();
            fs.root_dir()
                .create_file("bootmgr")
                .unwrap()
                .write_all(&vec![7u8; 300_000])
                .unwrap();
            fs.root_dir().create_dir("EFI").unwrap();
            fs.root_dir().create_dir("EFI/BOOT").unwrap();
            fs.root_dir()
                .create_file("EFI/BOOT/BOOTX64.EFI")
                .unwrap()
                .write_all(b"loader")
                .unwrap();
            fs.unmount().unwrap();
        }
        fix_dot_entries(&mut f).unwrap();
        {
            let fs = fatfs::FileSystem::new(&mut f, fatfs::FsOptions::new()).unwrap();
            let mut v = Vec::new();
            fs.root_dir()
                .open_file("bootmgr")
                .unwrap()
                .read_to_end(&mut v)
                .unwrap();
            assert_eq!(v, vec![7u8; 300_000]);
            assert_eq!(fs.volume_label().trim(), "TESTLABEL");
        }
        let mut bs = [0u8; 512];
        f.seek(SeekFrom::Start(0)).unwrap();
        f.read_exact(&mut bs).unwrap();
        if fat32 {
            assert_eq!(get16(&bs, 0x0E), 32);
        }
        assert_eq!(get32(&bs, 0x1C), 2048);
        if let Ok(out) = Command::new("fsck.vfat").arg("-n").arg(&path).output() {
            let text = String::from_utf8_lossy(&out.stdout);
            assert!(
                out.status.success() && !text.contains("Expected") && !text.contains("Bad"),
                "{text}"
            );
        }
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn fat32_formats_and_passes_fsck() {
        check(true, 100 << 20);
    }

    #[test]
    fn fat16_formats_and_passes_fsck() {
        check(false, 200 << 20);
    }

    #[test]
    fn fat32_rejects_tiny_volume() {
        let mut c = io::Cursor::new(vec![0u8; 8 << 20]);
        assert!(format(&mut c, 8 << 20, true, 4096, [b' '; 11], 0).is_err());
    }
}
