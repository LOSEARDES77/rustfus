// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const CHUNK: usize = 4 << 20;
const SECTOR: usize = 512;
const COMPRESSED: [&str; 6] = ["gz", "bz2", "xz", "lzma", "zst", "zip"];

/// A disk image opened for DD-mode writing, decompressed on the fly.
pub struct Source {
    pub reader: Box<dyn Read + Send>,
    /// Uncompressed size, when known without decoding the whole stream.
    pub size: Option<u64>,
    /// Bytes of the file read so far, for progress against `total`.
    pub consumed: Arc<AtomicU64>,
    /// Denominator for `consumed`: the file size, or the virtual size for sparse VHD/VHDX.
    pub total: u64,
}

struct Counted<R> {
    inner: R,
    n: Arc<AtomicU64>,
}

impl<R: Read> Read for Counted<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let k = self.inner.read(buf)?;
        self.n.fetch_add(k as u64, Ordering::Relaxed);
        Ok(k)
    }
}

fn ext(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

pub fn is_compressed(path: &Path) -> bool {
    COMPRESSED.contains(&ext(path).as_str())
}

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

/// Sparse virtual disks report progress in virtual bytes.
fn sparse(s: crate::vhdx::Sparse) -> Source {
    let (size, consumed) = (s.size, Arc::new(AtomicU64::new(0)));
    Source {
        reader: Box::new(Counted {
            inner: s,
            n: consumed.clone(),
        }),
        size: Some(size),
        consumed,
        total: size,
    }
}

pub fn open(path: &Path) -> io::Result<Source> {
    let mut f = File::open(path)?;
    let total = f.metadata()?.len();
    let consumed = Arc::new(AtomicU64::new(0));
    let mut size = None;
    let mut start = 0;
    let mut limit = u64::MAX;
    let mut zip_method = None;
    match ext(path).as_str() {
        "vhd" => {
            let mut footer = [0u8; 512];
            if total < 512 {
                return Err(invalid("VHD file is too small"));
            }
            f.seek(SeekFrom::End(-512))?;
            f.read_exact(&mut footer)?;
            if &footer[..8] != b"conectix" {
                return Err(invalid("Not a VHD file (missing footer)"));
            }
            match u32::from_be_bytes(footer[60..64].try_into().expect("4-byte slice")) {
                2 => {}
                3 => return crate::vhdx::open_vhd_dynamic(f, &footer).map(sparse),
                4 => {
                    return Err(invalid(
                        "Differencing VHDs are not supported; convert to a fixed VHD first",
                    ));
                }
                t => return Err(invalid(format!("Unknown VHD disk type {t}"))),
            }
            limit = total - 512;
            size = Some(limit);
        }
        "vhdx" => return crate::vhdx::open_vhdx(f).map(sparse),
        "zip" => {
            let mut zip = zip::ZipArchive::new(BufReader::new(&f)).map_err(io::Error::other)?;
            let idx = (0..zip.len())
                .find(|&i| zip.by_index_raw(i).is_ok_and(|e| !e.is_dir()))
                .ok_or_else(|| invalid("ZIP archive contains no files"))?;
            let e = zip.by_index_raw(idx).map_err(io::Error::other)?;
            if e.encrypted() {
                return Err(invalid("Encrypted ZIP archives are not supported"));
            }
            start = e.data_start().ok_or_else(|| invalid("Bad ZIP entry"))?;
            limit = e.compressed_size();
            size = Some(e.size());
            zip_method = Some(e.compression());
        }
        _ => {}
    }
    if limit == u64::MAX && !is_compressed(path) {
        size = Some(total);
    }
    f.seek(SeekFrom::Start(start))?;
    consumed.store(start, Ordering::Relaxed);
    let raw = BufReader::with_capacity(
        1 << 20,
        Counted {
            inner: f,
            n: consumed.clone(),
        }
        .take(limit),
    );
    let reader: Box<dyn Read + Send> = match ext(path).as_str() {
        "gz" => Box::new(flate2::read::MultiGzDecoder::new(raw)),
        "bz2" => Box::new(bzip2::read::MultiBzDecoder::new(raw)),
        "xz" => Box::new(lzma_rust2::XzReader::new(raw, true)),
        "lzma" => Box::new(lzma_rust2::LzmaReader::new_mem_limit(raw, u32::MAX, None)?),
        "zst" => Box::new(ruzstd::decoding::StreamingDecoder::new(raw).map_err(io::Error::other)?),
        "zip" => match zip_method {
            Some(zip::CompressionMethod::Stored) => Box::new(raw),
            Some(zip::CompressionMethod::Deflated) => {
                Box::new(flate2::read::DeflateDecoder::new(raw))
            }
            m => {
                return Err(invalid(format!(
                    "Unsupported ZIP compression {m:?} (only Stored/Deflate)"
                )));
            }
        },
        _ => Box::new(raw),
    };
    Ok(Source {
        reader,
        size,
        consumed,
        total,
    })
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn interrupted() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "Cancelled")
}

