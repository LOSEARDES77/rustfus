// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
//! Byte-granular Read/Write/Seek over a region of a raw device that only
//! accepts sector-aligned I/O (Windows PhysicalDrive, macOS rdisk).

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};

const SECTOR: usize = 512;
const BLOCK: usize = 1 << 20;
const MAX_BLOCKS: usize = 64;

pub fn read_at(f: &File, buf: &mut [u8], off: u64) -> io::Result<()> {
    #[cfg(unix)]
    return std::os::unix::fs::FileExt::read_exact_at(f, buf, off);
    #[cfg(windows)]
    {
        let mut done = 0;
        while done < buf.len() {
            match std::os::windows::fs::FileExt::seek_read(f, &mut buf[done..], off + done as u64)? {
                0 => return Err(io::ErrorKind::UnexpectedEof.into()),
                n => done += n,
            }
        }
        Ok(())
    }
}

pub fn write_at(f: &File, buf: &[u8], off: u64) -> io::Result<()> {
    #[cfg(unix)]
    return std::os::unix::fs::FileExt::write_all_at(f, buf, off);
    #[cfg(windows)]
    {
        let mut done = 0;
        while done < buf.len() {
            done += std::os::windows::fs::FileExt::seek_write(f, &buf[done..], off + done as u64)?;
        }
        Ok(())
    }
}

struct Block {
    data: Vec<u8>,
    /// Sectors whose content is known (read from disk or fully overwritten).
    valid: Vec<bool>,
    dirty: bool,
}

/// Write-back cache of 1 MiB blocks. Sequential writes that cover whole
/// sectors never read the device, so copying large files costs one pass.
pub struct BlockDev<'a> {
    file: &'a File,
    base: u64,
    len: u64,
    pos: u64,
    cache: HashMap<u64, Block>,
}

impl<'a> BlockDev<'a> {
    /// `base` and `len` must be sector-aligned.
    pub fn new(file: &'a File, base: u64, len: u64) -> Self {
        debug_assert!(base.is_multiple_of(SECTOR as u64) && len.is_multiple_of(SECTOR as u64));
        BlockDev { file, base, len, pos: 0, cache: HashMap::new() }
    }

    fn block_len(&self, idx: u64) -> usize {
        (self.len - idx * BLOCK as u64).min(BLOCK as u64) as usize
    }

    fn fill(&self, idx: u64, b: &mut Block, sectors: std::ops::Range<usize>) -> io::Result<()> {
        if sectors.clone().all(|s| b.valid[s]) {
            return Ok(());
        }
        let (start, end) = (sectors.start * SECTOR, sectors.end * SECTOR);
        let mut disk = vec![0u8; end - start];
        read_at(self.file, &mut disk, self.base + idx * BLOCK as u64 + start as u64)?;
        for s in sectors {
            if !b.valid[s] {
                let r = s * SECTOR - start..(s + 1) * SECTOR - start;
                b.data[s * SECTOR..(s + 1) * SECTOR].copy_from_slice(&disk[r]);
                b.valid[s] = true;
            }
        }
        Ok(())
    }

    fn block(&mut self, idx: u64) -> io::Result<&mut Block> {
        if !self.cache.contains_key(&idx) {
            if self.cache.len() >= MAX_BLOCKS {
                self.write_back()?;
                self.cache.clear();
            }
            let n = self.block_len(idx);
            let b = Block { data: vec![0; n], valid: vec![false; n / SECTOR], dirty: false };
            self.cache.insert(idx, b);
        }
        Ok(self.cache.get_mut(&idx).unwrap())
    }

    fn write_back(&mut self) -> io::Result<()> {
        let mut dirty: Vec<u64> = self.cache.iter().filter(|(_, b)| b.dirty).map(|(i, _)| *i).collect();
        dirty.sort();
        for idx in dirty {
            let mut b = self.cache.remove(&idx).unwrap();
            let n = b.valid.len();
            self.fill(idx, &mut b, 0..n)?;
            write_at(self.file, &b.data, self.base + idx * BLOCK as u64)?;
            b.dirty = false;
            self.cache.insert(idx, b);
        }
        Ok(())
    }

