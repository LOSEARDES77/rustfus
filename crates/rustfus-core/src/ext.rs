// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! ext2/ext3/ext4 formatter (Rufus does this with its bundled ext2fs), so
//! persistence partitions can be made without mkfs.ext* on any OS. Layout
//! follows mke2fs: sparse_super backups, 256-byte inodes, no resize inode,
//! no flex_bg/64bit. Ext4 adds extents and metadata_csum, which lets inode
//! tables past group 0 stay unwritten (the kernel zeroes them lazily).

use crate::bytes::{put16, put32};
use std::collections::BTreeMap;
use std::io::{self, Seek, SeekFrom, Write};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Ext2,
    Ext3,
    Ext4,
}

const ISIZE: usize = 256;

fn invalid(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, msg)
}

const CRC32C: [u32; 256] = {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                (c >> 1) ^ 0x82F6_3B78
            } else {
                c >> 1
            };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
};

/// Raw crc32c (no pre/post inversion), as ext4's checksums use it.
pub(crate) fn crc32c(crc: u32, data: &[u8]) -> u32 {
    data.iter().fold(crc, |c, &b| {
        CRC32C[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8)
    })
}

fn has_sb(g: u64) -> bool {
    g <= 1
        || [3, 5, 7].iter().any(|&p| {
            let mut x = g;
            while x.is_multiple_of(p) {
                x /= p;
            }
            x == 1
        })
}

fn set_bits(b: &mut [u8], r: std::ops::Range<u64>) {
    for i in r {
        b[i as usize / 8] |= 1 << (i % 8);
    }
}

fn random16() -> [u8; 16] {
    let mut u = [0u8; 16];
    u[..8].copy_from_slice(&crate::part::random_u64().to_le_bytes());
    u[8..].copy_from_slice(&crate::part::random_u64().to_le_bytes());
    u[6] = u[6] & 0x0F | 0x40;
    u[8] = u[8] & 0x3F | 0x80;
    u
}

struct Fs {
    bs: u64,
    first: u64,
    bpg: u64,
    blocks: u64,
    groups: u64,
    ipg: u64,
    itb: u64,
    gdtb: u64,
    ext4: bool,
    seed: u32,
    now: u32,
    used: Vec<u64>,
    g: u64,
    next: u64,
    data: BTreeMap<u64, Vec<u8>>,
}

impl Fs {
    fn start(&self, g: u64) -> u64 {
        self.first + g * self.bpg
    }
    fn bitmap(&self, g: u64) -> u64 {
        self.start(g) + if has_sb(g) { 1 + self.gdtb } else { 0 }
    }
    fn data_start(&self, g: u64) -> u64 {
        self.bitmap(g) + 2 + self.itb
    }
    fn end(&self, g: u64) -> u64 {
        (self.start(g) + self.bpg).min(self.blocks)
    }

    /// Blocks are handed out sequentially, so each group's used data blocks
    /// are the contiguous run starting at `data_start`.
    fn alloc(&mut self) -> io::Result<u64> {
        while self.next >= self.end(self.g) {
            self.g += 1;
            if self.g >= self.groups {
                return Err(invalid("partition too small for this filesystem"));
            }
            self.next = self.data_start(self.g);
        }
        self.used[self.g as usize] += 1;
        self.next += 1;
        Ok(self.next - 1)
    }

    fn table(&mut self, entries: &[u32]) -> io::Result<u32> {
        let b = self.alloc()?;
        let mut v = vec![0u8; self.bs as usize];
        for (k, &x) in entries.iter().enumerate() {
            put32(&mut v, 4 * k, x);
        }
        self.data.insert(b, v);
        Ok(b as u32)
    }

    /// Per-inode checksum seed (generation is always 0).
    fn ino_seed(&self, ino: u32) -> u32 {
        crc32c(crc32c(self.seed, &ino.to_le_bytes()), &[0; 4])
    }

    fn dir_block(&self, dir: u32, entries: &[(u32, u8, &[u8])]) -> io::Result<Vec<u8>> {
        let bs = self.bs as usize;
        let end = bs - if self.ext4 { 12 } else { 0 };
        let mut b = vec![0u8; bs];
        let mut off = 0;
        for (k, &(ino, kind, name)) in entries.iter().enumerate() {
            let min = (8 + name.len() + 3) & !3;
            if off + min > end {
                return Err(invalid("too many files for the root directory"));
            }
            let rec = if k + 1 == entries.len() {
                end - off
            } else {
                min
            };
            put32(&mut b, off, ino);
            put16(&mut b, off + 4, rec as u16);
            b[off + 6] = name.len() as u8;
            b[off + 7] = kind;
            b[off + 8..off + 8 + name.len()].copy_from_slice(name);
            off += rec;
        }
        if entries.is_empty() {
            put16(&mut b, 4, end as u16);
        }
        if self.ext4 {
            put16(&mut b, end + 4, 12);
            b[end + 7] = 0xDE;
            let c = crc32c(self.ino_seed(dir), &b[..end]);
            put32(&mut b, end + 8, c);
        }
        Ok(b)
    }