/// MD5, SHA-1, SHA-256 and SHA-512 of a file, in one pass.
pub fn hashes(path: &Path, progress: &dyn Fn(f32), cancel: &AtomicBool) -> io::Result<[String; 4]> {
    use sha2::Digest;
    let mut f = File::open(path)?;
    let total = f.metadata()?.len().max(1);
    let (mut md5, mut sha1, mut sha256, mut sha512) = (
        md5::Md5::new(),
        sha1::Sha1::new(),
        sha2::Sha256::new(),
        sha2::Sha512::new(),
    );
    let mut buf = vec![0u8; CHUNK];
    let mut done = 0u64;
    // ponytail: hashes run sequentially per chunk; one thread per hash if this becomes the bottleneck.
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(interrupted());
        }
        let n = match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        };
        md5.update(&buf[..n]);
        sha1.update(&buf[..n]);
        sha256.update(&buf[..n]);
        sha512.update(&buf[..n]);
        done += n as u64;
        progress(done as f32 / total as f32);
    }
    Ok([
        hex(&md5.finalize()),
        hex(&sha1.finalize()),
        hex(&sha256.finalize()),
        hex(&sha512.finalize()),
    ])
}

/// Without this, Linux would read back the page cache instead of the device.
pub(crate) fn drop_cache(_f: &File) {
    #[cfg(target_os = "linux")]
    // SAFETY: posix_fadvise only reads its integer arguments, and the fd stays
    // open for the call because `_f` is borrowed. Failure is harmless (the
    // result is a hint), so the return value is ignored.
    unsafe {
        use std::os::fd::AsRawFd;
        libc::posix_fadvise(_f.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED);
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct BadBlocks {
    pub bad_sectors: u64,
    pub read_errors: u64,
    pub write_errors: u64,
    pub corruption_errors: u64,
}

const RANDOM: u32 = u32::MAX;
/// Rufus aborts the test past this many bad blocks.
const MAX_BAD_BLOCKS: usize = 256;

/// Rufus' "Check device for bad blocks" choices (label, patterns per pass);
/// multi-byte patterns repeat big-endian like Rufus. The random entry is
/// Rufus' random `pattern_fill` mode, which its combo doesn't expose.
pub const BAD_BLOCK_MODES: &[(&str, &[u32])] = &[
    ("1 pass", &[0x55]),
    ("2 passes", &[0x55, 0xaa]),
    ("4 passes for SLC NAND", &[0x00, 0xff, 0x55, 0xaa]),
    ("4 passes for MLC NAND", &[0x00, 0xff, 0x33, 0xcc]),
    ("4 passes for TLC NAND", &[0x00, 0xff, 0x1c71c7, 0xe38e38]),
    ("1 pass, random pattern", &[RANDOM]),
];

fn pattern_fill(buf: &mut [u8], pat: u32) {
    if pat == RANDOM {
        let mut x = crate::part::random_u64() | 1;
        for c in buf.chunks_mut(8) {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            c.copy_from_slice(&x.to_le_bytes()[..c.len()]);
        }
        return;
    }
    let b = pat.to_be_bytes();
    let b = &b[b.iter().position(|&x| x != 0).unwrap_or(3)..];
    for (d, s) in buf.iter_mut().zip(b.iter().cycle()) {
        *d = *s;
    }
}

/// Stamps each sector with its number so fake (wrap-around) drives show up as corruption.
fn stamp(buf: &mut [u8], first_sector: u64) {
    for (i, s) in buf.chunks_mut(SECTOR).enumerate() {
        s[..8].copy_from_slice(&(first_sector + i as u64).to_le_bytes());
    }
}

/// Runs `op` on the whole chunk at `off`; if it fails, retries sector by
/// sector and returns the sectors that still fail.
fn chunk_io(
    f: &mut File,
    off: u64,
    buf: &mut [u8],
    op: fn(&mut File, &mut [u8]) -> io::Result<()>,
) -> Vec<u64> {
    if f.seek(SeekFrom::Start(off))
        .and_then(|_| op(f, buf))
        .is_ok()
    {
        return vec![];
    }
    let mut bad = vec![];
    for (i, s) in buf.chunks_mut(SECTOR).enumerate() {
        let pos = off + (i * SECTOR) as u64;
        if f.seek(SeekFrom::Start(pos)).and_then(|_| op(f, s)).is_err() {
            bad.push(pos / SECTOR as u64);
        }
    }
    bad
}

/// Destructive write/read test of the whole device, like Rufus' bad blocks check.
/// `mode` indexes `BAD_BLOCK_MODES`.
pub fn bad_blocks(
    dev: &str,
    size: u64,
    mode: usize,
    progress: &dyn Fn(f32, String),
    cancel: &AtomicBool,
) -> io::Result<BadBlocks> {
    let size = size / SECTOR as u64 * SECTOR as u64;
    let mut f = OpenOptions::new().read(true).write(true).open(dev)?;
    let pats = BAD_BLOCK_MODES[mode.min(BAD_BLOCK_MODES.len() - 1)].1;
    let mut r = BadBlocks::default();
    let mut bad = HashSet::new();
    let (mut pbuf, mut wbuf, mut rbuf) = (vec![0u8; CHUNK], vec![0u8; CHUNK], vec![0u8; CHUNK]);
    let steps = (pats.len() * 2) as f32;
    'test: for (p, &pat) in pats.iter().enumerate() {
        pattern_fill(&mut pbuf, pat);
        let name = if pat == RANDOM {
            "random data".into()
        } else {
            format!("0x{pat:02X}")
        };
        for phase in 0..2 {
            let mut off = 0u64;
            while off < size {
                if cancel.load(Ordering::Relaxed) {
                    return Err(interrupted());
                }
                let len = (size - off).min(CHUNK as u64) as usize;
                wbuf[..len].copy_from_slice(&pbuf[..len]);
                stamp(&mut wbuf[..len], off / SECTOR as u64);
                if phase == 0 {
                    let errs = chunk_io(&mut f, off, &mut wbuf[..len], |f, b| f.write_all(b));
                    r.write_errors += errs.len() as u64;
                    bad.extend(errs);
                } else {
                    let errs = chunk_io(&mut f, off, &mut rbuf[..len], |f, b| f.read_exact(b));
                    r.read_errors += errs.len() as u64;
                    for (i, (a, b)) in wbuf[..len]
                        .chunks(SECTOR)
                        .zip(rbuf[..len].chunks(SECTOR))
                        .enumerate()
                    {
                        let sector = off / SECTOR as u64 + i as u64;
                        if a != b && !errs.contains(&sector) {
                            r.corruption_errors += 1;
                            bad.insert(sector);
                        }
                    }
                    bad.extend(errs);
                }
                off += len as u64;
                let frac = off as f32 / size.max(1) as f32;
                let what = if phase == 0 { "Writing" } else { "Verifying" };
                progress(
                    ((p * 2 + phase) as f32 + frac) / steps,
                    format!(
                        "Pass {}/{}: {what} {name} ({:.1}%)",
                        p + 1,
                        pats.len(),
                        frac * 100.0
                    ),
                );
                if bad.len() >= MAX_BAD_BLOCKS {
                    progress(1.0, "Too many bad blocks, aborting test".into());
                    break 'test;
                }
            }
            if phase == 0 {
                f.sync_all()?;
                drop_cache(&f);
            }
        }
    }
    r.bad_sectors = bad.len() as u64;
    Ok(r)
}

/// CHS geometry per the VHD spec (cylinders, heads, sectors per track).
fn vhd_geometry(size: u64) -> (u16, u8, u8) {
    let total = (size / 512).min(65535 * 16 * 255);
    let (spt, heads, cth);
    if total >= 65535 * 16 * 63 {
        (spt, heads) = (255, 16);
        cth = total / spt;
    } else {
        let mut s = 17;
        let mut c = total / s;
        let mut h = c.div_ceil(1024).max(4);
        if c >= h * 1024 || h > 16 {
            (s, h) = (31, 16);
            c = total / s;
        }
        if c >= h * 1024 {
            (s, h) = (63, 16);
            c = total / s;
        }
        (spt, heads, cth) = (s, h, c);
    }
    ((cth / heads) as u16, heads as u8, spt as u8)
}

fn vhd_footer(size: u64) -> [u8; 512] {
    let mut f = [0u8; 512];
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let (c, h, s) = vhd_geometry(size);
    f[..8].copy_from_slice(b"conectix");
    f[8..12].copy_from_slice(&2u32.to_be_bytes()); // features: reserved bit
    f[12..16].copy_from_slice(&0x0001_0000u32.to_be_bytes()); // format version 1.0
    f[16..24].fill(0xff); // data offset: none (fixed disk)
    f[24..28].copy_from_slice(&(now.as_secs().saturating_sub(946_684_800) as u32).to_be_bytes());
    f[28..32].copy_from_slice(b"rufs");
    f[32..36].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    f[36..40].copy_from_slice(b"Wi2k");
    f[40..48].copy_from_slice(&size.to_be_bytes());
    f[48..56].copy_from_slice(&size.to_be_bytes());
    f[56..58].copy_from_slice(&c.to_be_bytes());
    f[58] = h;
    f[59] = s;
    f[60..64].copy_from_slice(&2u32.to_be_bytes()); // fixed disk
    // Unique ID: no RNG dependency, time and size are unique enough here.
    f[68..84].copy_from_slice(&(now.as_nanos() ^ ((size as u128) << 64)).to_le_bytes());
    let sum: u32 = f.iter().map(|&b| b as u32).sum();
    f[64..68].copy_from_slice(&(!sum).to_be_bytes());
    f
}

pub fn is_ffu(path: &Path) -> bool {
    ext(path) == "ffu"
}

/// Runs `%SystemRoot%\System32\dism.exe` with Rufus' argument quoting.
#[cfg(windows)]
fn dism(args: &str) -> io::Result<()> {
    use std::os::windows::process::CommandExt;
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    let st = std::process::Command::new(format!(r"{root}\System32\dism.exe"))
        .raw_arg(args)
        .status()?;
    if st.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("dism failed ({st})")))
    }
}

