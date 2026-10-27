// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! Dynamic VHDX writer (MS-VHDX) and sparse readers for dynamic VHDX/VHD.
use crate::bytes::{get16, get32, get64, put, put16, put32, put64};
use crate::part::random_guid;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};

const KB: usize = 1 << 10;
const MB: u64 = 1 << 20;
/// Payload block size, same as the Windows/Rufus default.
const BLOCK: u64 = 32 * MB;
const SECTOR: u64 = 512;
const ZERO: u64 = 2;
const FULLY_PRESENT: u64 = 6;
const BAT_GUID: &str = "2DC27766-F623-4200-9D64-115E9BFD4A08";
const META_GUID: &str = "8B7CA206-4790-4B9A-B8FE-575F050F886E";
const FILE_PARAMS: &str = "CAA16737-FA36-4D43-B3B6-33F0AA44E76B";
const DISK_SIZE: &str = "2FA54224-CD1B-4876-B211-5DBED83BF4B8";
const PAGE83: &str = "BECA12AB-B2E6-4523-93EF-C309E000C746";
const LOGICAL_SECTOR: &str = "8141BF1D-A96F-4709-BA47-F233A8FAAB5F";
const PHYSICAL_SECTOR: &str = "CDA348C7-445D-4471-9CC9-E9885251C556";

pub fn crc32c(data: &[u8]) -> u32 {
    !crate::ext::crc32c(!0, data)
}

/// Stores the CRC-32C of `buf` (with the checksum field zeroed) at offset 4.
fn seal(buf: &mut [u8]) {
    buf[4..8].fill(0);
    let c = crc32c(buf);
    put32(buf, 4, c);
}

fn sealed(buf: &[u8]) -> bool {
    let mut b = buf.to_vec();
    seal(&mut b);
    b[4..8] == buf[4..8]
}

/// Textual GUID to its mixed-endian on-disk form.
fn guid(s: &str) -> [u8; 16] {
    let hex: Vec<u8> = s.bytes().filter(|&c| c != b'-').collect();
    let mut g = [0u8; 16];
    for (i, b) in g.iter_mut().enumerate() {
        let pair = std::str::from_utf8(&hex[i * 2..i * 2 + 2]).expect("GUID constants are ASCII");
        *b = u8::from_str_radix(pair, 16).expect("GUID constants are hex");
    }
    g[..4].reverse();
    g[4..6].reverse();
    g[6..8].reverse();
    g
}

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

/// Payload blocks per sector bitmap block (the BAT interleaves one bitmap entry after each run).
fn chunk_ratio(block: u64, sector: u64) -> u64 {
    (1u64 << 23) * sector / block
}

fn bat_index(b: u64, cr: u64) -> usize {
    (b + b / cr) as usize
}