    fn prepare(&self, idx: u64, blk: &mut Block, off: usize, n: usize, write: bool) -> io::Result<()> {
        let sectors = off / SECTOR..(off + n).div_ceil(SECTOR);
        if !write {
            return self.fill(idx, blk, sectors);
        }
        // Only partially covered edge sectors need their old content.
        if !off.is_multiple_of(SECTOR) {
            self.fill(idx, blk, sectors.start..sectors.start + 1)?;
        }
        if !(off + n).is_multiple_of(SECTOR) {
            self.fill(idx, blk, sectors.end - 1..sectors.end)?;
        }
        sectors.for_each(|s| blk.valid[s] = true);
        blk.dirty = true;
        Ok(())
    }

    /// Walks `len` bytes from the current position block by block, calling
    /// `f(block, offset_in_block, bytes_done, n)` on each prepared piece.
    fn io(&mut self, len: usize, write: bool, mut f: impl FnMut(&mut Block, usize, usize, usize)) -> io::Result<usize> {
        let len = len.min((self.len - self.pos.min(self.len)) as usize);
        let mut done = 0;
        while done < len {
            let idx = self.pos / BLOCK as u64;
            let off = (self.pos % BLOCK as u64) as usize;
            let n = (len - done).min(self.block_len(idx) - off);
            self.block(idx)?;
            let mut blk = self.cache.remove(&idx).unwrap();
            let r = self.prepare(idx, &mut blk, off, n, write);
            if r.is_ok() {
                f(&mut blk, off, done, n);
            }
            self.cache.insert(idx, blk);
            r?;
            done += n;
            self.pos += n as u64;
        }
        Ok(done)
    }
}

impl Read for BlockDev<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.io(buf.len(), false, |b, off, done, n| {
            buf[done..done + n].copy_from_slice(&b.data[off..off + n])
        })
    }
}

impl Write for BlockDev<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.pos + buf.len() as u64 > self.len {
            return Err(io::Error::new(io::ErrorKind::WriteZero, "write past end of partition"));
        }
        self.io(buf.len(), true, |b, off, done, n| {
            b.data[off..off + n].copy_from_slice(&buf[done..done + n])
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        self.write_back()?;
        self.file.sync_all()
    }
}

impl Seek for BlockDev<'_> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new = match pos {
            SeekFrom::Start(p) => p as i128,
            SeekFrom::End(d) => self.len as i128 + d as i128,
            SeekFrom::Current(d) => self.pos as i128 + d as i128,
        };
        if new < 0 || new > self.len as i128 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "seek out of range"));
        }
        self.pos = new as u64;
        Ok(self.pos)
    }
}

impl Drop for BlockDev<'_> {
    fn drop(&mut self) {
        self.write_back().ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unaligned_io_round_trips() {
        let path = std::env::temp_dir().join(format!("rustfus-bd-{}", std::process::id()));
        std::fs::write(&path, vec![0xAAu8; 3 * BLOCK]).unwrap();
        let f = std::fs::OpenOptions::new().read(true).write(true).open(&path).unwrap();
        {
            let mut d = BlockDev::new(&f, 4096, 2 * BLOCK as u64);
            d.seek(SeekFrom::Start(BLOCK as u64 - 3)).unwrap();
            d.write_all(b"hello").unwrap();
            let big: Vec<u8> = (0..BLOCK + 100).map(|i| i as u8).collect();
            d.seek(SeekFrom::Start(700)).unwrap();
            d.write_all(&big[..BLOCK - 800]).unwrap();
            assert!(d.write_all(&[0; 1]).is_ok());
            d.seek(SeekFrom::End(0)).unwrap();
            assert!(d.write_all(&[0; 1]).is_err());
            d.flush().unwrap();
            let mut back = [0u8; 5];
            d.seek(SeekFrom::Start(BLOCK as u64 - 3)).unwrap();
            d.read_exact(&mut back).unwrap();
            assert_eq!(&back, b"hello");
        }
        let raw = std::fs::read(&path).unwrap();
        assert!(raw[..4096 + 700].iter().all(|&b| b == 0xAA));
        assert_eq!(raw[4096 + 700], 0);
        assert_eq!(&raw[4096 + BLOCK - 3..4096 + BLOCK + 2], b"hello");
        assert!(raw[4096 + 2 * BLOCK..].iter().all(|&b| b == 0xAA));
        std::fs::remove_file(path).ok();
    }
}