    /// Allocates blocks for `size` bytes (the first `content.len()` of
    /// them stored, the rest zero) and returns the finished inode.
    fn inode(
        &mut self,
        ino: u32,
        mode: u16,
        links: u16,
        content: &[u8],
        size: u64,
    ) -> io::Result<[u8; ISIZE]> {
        let bs = self.bs as usize;
        let mut list = Vec::new();
        for i in 0..size.div_ceil(self.bs) as usize {
            let b = self.alloc()?;
            if i * bs < content.len() {
                let mut blk = content[i * bs..content.len().min(i * bs + bs)].to_vec();
                blk.resize(bs, 0);
                self.data.insert(b, blk);
            }
            list.push(b);
        }
        let mut r = [0u8; ISIZE];
        let mut meta = 0;
        let ib = &mut r[0x28..0x64];
        if self.ext4 {
            let mut runs: Vec<(u32, u64, u32)> = Vec::new();
            for (i, &b) in list.iter().enumerate() {
                match runs.last_mut() {
                    Some(x) if x.1 + x.2 as u64 == b && x.2 < 32768 => x.2 += 1,
                    _ => runs.push((i as u32, b, 1)),
                }
            }
            let hdr = |buf: &mut [u8], n: usize, max: usize, depth: u16| {
                put16(buf, 0, 0xF30A);
                put16(buf, 2, n as u16);
                put16(buf, 4, max as u16);
                put16(buf, 6, depth);
            };
            let extents = |buf: &mut [u8]| {
                for (k, &(lblk, start, len)) in runs.iter().enumerate() {
                    put32(buf, 12 + 12 * k, lblk);
                    put16(buf, 16 + 12 * k, len as u16);
                    put16(buf, 18 + 12 * k, (start >> 32) as u16);
                    put32(buf, 20 + 12 * k, start as u32);
                }
            };
            if runs.len() <= 4 {
                hdr(ib, runs.len(), 4, 0);
                extents(ib);
            } else {
                let max = (bs - 12) / 12;
                if runs.len() > max {
                    return Err(io::Error::other("file too fragmented"));
                }
                let mut leaf = vec![0u8; bs];
                hdr(&mut leaf, runs.len(), max, 0);
                extents(&mut leaf);
                let c = crc32c(self.ino_seed(ino), &leaf[..12 + 12 * max]);
                put32(&mut leaf, 12 + 12 * max, c);
                let lb = self.alloc()?;
                self.data.insert(lb, leaf);
                meta = 1;
                hdr(ib, 1, 4, 1);
                put32(ib, 16, lb as u32);
            }
            put32(&mut r, 0x20, 0x80000); // EXT4_EXTENTS_FL
        } else {
            let p = bs / 4;
            let l: Vec<u32> = list.iter().map(|&b| b as u32).collect();
            for (k, &b) in l.iter().take(12).enumerate() {
                put32(ib, 4 * k, b);
            }
            if l.len() > 12 {
                put32(ib, 48, self.table(&l[12..l.len().min(12 + p)])?);
                meta += 1;
            }
            if l.len() > 12 + p {
                let mut l2 = Vec::new();
                for c in l[12 + p..].chunks(p) {
                    l2.push(self.table(c)?);
                }
                if l2.len() > p {
                    return Err(io::Error::other("file too large"));
                }
                put32(ib, 52, self.table(&l2)?);
                meta += 1 + l2.len() as u64;
            }
        }
        put16(&mut r, 0, mode);
        put32(&mut r, 4, size as u32);
        for at in [0x08, 0x0C, 0x10, 0x90] {
            put32(&mut r, at, self.now);
        }
        put16(&mut r, 0x1A, links);
        put32(
            &mut r,
            0x1C,
            ((list.len() as u64 + meta) * self.bs / 512) as u32,
        );
        put32(&mut r, 0x6C, (size >> 32) as u32);
        put16(&mut r, 0x80, 32); // i_extra_isize
        if self.ext4 {
            let c = crc32c(self.ino_seed(ino), &r);
            put16(&mut r, 0x7C, c as u16);
            put16(&mut r, 0x82, (c >> 16) as u16);
        }
        Ok(r)
    }
}