/// Writes `size` bytes of `src` as a dynamic VHDX; all-zero blocks are left out.
/// `tick` gets the bytes consumed so far and may abort by returning an error.
pub fn save(
    src: &mut impl Read,
    dst: &mut File,
    size: u64,
    tick: &dyn Fn(u64) -> io::Result<()>,
) -> io::Result<()> {
    if size == 0 || !size.is_multiple_of(SECTOR) {
        return Err(invalid(
            "VHDX disk size must be a non-zero multiple of 512 bytes",
        ));
    }
    let (log_off, meta_off, bat_off) = (MB, 2 * MB, 3 * MB);
    let blocks = size.div_ceil(BLOCK);
    let cr = chunk_ratio(BLOCK, SECTOR);
    let entries = blocks + (blocks - 1) / cr;
    let bat_len = (entries * 8).next_multiple_of(MB);
    let mut bat = vec![0u64; entries as usize];
    let mut next = bat_off + bat_len;
    let mut buf = vec![0u8; BLOCK as usize];
    for b in 0..blocks {
        let len = (size - b * BLOCK).min(BLOCK) as usize;
        src.read_exact(&mut buf[..len])?;
        bat[bat_index(b, cr)] = if buf[..len].iter().all(|&x| x == 0) {
            ZERO
        } else {
            buf[len..].fill(0);
            dst.seek(SeekFrom::Start(next))?;
            dst.write_all(&buf)?;
            next += BLOCK;
            (next - BLOCK) | FULLY_PRESENT // offsets are MiB aligned
        };
        tick(b * BLOCK + len as u64)?;
    }
    dst.set_len(next)?;

    let mut head = vec![0u8; 256 * KB + 64 * KB];
    put(&mut head, 0, b"vhdxfile");
    let creator: Vec<u8> = "rustfus"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    put(&mut head, 8, &creator);
    let (fw, dw) = (random_guid(), random_guid());
    for (seq, off) in [(1u64, 64 * KB), (2, 128 * KB)] {
        let h = &mut head[off..off + 4 * KB];
        put(h, 0, b"head");
        put64(h, 8, seq);
        put(h, 16, &fw);
        put(h, 32, &dw);
        // LogGuid stays zero: nothing to replay.
        put16(h, 66, 1);
        put32(h, 68, MB as u32);
        put64(h, 72, log_off);
        seal(h);
    }
    let mut rt = vec![0u8; 64 * KB];
    put(&mut rt, 0, b"regi");
    put32(&mut rt, 8, 2);
    for (i, (id, off, len)) in [(BAT_GUID, bat_off, bat_len), (META_GUID, meta_off, MB)]
        .into_iter()
        .enumerate()
    {
        let e = 16 + i * 32;
        put(&mut rt, e, &guid(id));
        put64(&mut rt, e + 16, off);
        put32(&mut rt, e + 24, len as u32);
        put32(&mut rt, e + 28, 1); // required
    }
    seal(&mut rt);
    put(&mut head, 192 * KB, &rt);
    put(&mut head, 256 * KB, &rt);

    let mut meta = vec![0u8; MB as usize];
    put(&mut meta, 0, b"metadata");
    let items: [(&str, u32, Vec<u8>); 5] = [
        (
            FILE_PARAMS,
            4,
            [(BLOCK as u32).to_le_bytes(), [0; 4]].concat(),
        ),
        (DISK_SIZE, 6, size.to_le_bytes().to_vec()),
        (PAGE83, 6, random_guid().to_vec()),
        (LOGICAL_SECTOR, 6, (SECTOR as u32).to_le_bytes().to_vec()),
        (PHYSICAL_SECTOR, 6, (SECTOR as u32).to_le_bytes().to_vec()),
    ];
    put16(&mut meta, 10, items.len() as u16);
    let mut off = 64 * KB;
    for (i, (id, flags, data)) in items.iter().enumerate() {
        let e = 32 + i * 32;
        put(&mut meta, e, &guid(id));
        put32(&mut meta, e + 16, off as u32);
        put32(&mut meta, e + 20, data.len() as u32);
        put32(&mut meta, e + 24, *flags); // bit 1: virtual disk, bit 2: required
        put(&mut meta, off, data);
        off += data.len();
    }

    let bat: Vec<u8> = bat.iter().flat_map(|e| e.to_le_bytes()).collect();
    for (off, data) in [(meta_off, &meta), (bat_off, &bat), (0, &head)] {
        dst.seek(SeekFrom::Start(off))?;
        dst.write_all(data)?;
    }
    Ok(())
}

/// A sparse virtual disk read sequentially: unallocated blocks read as zeros.
pub struct Sparse {
    f: File,
    pub size: u64,
    block: u64,
    /// File offset of each block's data, `None` when not allocated.
    map: Vec<Option<u64>>,
    pos: u64,
}

impl Read for Sparse {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.pos >= self.size || buf.is_empty() {
            return Ok(0);
        }
        let (b, off) = (self.pos / self.block, self.pos % self.block);
        let n = (buf.len() as u64)
            .min(self.block - off)
            .min(self.size - self.pos) as usize;
        match self.map.get(b as usize).copied().flatten() {
            Some(at) => {
                self.f.seek(SeekFrom::Start(at + off))?;
                self.f.read_exact(&mut buf[..n])?;
            }
            None => buf[..n].fill(0),
        }
        self.pos += n as u64;
        Ok(n)
    }
}

fn read_at(f: &mut File, off: u64, len: usize) -> io::Result<Vec<u8>> {
    let mut b = vec![0u8; len];
    f.seek(SeekFrom::Start(off))?;
    f.read_exact(&mut b)?;
    Ok(b)
}

