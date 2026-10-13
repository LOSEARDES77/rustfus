// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
#![allow(dead_code)]
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
            match u32::from_be_bytes(footer[60..64].try_into().unwrap()) {
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
#[cfg(target_os = "linux")]
fn drop_cache(f: &File) {
    use std::os::fd::AsRawFd;
    unsafe {
        libc::posix_fadvise(f.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED);
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
    buf.iter_mut()
        .zip(b.iter().cycle())
        .for_each(|(d, s)| *d = *s);
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
                #[cfg(target_os = "linux")]
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
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("rustfus-image-{}-{name}", std::process::id()))
    }

    fn data() -> Vec<u8> {
        (0..300_000u32)
            .map(|i| (i % 251) as u8 ^ (i >> 10) as u8)
            .collect()
    }

    fn roundtrip(name: &str, bytes: &[u8]) -> Vec<u8> {
        let p = tmp(name);
        std::fs::write(&p, bytes).unwrap();
        let mut s = open(&p).unwrap();
        let mut out = vec![];
        s.reader.read_to_end(&mut out).unwrap();
        assert_eq!(s.total, bytes.len() as u64);
        assert!(s.consumed.load(Ordering::Relaxed) > 0);
        std::fs::remove_file(&p).ok();
        out
    }

    #[test]
    fn compression_formats_roundtrip() {
        let d = data();
        let mut gz = flate2::write::GzEncoder::new(vec![], flate2::Compression::fast());
        gz.write_all(&d).unwrap();
        assert_eq!(roundtrip("a.img.gz", &gz.finish().unwrap()), d);

        let mut bz = bzip2::write::BzEncoder::new(vec![], bzip2::Compression::fast());
        bz.write_all(&d).unwrap();
        assert_eq!(roundtrip("a.img.bz2", &bz.finish().unwrap()), d);

        let mut xz =
            lzma_rust2::XzWriter::new(vec![], lzma_rust2::XzOptions::with_preset(1)).unwrap();
        xz.write_all(&d).unwrap();
        assert_eq!(roundtrip("a.img.xz", &xz.finish().unwrap()), d);

        let opts = lzma_rust2::LzmaOptions::with_preset(1);
        let mut lz =
            lzma_rust2::LzmaWriter::new_use_header(vec![], &opts, Some(d.len() as u64)).unwrap();
        lz.write_all(&d).unwrap();
        assert_eq!(roundtrip("a.img.lzma", &lz.finish().unwrap()), d);

        let zst =
            ruzstd::encoding::compress_to_vec(&d[..], ruzstd::encoding::CompressionLevel::Fastest);
        assert_eq!(roundtrip("a.img.zst", &zst), d);

        for method in [
            zip::CompressionMethod::Stored,
            zip::CompressionMethod::Deflated,
        ] {
            let mut z = zip::ZipWriter::new(io::Cursor::new(vec![]));
            let o = zip::write::SimpleFileOptions::default().compression_method(method);
            z.add_directory("dir/", o).unwrap();
            z.start_file("dir/disk.img", o).unwrap();
            z.write_all(&d).unwrap();
            assert_eq!(roundtrip("a.zip", &z.finish().unwrap().into_inner()), d);
        }
        assert!(is_compressed(Path::new("x.IMG.XZ")) && !is_compressed(Path::new("x.vhd")));
    }

    #[test]
    fn vhd_fixed_stripped_differencing_rejected() {
        let d = vec![7u8; 4096];
        let mut v = d.clone();
        v.extend_from_slice(&vhd_footer(d.len() as u64));
        assert_eq!(roundtrip("fixed.vhd", &v), d);

        let n = v.len();
        v[n - 512 + 60..n - 512 + 64].copy_from_slice(&4u32.to_be_bytes());
        let p = tmp("diff.vhd");
        std::fs::write(&p, &v).unwrap();
        let err = open(&p).err().unwrap();
        assert!(err.to_string().contains("Differencing"));
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn hashes_abc() {
        let p = tmp("abc");
        std::fs::write(&p, b"abc").unwrap();
        let h = hashes(&p, &|_| {}, &AtomicBool::new(false)).unwrap();
        std::fs::remove_file(&p).ok();
        assert_eq!(h[0], "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(h[1], "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            h[2],
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(h[3].starts_with("ddaf35a193617aba") && h[3].ends_with("a54ca49f"));
        assert!(hashes(&p, &|_| {}, &AtomicBool::new(true)).is_err());
    }

    #[test]
    fn bad_blocks_clean_file() {
        let p = tmp("bb.bin");
        File::create(&p).unwrap().set_len((5 << 20) + 1536).unwrap();
        for (mode, (label, _)) in BAD_BLOCK_MODES.iter().enumerate() {
            let r = bad_blocks(
                p.to_str().unwrap(),
                (5 << 20) + 1536,
                mode,
                &|_, _| {},
                &AtomicBool::new(false),
            )
            .unwrap();
            assert_eq!(r, BadBlocks::default(), "{label}");
        }
        std::fs::remove_file(&p).ok();
        let mut b = [0u8; 7];
        pattern_fill(&mut b, 0x1c71c7);
        assert_eq!(b, [0x1c, 0x71, 0xc7, 0x1c, 0x71, 0xc7, 0x1c]);
        pattern_fill(&mut b, 0);
        assert_eq!(b, [0; 7]);
    }

    fn qemu_img(args: &[&str]) -> Option<bool> {
        std::process::Command::new("qemu-img")
            .args(args)
            .output()
            .ok()
            .map(|o| o.status.success())
    }

    fn read_all(p: &Path) -> Vec<u8> {
        let mut s = open(p).unwrap();
        let mut out = vec![];
        s.reader.read_to_end(&mut out).unwrap();
        assert_eq!(s.size, Some(out.len() as u64));
        out
    }

    #[test]
    fn save_vhdx_roundtrip() {
        let (dev, out) = (tmp("vdev.bin"), tmp("out.vhdx"));
        let mut d = vec![0u8; 70 << 20];
        for (at, len) in [(0, 4096), (20 << 20, 1 << 20), ((70 << 20) - 512, 512)] {
            d[at..at + len]
                .iter_mut()
                .enumerate()
                .for_each(|(i, b)| *b = (i % 251) as u8 + 1);
        }
        std::fs::write(&dev, &d).unwrap();
        save_image(
            dev.to_str().unwrap(),
            d.len() as u64,
            &out,
            &|_| {},
            &AtomicBool::new(false),
        )
        .unwrap();
        let v = std::fs::read(&out).unwrap();
        // 3 blocks of 32 MiB, the middle one all zero: 2 payload blocks + 4 MiB of headers/log/metadata/BAT.
        assert_eq!(v.len(), (4 + 64) << 20);
        assert_eq!(&v[..8], b"vhdxfile");
        for off in [64 << 10, 128 << 10] {
            let h = &v[off..off + 4096];
            let crc = u32::from_le_bytes(h[4..8].try_into().unwrap());
            let mut z = h.to_vec();
            z[4..8].fill(0);
            assert_eq!(crc, crate::vhdx::crc32c(&z));
        }
        for off in [192 << 10, 256 << 10] {
            let mut r = v[off..off + (64 << 10)].to_vec();
            let crc = u32::from_le_bytes(r[4..8].try_into().unwrap());
            r[4..8].fill(0);
            assert_eq!(crc, crate::vhdx::crc32c(&r));
        }
        assert!(read_all(&out) == d);
        let (o, ds) = (out.to_str().unwrap(), dev.to_str().unwrap());
        if qemu_img(&["--version"]).is_some() {
            assert_eq!(qemu_img(&["check", "-f", "vhdx", o]), Some(true));
            assert_eq!(
                qemu_img(&["compare", "-f", "vhdx", "-F", "raw", o, ds]),
                Some(true)
            );
        }
        std::fs::remove_file(&dev).ok();
        std::fs::remove_file(&out).ok();
    }

    #[test]
    fn dynamic_vhd_and_qemu_vhdx_read() {
        let (raw, vhd, vhdx) = (tmp("q.raw"), tmp("q.vhd"), tmp("q.vhdx"));
        let mut d = vec![0u8; 9 << 20];
        d[..1000].fill(0xab);
        d[(5 << 20) + 7..(6 << 20) + 9]
            .iter_mut()
            .enumerate()
            .for_each(|(i, b)| *b = i as u8);
        std::fs::write(&raw, &d).unwrap();
        let r = raw.to_str().unwrap();
        if qemu_img(&[
            "convert",
            "-f",
            "raw",
            "-O",
            "vpc",
            "-o",
            "subformat=dynamic,force_size=on",
            r,
            vhd.to_str().unwrap(),
        ]) == Some(true)
        {
            assert!(read_all(&vhd) == d);
            assert_eq!(
                qemu_img(&[
                    "convert",
                    "-f",
                    "raw",
                    "-O",
                    "vhdx",
                    r,
                    vhdx.to_str().unwrap()
                ]),
                Some(true)
            );
            assert!(read_all(&vhdx) == d);
        } else {
            // Hand-made dynamic VHD: 2 MiB disk, 1 MiB blocks, only block 1 allocated.
            let mut f = vhd_footer(2 << 20);
            f[16..24].copy_from_slice(&512u64.to_be_bytes());
            f[60..64].copy_from_slice(&3u32.to_be_bytes());
            let mut v = f.to_vec();
            let mut dh = vec![0u8; 1024];
            dh[..8].copy_from_slice(b"cxsparse");
            dh[16..24].copy_from_slice(&1536u64.to_be_bytes());
            dh[28..32].copy_from_slice(&2u32.to_be_bytes());
            dh[32..36].copy_from_slice(&(1u32 << 20).to_be_bytes());
            v.extend_from_slice(&dh);
            let mut bat = vec![0xffu8; 512];
            bat[4..8].copy_from_slice(&4u32.to_be_bytes()); // sector 4 = byte 2048
            v.extend_from_slice(&bat);
            v.extend_from_slice(&[0u8; 512]); // bitmap
            v.extend((0..1u32 << 20).map(|i| (i % 7) as u8));
            v.extend_from_slice(&f);
            std::fs::write(&vhd, &v).unwrap();
            let got = read_all(&vhd);
            assert!(
                got[..1 << 20].iter().all(|&b| b == 0)
                    && got[1 << 20..] == v[2560..2560 + (1 << 20)]
            );
        }
        for p in [raw, vhd, vhdx] {
            std::fs::remove_file(&p).ok();
        }
    }

    #[test]
    fn ffu_needs_windows() {
        assert!(is_ffu(Path::new("a.FFU")) && !is_ffu(Path::new("a.vhdx")));
        #[cfg(not(windows))]
        assert!(apply_ffu(Path::new("a.ffu"), "/dev/null").is_err());
    }

    #[test]
    fn save_vhd_roundtrip() {
        let (dev, out) = (tmp("dev.bin"), tmp("out.vhd"));
        let d: Vec<u8> = (0..(5u32 << 20) + 1024).map(|i| (i % 253) as u8).collect();
        std::fs::write(&dev, &d).unwrap();
        save_image(
            dev.to_str().unwrap(),
            d.len() as u64,
            &out,
            &|_| {},
            &AtomicBool::new(false),
        )
        .unwrap();
        let v = std::fs::read(&out).unwrap();
        assert_eq!(v.len(), d.len() + 512);
        let mut footer = v[d.len()..].to_vec();
        assert_eq!(&footer[..8], b"conectix");
        let stored = u32::from_be_bytes(footer[64..68].try_into().unwrap());
        footer[64..68].fill(0);
        assert_eq!(stored, !footer.iter().map(|&b| b as u32).sum::<u32>());
        assert_eq!(roundtrip("out2.vhd", &v), d);
        assert_eq!(vhd_geometry(4 << 30), (8322, 16, 63));
        std::fs::remove_file(&dev).ok();
        std::fs::remove_file(&out).ok();
    }
}