/// mke2fs' default journal size, in blocks.
fn journal_blocks(blocks: u64) -> u64 {
    match blocks {
        b if b < 32768 => 1024,
        b if b < 256 * 1024 => 4096,
        b if b < 512 * 1024 => 8192,
        b if b < 4096 * 1024 => 16384,
        b if b < 8192 * 1024 => 32768,
        b if b < 16384 * 1024 => 65536,
        b if b < 32768 * 1024 => 131072,
        _ => 262144,
    }
}

/// Formats `size` bytes as an empty ext filesystem with root dir,
/// lost+found, the given label (<=16 bytes), block size (0 = default: 1024
/// below 512 MiB, else 4096, like mke2fs), and the given small files placed
/// in the root directory.
pub fn format<D: Write + Seek>(
    dev: &mut D,
    size: u64,
    kind: Kind,
    label: &str,
    block_size: u32,
    files: &[(&str, &[u8])],
) -> io::Result<()> {
    let bs = match block_size {
        0 if size < 512 << 20 => 1024,
        0 => 4096,
        1024 | 2048 | 4096 => block_size as u64,
        _ => return Err(invalid("invalid block size")),
    };
    if label.len() > 16 {
        return Err(invalid("label longer than 16 bytes"));
    }
    for (name, _) in files {
        if name.is_empty()
            || name.len() > 255
            || name.contains(['/', '\0'])
            || [".", "..", "lost+found"].contains(name)
        {
            return Err(invalid("invalid file name"));
        }
    }
    let (ext4, journal) = (kind == Kind::Ext4, kind != Kind::Ext2);
    let mut blocks = size / bs;
    if blocks > u32::MAX as u64 {
        return Err(invalid("partition too large for ext without 64bit"));
    }
    if blocks < if journal { 2048 } else { 64 } {
        return Err(invalid("partition too small for this filesystem"));
    }
    let first = (bs == 1024) as u64;
    let bpg = 8 * bs;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as u32;
    let uuid = random16();
    let mut fs = Fs {
        bs,
        first,
        bpg,
        blocks,
        groups: 0,
        ipg: 0,
        itb: 0,
        gdtb: 0,
        ext4,
        seed: crc32c(!0, &uuid),
        now,
        used: vec![],
        g: 0,
        next: 0,
        data: BTreeMap::new(),
    };
    let ratio = if size < 512 << 20 { 4096 } else { 16384 };
    let nfiles = files.len() as u64;
    fs.groups = (blocks - first).div_ceil(bpg);
    loop {
        fs.gdtb = (fs.groups * 32).div_ceil(bs);
        fs.ipg = (blocks * bs / ratio)
            .div_ceil(fs.groups)
            .max(16 + nfiles)
            .next_multiple_of((bs / ISIZE as u64).max(8))
            .min(bpg);
        fs.itb = fs.ipg * ISIZE as u64 / bs;
        // Drop a last group too small to be worth its metadata, like mke2fs.
        let last = fs.groups - 1;
        if last > 0 && blocks < fs.data_start(last) + 50 {
            blocks = fs.start(last);
            fs.blocks = blocks;
            fs.groups -= 1;
            continue;
        }
        break;
    }
    let (groups, ipg) = (fs.groups, fs.ipg);
    if 12 + nfiles > ipg || ipg * groups > u32::MAX as u64 {
        return Err(invalid("too many files"));
    }
    fs.used = vec![0; groups as usize];
    fs.next = fs.data_start(0);

    let mut inodes = Vec::new();
    let mut ents: Vec<(u32, u8, &[u8])> = vec![(2, 2, b"."), (2, 2, b".."), (11, 2, b"lost+found")];
    for (i, (name, _)) in files.iter().enumerate() {
        ents.push((12 + i as u32, 1, name.as_bytes()));
    }
    let root = fs.dir_block(2, &ents)?;
    inodes.push((2, fs.inode(2, 0x41ED, 3, &root, bs)?));
    let mut lpf = fs.dir_block(11, &[(11, 2, b"."), (2, 2, b"..")])?;
    for _ in 1..(16384 / bs).min(12) {
        lpf.extend(fs.dir_block(11, &[])?);
    }
    inodes.push((11, fs.inode(11, 0x41C0, 2, &lpf, lpf.len() as u64)?));
    for (i, (_, data)) in files.iter().enumerate() {
        let ino = 12 + i as u32;
        inodes.push((ino, fs.inode(ino, 0x81A4, 1, data, data.len() as u64)?));
    }
    let mut sb = vec![0u8; 1024];
    if journal {
        let jlen = journal_blocks(blocks);
        let mut jsb = vec![0u8; bs as usize];
        for (at, v) in [
            (0, 0xC03B_3998),
            (4, 4),
            (12, bs as u32),
            (16, jlen as u32),
            (20, 1),
            (24, 1),
            (64, 1),
        ] {
            jsb[at..at + 4].copy_from_slice(&u32::to_be_bytes(v));
        }
        jsb[48..64].copy_from_slice(&uuid);
        let j = fs.inode(8, 0x8180, 1, &jsb, jlen * bs)?;
        // Backup of the journal inode's block map, plus its size.
        sb[0x10C..0x148].copy_from_slice(&j[0x28..0x64]);
        sb[0x148..0x150].copy_from_slice(
            &j[0x6C..0x70]
                .iter()
                .chain(&j[4..8])
                .copied()
                .collect::<Vec<_>>(),
        );
        put32(&mut sb, 0xE0, 8);
        sb[0xFD] = 1;
        inodes.push((8, j));
    }
    let used_inodes = 11 + nfiles;

    let bitmaps = |g: u64| {
        let (s, e) = (fs.start(g), fs.end(g));
        let mut bb = vec![0u8; bs as usize];
        set_bits(&mut bb, 0..fs.data_start(g) - s + fs.used[g as usize]);
        set_bits(&mut bb, e - s..bpg);
        let mut ib = vec![0u8; bs as usize];
        set_bits(&mut ib, 0..if g == 0 { used_inodes } else { 0 });
        set_bits(&mut ib, ipg..8 * bs);
        (bb, ib)
    };
    let mut gdt = vec![0u8; (fs.gdtb * bs) as usize];
    let mut free_blocks = 0;
    for g in 0..groups {
        let (bb, ib) = bitmaps(g);
        let free = fs.end(g) - fs.data_start(g) - fs.used[g as usize];
        let ui = if g == 0 { used_inodes } else { 0 };
        free_blocks += free;
        let d = &mut gdt[g as usize * 32..g as usize * 32 + 32];
        put32(d, 0, fs.bitmap(g) as u32);
        put32(d, 4, fs.bitmap(g) as u32 + 1);
        put32(d, 8, fs.bitmap(g) as u32 + 2);
        put16(d, 0x0C, free as u16);
        put16(d, 0x0E, (ipg - ui) as u16);
        put16(d, 0x10, if g == 0 { 2 } else { 0 });
        if ext4 {
            put16(d, 0x12, if g == 0 { 4 } else { 1 }); // ITABLE_ZEROED / INODE_UNINIT
            put16(d, 0x18, crc32c(fs.seed, &bb) as u16);
            put16(d, 0x1A, crc32c(fs.seed, &ib[..ipg as usize / 8]) as u16);
            put16(d, 0x1C, (ipg - ui) as u16);
            let c = crc32c(crc32c(fs.seed, &(g as u32).to_le_bytes()), d); // checksum field still zero
            put16(d, 0x1E, c as u16);
        }
    }

    for (at, v) in [
        (0x00, ipg * groups),
        (0x04, blocks),
        (0x08, blocks / 20),
        (0x0C, free_blocks),
        (0x10, ipg * groups - used_inodes),
        (0x14, first),
        (0x18, bs.trailing_zeros() as u64 - 10),
        (0x1C, bs.trailing_zeros() as u64 - 10),
        (0x20, bpg),
        (0x24, bpg),
        (0x28, ipg),
        (0x30, now as u64),
        (0x40, now as u64),
        (0x4C, 1),
        (0x54, 11),
        (0x5C, 0x28 | if journal { 4 } else { 0 }), // ext_attr, dir_index, has_journal
        (0x60, 2 | if ext4 { 0x40 } else { 0 }),    // filetype, extents
        (0x64, 3 | if ext4 { 0x468 } else { 0 }), // sparse_super, large_file (+ huge_file, dir_nlink, extra_isize, metadata_csum)
        (0x100, 0x0C),                            // user_xattr, acl
        (0x108, now as u64),
        (0x160, 1), // signed directory hash
    ] {
        put32(&mut sb, at, v as u32);
    }
    for (at, v) in [
        (0x36, 0xFFFF),
        (0x38, 0xEF53),
        (0x3A, 1),
        (0x3C, 1),
        (0x58, ISIZE as u16),
        (0x15C, 32),
        (0x15E, 32),
    ] {
        put16(&mut sb, at, v);
    }
    sb[0x68..0x78].copy_from_slice(&uuid);
    sb[0x78..0x78 + label.len()].copy_from_slice(label.as_bytes());
    sb[0xEC..0xFC].copy_from_slice(&random16());
    sb[0xFC] = 1; // half_md4
    sb[0x175] = ext4 as u8; // crc32c

    let mut itable0 = vec![0u8; (fs.itb * bs) as usize];
    for (ino, raw) in &inodes {
        let at = (*ino as usize - 1) * ISIZE;
        itable0[at..at + ISIZE].copy_from_slice(raw);
    }
    let mut buf = Vec::new();
    let mut pos;
    let flush = |dev: &mut D, buf: &mut Vec<u8>, pos: &mut u64| -> io::Result<()> {
        dev.seek(SeekFrom::Start(*pos))?;
        dev.write_all(buf)?;
        *pos += buf.len() as u64;
        buf.clear();
        Ok(())
    };
    let zero_block = vec![0u8; bs as usize];
    for g in 0..groups {
        pos = fs.start(g) * bs;
        if g == 0 {
            pos = 0;
            buf.resize(1024, 0);
        }
        if has_sb(g) {
            put16(&mut sb, 0x5A, g as u16);
            if ext4 {
                let c = crc32c(!0, &sb[..0x3FC]);
                put32(&mut sb, 0x3FC, c);
            }
            buf.extend_from_slice(&sb);
            buf.resize(buf.len().next_multiple_of(bs as usize), 0);
            buf.extend_from_slice(&gdt);
        }
        let (bb, ib) = bitmaps(g);
        buf.extend_from_slice(&bb);
        buf.extend_from_slice(&ib);
        if g == 0 {
            buf.extend_from_slice(&itable0);
        } else if !ext4 {
            buf.resize(buf.len() + itable0.len(), 0);
        } else {
            flush(dev, &mut buf, &mut pos)?;
            pos += itable0.len() as u64;
        }
        let ds = fs.data_start(g);
        for b in ds..ds + fs.used[g as usize] {
            buf.extend_from_slice(fs.data.get(&b).unwrap_or(&zero_block));
            if buf.len() >= 1 << 20 {
                flush(dev, &mut buf, &mut pos)?;
            }
        }
        flush(dev, &mut buf, &mut pos)?;
    }
    dev.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn crc32c_check_value() {
        assert_eq!(!crc32c(!0, b"123456789"), 0xE306_9283);
    }

    #[test]
    fn superblock_in_memory() {
        let mut c = io::Cursor::new(vec![0u8; 8 << 20]);
        format(&mut c, 8 << 20, Kind::Ext4, "casper-rw", 0, &[]).unwrap();
        let b = c.into_inner();
        assert_eq!(&b[1080..1082], &[0x53, 0xEF]);
        assert_eq!(&b[1024 + 0x78..1024 + 0x78 + 10], b"casper-rw\0");
    }

    #[test]
    fn e2fsck_clean() {
        let run = |cmd: &str, args: &[&str], img: &std::path::Path| {
            let o = Command::new(cmd).args(args).arg(img).output().unwrap();
            let text = String::from_utf8_lossy(&o.stdout).into_owned()
                + &String::from_utf8_lossy(&o.stderr);
            (o.status.success(), o.stdout, text)
        };
        if ["e2fsck", "dumpe2fs", "debugfs"]
            .iter()
            .any(|t| Command::new(t).arg("-V").output().is_err())
        {
            eprintln!("e2fsprogs not installed, skipping");
            return;
        }
        let big: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        for kind in [Kind::Ext2, Kind::Ext3, Kind::Ext4] {
            for (size, bs) in [(64u64 << 20, 1024), (300 << 20, 0), (2 << 30, 4096)] {
                let path = std::env::temp_dir().join(format!(
                    "rustfus-ext-{}-{kind:?}-{size}",
                    std::process::id()
                ));
                let mut f = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(&path)
                    .unwrap();
                f.set_len(size).unwrap();
                format(
                    &mut f,
                    size,
                    kind,
                    "persistence",
                    bs,
                    &[("persistence.conf", b"/ union\n"), ("big.bin", &big)],
                )
                .unwrap();
                drop(f);
                let (ok, _, out) = run("e2fsck", &["-fn"], &path);
                assert!(ok && !out.contains("Fix?"), "{kind:?} {size}: {out}");
                let (_, _, out) = run("dumpe2fs", &["-h"], &path);
                assert!(
                    out.contains("Filesystem volume name:   persistence\n"),
                    "{out}"
                );
                assert_eq!(
                    run("debugfs", &["-R", "cat /persistence.conf"], &path).1,
                    b"/ union\n"
                );
                assert!(
                    run("debugfs", &["-R", "cat /big.bin"], &path).1 == big,
                    "{kind:?} {size}: big.bin differs"
                );
                std::fs::remove_file(path).ok();
            }
        }
    }
}