pub fn open_vhdx(mut f: File) -> io::Result<Sparse> {
    let head = read_at(&mut f, 0, 320 * KB).map_err(|_| invalid("VHDX file is too small"))?;
    if &head[..8] != b"vhdxfile" {
        return Err(invalid("Not a VHDX file (missing signature)"));
    }
    let h = [64 * KB, 128 * KB]
        .iter()
        .map(|&o| &head[o..o + 4 * KB])
        .filter(|h| &h[..4] == b"head" && sealed(h))
        .max_by_key(|h| get64(h, 8))
        .ok_or_else(|| invalid("VHDX headers are corrupted"))?;
    let log_guid = &h[48..64];
    if log_guid.iter().any(|&b| b != 0) {
        // Any log entry tagged with the active log GUID may hold unapplied writes.
        let (log_off, log_len) = (get64(h, 72), get32(h, 68) as usize);
        let log = read_at(&mut f, log_off, log_len)?;
        if log
            .chunks(4 * KB)
            .any(|e| e.len() >= 48 && &e[..4] == b"loge" && &e[32..48] == log_guid)
        {
            return Err(invalid(
                "VHDX has a pending log; open it once with Hyper-V or qemu-img to replay it",
            ));
        }
    }
    let rt = [192 * KB, 256 * KB]
        .iter()
        .map(|&o| &head[o..o + 64 * KB])
        .find(|r| &r[..4] == b"regi" && sealed(r))
        .ok_or_else(|| invalid("VHDX region table is corrupted"))?;
    let region = |id: &str| {
        (0..get32(rt, 8).min(2047) as usize)
            .map(|i| &rt[16 + i * 32..48 + i * 32])
            .find(|e| e[..16] == guid(id))
            .map(|e| (get64(e, 16), get32(e, 24)))
            .ok_or_else(|| invalid("VHDX region table lacks BAT or metadata"))
    };
    let ((bat_off, bat_len), (meta_off, meta_len)) = (region(BAT_GUID)?, region(META_GUID)?);
    let meta = read_at(&mut f, meta_off, meta_len as usize)?;
    if &meta[..8] != b"metadata" {
        return Err(invalid("VHDX metadata table is corrupted"));
    }
    let item = |id: &str, need: usize| {
        (0..get16(&meta, 10).min(2047) as usize)
            .map(|i| &meta[32 + i * 32..64 + i * 32])
            .find(|e| e[..16] == guid(id) && get32(e, 20) as usize >= need)
            .and_then(|e| meta.get(get32(e, 16) as usize..get32(e, 16) as usize + need))
            .ok_or_else(|| invalid(format!("VHDX metadata item {id} is missing")))
    };
    let params = item(FILE_PARAMS, 8)?;
    if get32(params, 4) & 2 != 0 {
        return Err(invalid(
            "Differencing VHDX images are not supported; merge or convert it first",
        ));
    }
    let (block, size) = (get32(params, 0) as u64, get64(item(DISK_SIZE, 8)?, 0));
    let sector = get32(item(LOGICAL_SECTOR, 4)?, 0) as u64;
    if !(MB..=256 * MB).contains(&block)
        || !block.is_power_of_two()
        || ![512, 4096].contains(&sector)
    {
        return Err(invalid("Unsupported VHDX block or sector size"));
    }
    let blocks = size.div_ceil(block);
    let cr = chunk_ratio(block, sector);
    let entries = if blocks == 0 {
        0
    } else {
        blocks + (blocks - 1) / cr
    };
    if entries * 8 > bat_len as u64 {
        return Err(invalid("VHDX BAT is too small for the disk size"));
    }
    let bat = read_at(&mut f, bat_off, entries as usize * 8)?;
    let map = (0..blocks)
        .map(|b| match get64(&bat, bat_index(b, cr) * 8) {
            e if e & 7 == FULLY_PRESENT => Ok(Some(e >> 20 << 20)),
            e if e & 7 == 7 => Err(invalid("Partially present VHDX blocks are not supported")),
            _ => Ok(None),
        })
        .collect::<io::Result<_>>()?;
    Ok(Sparse {
        f,
        size,
        block,
        map,
        pos: 0,
    })
}

/// Dynamic VHD (disk type 3), given its footer.
pub fn open_vhd_dynamic(mut f: File, footer: &[u8]) -> io::Result<Sparse> {
    let be32 =
        |b: &[u8], o: usize| u32::from_be_bytes(b[o..o + 4].try_into().expect("4-byte slice"));
    let be64 =
        |b: &[u8], o: usize| u64::from_be_bytes(b[o..o + 8].try_into().expect("8-byte slice"));
    let dh = read_at(&mut f, be64(footer, 16), 1024)
        .map_err(|_| invalid("VHD dynamic header is missing"))?;
    if &dh[..8] != b"cxsparse" {
        return Err(invalid("VHD dynamic header is corrupted"));
    }
    let (size, block) = (be64(footer, 48), be32(&dh, 32) as u64);
    if block < SECTOR || !block.is_power_of_two() {
        return Err(invalid("Unsupported VHD block size"));
    }
    let entries = (be32(&dh, 28) as u64).min(size.div_ceil(block));
    let bitmap = (block / SECTOR / 8).next_multiple_of(SECTOR);
    let bat = read_at(&mut f, be64(&dh, 16), entries as usize * 4)?;
    let map = bat
        .chunks(4)
        .map(|e| match be32(e, 0) {
            u32::MAX => None,
            s => Some(s as u64 * SECTOR + bitmap),
        })
        .collect();
    Ok(Sparse {
        f,
        size,
        block,
        map,
        pos: 0,
    })
}

#[cfg(test)]
#[path = "../tests/unit/vhdx.rs"]
mod tests;