#[cfg(not(windows))]
fn dism(_: &str) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "FFU images require Windows (dism)",
    ))
}

/// Writes an FFU image to `dev` (`\\.\PhysicalDriveN`) with dism, like Rufus.
pub fn apply_ffu(path: &Path, dev: &str) -> io::Result<()> {
    dism(&format!(
        "/Apply-Ffu /ApplyDrive:{dev} /ImageFile:\"{}\"",
        path.display()
    ))
}

/// Dumps the whole device to `out`: `.vhd` gets a fixed-VHD footer, `.vhdx`
/// becomes a dynamic VHDX, `.ffu` is captured with dism (Windows only).
pub fn save_image(
    dev: &str,
    size: u64,
    out: &Path,
    progress: &dyn Fn(f32),
    cancel: &AtomicBool,
) -> io::Result<()> {
    if is_ffu(out) {
        progress(0.0);
        let r = dism(&format!(
            "/Capture-Ffu /CaptureDrive:{dev} /ImageFile:\"{}\" /Name:\"disk\" /Description:\"Created by rustfus\"",
            out.display()
        ));
        progress(1.0);
        return r;
    }
    let res = (|| {
        let mut src = File::open(dev)?;
        let mut dst = File::create(out)?;
        if ext(out) == "vhdx" {
            let tick = |done: u64| {
                progress(done as f32 / size as f32);
                if cancel.load(Ordering::Relaxed) {
                    Err(interrupted())
                } else {
                    Ok(())
                }
            };
            crate::vhdx::save(&mut src, &mut dst, size, &tick)?;
            return dst.sync_all();
        }
        let mut buf = vec![0u8; CHUNK];
        let mut off = 0u64;
        while off < size {
            if cancel.load(Ordering::Relaxed) {
                return Err(interrupted());
            }
            let len = (size - off).min(CHUNK as u64) as usize;
            src.read_exact(&mut buf[..len])?;
            dst.write_all(&buf[..len])?;
            off += len as u64;
            progress(off as f32 / size as f32);
        }
        if ext(out) == "vhd" {
            dst.write_all(&vhd_footer(size))?;
        }
        dst.sync_all()
    })();
    if res.is_err() {
        std::fs::remove_file(out).ok();
    }
    res
}

#[cfg(test)]
#[path = "../tests/unit/image.rs"]
mod tests;
