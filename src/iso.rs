// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
#![allow(dead_code)]
use std::collections::HashSet;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

const BLK: u64 = 2048;

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

fn le32(b: &[u8], o: usize) -> u32 {
    b.get(o..o + 4).map_or(0, |s| u32::from_le_bytes(s.try_into().unwrap()))
}

fn read_at(f: &File, buf: &mut [u8], off: u64) -> io::Result<usize> {
    #[cfg(unix)]
    return std::os::unix::fs::FileExt::read_at(f, buf, off);
    #[cfg(windows)]
    return std::os::windows::fs::FileExt::seek_read(f, buf, off);
}

fn read_exact_at(f: &File, buf: &mut [u8], mut off: u64) -> io::Result<()> {
    let mut n = 0;
    while n < buf.len() {
        match read_at(f, &mut buf[n..], off) {
            Ok(0) => return Err(bad("short read")),
            Ok(k) => (n, off) = (n + k, off + k as u64),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[derive(Default, Debug, Clone, Copy)]
pub struct ElTorito {
    pub bios: bool,
    pub efi: bool,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    extents: Vec<(u64, u64)>,
    link: Option<String>,
}

impl Entry {
    pub fn symlink(&self) -> Option<&str> {
        self.link.as_deref()
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Names {
    Rr,
    Joliet,
    Plain,
}

pub struct Iso {
    f: File,
    vol: String,
    root: (u64, u64),
    names: Names,
    boot: ElTorito,
    udf: Option<Udf>,
}

struct Rec<'a> {
    ext: u64,
    len: u64,
    flags: u8,
    id: &'a [u8],
    su: &'a [u8],
}

fn parse_rec(r: &[u8]) -> io::Result<Rec<'_>> {
    let n = *r.get(32).ok_or_else(|| bad("bad directory record"))? as usize;
    let id = r.get(33..33 + n).ok_or_else(|| bad("bad directory record"))?;
    let su = r.get(33 + n + (1 - n % 2)..).unwrap_or(&[]);
    Ok(Rec { ext: le32(r, 2) as u64 * BLK, len: le32(r, 10) as u64, flags: r[25], id, su })
}

#[derive(Default)]
struct Susp {
    sp: bool,
    name: Option<String>,
    link: Option<String>,
    relocated: bool,
    child: Option<u64>,
}

fn boot_catalog(f: &File, lba: u32) -> ElTorito {
    let mut b = [0u8; 2048];
    let mut et = ElTorito::default();
    if read_exact_at(f, &mut b, lba as u64 * BLK).is_err() || b[0] != 1 || b[30..32] != [0x55, 0xAA] {
        return et;
    }
    let mut plat = b[1];
    for r in b.chunks(32) {
        match r[0] {
            0x88 if plat == 0 => et.bios = true,
            0x88 if plat == 0xef => et.efi = true,
            0x90 | 0x91 => plat = r[1],
            _ => {}
        }
    }
    et
}

impl Iso {
    /// ISO9660 when it has a usable PVD/root, else UDF (UDF-only images, or bridge images with a broken ISO side).
    pub fn open(path: &Path) -> io::Result<Iso> {
        let f = File::open(path)?;
        Self::open_iso9660(f.try_clone()?).or_else(|e| Self::open_udf(f).map_err(|_| e))
    }

    fn open_iso9660(f: File) -> io::Result<Iso> {
        let root_of = |vd: &[u8]| parse_rec(&vd[156..190]).map(|r| (r.ext, r.len));
        let mut vd = [0u8; 2048];
        let (mut pvd, mut joliet, mut boot) = (None, None, ElTorito::default());
        for s in 16..256 {
            read_exact_at(&f, &mut vd, s * BLK)?;
            if &vd[1..6] != b"CD001" {
                return Err(bad("not an ISO9660 image"));
            }
            match vd[0] {
                0 if &vd[7..30] == b"EL TORITO SPECIFICATION" => boot = boot_catalog(&f, le32(&vd, 71)),
                1 if pvd.is_none() => pvd = Some((root_of(&vd)?, String::from_utf8_lossy(&vd[40..72]).trim().to_string())),
                2 if vd[88..90] == [0x25, 0x2f] && matches!(vd[90], 0x40 | 0x43 | 0x45) => joliet = Some(root_of(&vd)?),
                255 => break,
                _ => {}
            }
        }
        let (root, vol) = pvd.ok_or_else(|| bad("no primary volume descriptor"))?;
        let mut iso = Iso { f, vol, root, names: Names::Plain, boot, udf: None };
        let mut sec = [0u8; 2048];
        read_exact_at(&iso.f, &mut sec, root.0)?;
        let dot = parse_rec(sec.get(..sec[0] as usize).unwrap_or(&[]))?;
        if iso.susp(dot.su)?.sp {
            iso.names = Names::Rr;
        } else if let Some(j) = joliet {
            (iso.root, iso.names) = (j, Names::Joliet);
        }
        Ok(iso)
    }

    pub fn volume_id(&self) -> String {
        self.vol.clone()
    }

    pub fn el_torito(&self) -> ElTorito {
        self.boot
    }

    pub fn entries(&self) -> io::Result<Vec<Entry>> {
        let mut out = Vec::new();
        match &self.udf {
            Some(u) => self.udf_walk(u, u.icb(&self.f, u.root)?, "", &mut out, &mut HashSet::new(), 0)?,
            None => self.walk(self.root, "", &mut out, &mut HashSet::new(), 0)?,
        }
        Ok(out)
    }

    pub fn reader<'a>(&'a self, e: &Entry) -> io::Result<impl Read + 'a> {
        if e.is_dir {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "is a directory"));
        }
        Ok(ExtReader { f: &self.f, ext: e.extents.clone(), i: 0, pos: 0 })
    }

    /// Reads up to `buf.len()` bytes at `offset` within file `e` (fills `buf` unless EOF; 0 at EOF).
    pub fn read_at(&self, e: &Entry, mut offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        if e.is_dir {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "is a directory"));
        }
        let mut r = ExtReader { f: &self.f, ext: e.extents.clone(), i: 0, pos: 0 };
        while r.ext.get(r.i).is_some_and(|x| offset >= x.1) {
            offset -= r.ext[r.i].1;
            r.i += 1;
        }
        r.pos = offset;
        let mut n = 0;
        while n < buf.len() {
            match r.read(&mut buf[n..])? {
                0 => break,
                k => n += k,
            }
        }
        Ok(n)
    }

    fn susp(&self, su: &[u8]) -> io::Result<Susp> {
        let mut s = Susp::default();
        let (mut name, mut link, mut cont) = (None::<Vec<u8>>, None::<String>, false);
        let mut area = su.to_vec();
        for _ in 0..16 {
            let (mut p, mut ce) = (0, None);
            while p + 4 <= area.len() {
                let l = area[p + 2] as usize;
                if l < 4 || p + l > area.len() {
                    break;
                }
                let e = &area[p..p + l];
                match &e[..2] {
                    b"SP" => s.sp = l >= 7 && e[4..6] == [0xbe, 0xef],
                    b"NM" if l >= 5 && e[4] & 6 == 0 => name.get_or_insert_default().extend(&e[5..]),
                    b"SL" if l >= 5 => {
                        let link = link.get_or_insert_default();
                        let mut c = 5;
                        while let (Some(&cf), Some(&cl)) = (e.get(c), e.get(c + 1)) {
                            let Some(body) = e.get(c + 2..c + 2 + cl as usize) else { break };
                            if !link.is_empty() && !cont && !link.ends_with('/') {
                                link.push('/');
                            }
                            match cf & 0x0e {
                                2 => link.push('.'),
                                4 => link.push_str(".."),
                                8 => link.push('/'),
                                _ => link.push_str(&String::from_utf8_lossy(body)),
                            }
                            cont = cf & 1 != 0;
                            c += 2 + cl as usize;
                        }
                    }
                    b"CE" if l >= 28 => ce = Some((le32(e, 4) as u64 * BLK + le32(e, 12) as u64, le32(e, 20) as usize)),
                    b"RE" => s.relocated = true,
                    b"CL" if l >= 12 => s.child = Some(le32(e, 4) as u64 * BLK),
                    b"ST" => break,
                    _ => {}
                }
                p += l;
            }
            let Some((off, len)) = ce else { break };
            if len > 1 << 16 {
                return Err(bad("bad SUSP continuation"));
            }
            area = vec![0; len];
            read_exact_at(&self.f, &mut area, off)?;
        }
        s.name = name.map(|n| String::from_utf8_lossy(&n).into_owned());
        s.link = link;
        Ok(s)
    }

    fn walk(&self, (ext, len): (u64, u64), prefix: &str, out: &mut Vec<Entry>, seen: &mut HashSet<u64>, depth: u32) -> io::Result<()> {
        if depth > 64 {
            return Err(bad("directory tree too deep"));
        }
        if !seen.insert(ext) {
            return Ok(());
        }
        if len > 1 << 28 {
            return Err(bad("directory too large"));
        }
        let mut buf = vec![0; len as usize];
        read_exact_at(&self.f, &mut buf, ext)?;
        let (mut local, mut last_id, mut more) = (Vec::<Entry>::new(), &[][..], false);
        let mut pos = 0;
        while pos < buf.len() {
            let n = buf[pos] as usize;
            if n == 0 {
                pos = (pos / 2048 + 1) * 2048;
                continue;
            }
            let r = parse_rec(buf.get(pos..pos + n).ok_or_else(|| bad("bad directory record"))?)?;
            pos += n;
            if r.id == [0] || r.id == [1] || r.flags & 4 != 0 {
                continue;
            }
            if more && r.id == last_id {
                if let Some(e) = local.last_mut() {
                    e.extents.push((r.ext, r.len));
                    e.size += r.len;
                }
                more = r.flags & 0x80 != 0;
                continue;
            }
            (last_id, more) = (r.id, r.flags & 0x80 != 0);
            let s = if self.names == Names::Rr { self.susp(r.su)? } else { Susp::default() };
            if s.relocated {
                continue;
            }
            let name = match (s.name, self.names) {
                (Some(n), _) => n,
                (None, Names::Joliet) => {
                    let u: Vec<u16> = r.id.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
                    let n = String::from_utf16_lossy(&u);
                    n.split(';').next().unwrap_or("").to_string()
                }
                _ => {
                    let n = String::from_utf8_lossy(r.id);
                    n.split(';').next().unwrap_or("").trim_end_matches('.').to_string()
                }
            };
            if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0']) {
                continue;
            }
            let path = format!("{prefix}{name}");
            let mut e = Entry { path, is_dir: r.flags & 2 != 0, size: r.len, extents: vec![(r.ext, r.len)], link: s.link };
            if let Some(c) = s.child {
                let mut sec = [0u8; 2048];
                read_exact_at(&self.f, &mut sec, c)?;
                let dot = parse_rec(sec.get(..sec[0] as usize).unwrap_or(&[]))?;
                (e.is_dir, e.extents) = (true, vec![(c, dot.len)]);
            }
            if e.is_dir {
                e.size = 0;
            } else if e.link.is_some() {
                (e.size, e.extents) = (0, vec![]);
            }
            local.push(e);
        }
        out.extend(local.iter().cloned());
        for e in local.iter().filter(|e| e.is_dir) {
            self.walk(e.extents[0], &format!("{}/", e.path), out, seen, depth + 1)?;
        }
        Ok(())
    }
}

struct ExtReader<'a> {
    f: &'a File,
    ext: Vec<(u64, u64)>,
    i: usize,
    pos: u64,
}

impl Read for ExtReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        while let Some(&(off, len)) = self.ext.get(self.i) {
            if self.pos >= len {
                (self.i, self.pos) = (self.i + 1, 0);
                continue;
            }
            let n = (buf.len() as u64).min(len - self.pos) as usize;
            if n == 0 {
                return Ok(0);
            }
            let k = if off == ZERO {
                buf[..n].fill(0);
                n
            } else {
                read_at(self.f, &mut buf[..n], off + self.pos)?
            };
            if k == 0 {
                return Err(bad("short read"));
            }
            self.pos += k as u64;
            return Ok(k);
        }
        Ok(0)
    }
}

// ---- UDF (ECMA-167 / OSTA UDF 1.02-2.60), read-only. 2048-byte blocks only. ----

/// Extent offset meaning "reads as zeros" (allocated-but-unrecorded / sparse extents).
const ZERO: u64 = u64::MAX;

fn le16(b: &[u8], o: usize) -> u16 {
    b.get(o..o + 2).map_or(0, |s| u16::from_le_bytes([s[0], s[1]]))
}

fn le64(b: &[u8], o: usize) -> u64 {
    b.get(o..o + 8).map_or(0, |s| u64::from_le_bytes(s.try_into().unwrap()))
}

/// Checks a descriptor tag's identifier and header checksum.
fn tag(b: &[u8], id: u16) -> io::Result<()> {
    let ok = b.len() >= 16 && le16(b, 0) == id && (0..16).filter(|&i| i != 4).fold(0u8, |s, i| s.wrapping_add(b[i])) == b[4];
    if !ok {
        return Err(bad("bad UDF descriptor tag"));
    }
    Ok(())
}

/// OSTA CS0 compressed Unicode (8- or 16-bit).
fn cs0(b: &[u8]) -> String {
    match b.split_first() {
        Some((8 | 254, r)) => r.iter().map(|&c| c as char).collect(),
        Some((16 | 255, r)) => String::from_utf16_lossy(&r.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect::<Vec<_>>()),
        _ => String::new(),
    }
}

fn dstring(b: &[u8]) -> String {
    let n = b.last().map_or(0, |&n| n as usize);
    if n < b.len() { cs0(&b[..n]) } else { String::new() }
}

enum Part {
    /// Byte offset of a physical (or sparable) partition.
    Phys(u64),
    /// UDF 2.50+ metadata partition: physical extents of the metadata file.
    Meta(Vec<(u64, u64)>),
}

struct Udf {
    parts: Vec<Part>,
    root: (u16, u32),
}

struct Node {
    at: u64,
    kind: u8,
    size: u64,
    ext: Vec<(u64, u64)>,
}

impl Udf {
    /// Maps `len` bytes at logical block `lbn` of partition reference `part` to physical extents.
    fn map(&self, part: u16, lbn: u32, len: u64) -> io::Result<Vec<(u64, u64)>> {
        match self.parts.get(part as usize) {
            Some(Part::Phys(s)) => Ok(vec![(s + lbn as u64 * BLK, len)]),
            Some(Part::Meta(ext)) => {
                let (mut off, mut left, mut out) = (lbn as u64 * BLK, len, vec![]);
                for &(o, l) in ext {
                    if left == 0 {
                        break;
                    }
                    if off >= l {
                        off -= l;
                        continue;
                    }
                    if o == ZERO {
                        return Err(bad("UDF metadata block not recorded"));
                    }
                    let n = (l - off).min(left);
                    out.push((o + off, n));
                    (left, off) = (left - n, 0);
                }
                if left > 0 {
                    return Err(bad("UDF block outside metadata partition"));
                }
                Ok(out)
            }
            None => Err(bad("bad UDF partition reference")),
        }
    }

    fn block(&self, f: &File, part: u16, lbn: u32) -> io::Result<(u64, [u8; 2048])> {
        let off = self.map(part, lbn, BLK)?[0].0;
        let mut b = [0u8; 2048];
        read_exact_at(f, &mut b, off)?;
        Ok((off, b))
    }

    /// Reads a File Entry / Extended File Entry and resolves its data to physical extents.
    fn icb(&self, f: &File, (part, lbn): (u16, u32)) -> io::Result<Node> {
        let (at, b) = self.block(f, part, lbn)?;
        let id = le16(&b, 0);
        let hdr = match id {
            261 => 176,
            266 => 216,
            _ => return Err(bad("bad UDF file entry")),
        };
        tag(&b, id)?;
        let (l_ea, l_ad, size) = (le32(&b, hdr - 8) as usize, le32(&b, hdr - 4) as usize, le64(&b, 56));
        let start = hdr.checked_add(l_ea).ok_or_else(|| bad("bad UDF file entry"))?;
        let ads = b.get(start..start.saturating_add(l_ad)).ok_or_else(|| bad("bad UDF file entry"))?;
        let ext = match le16(&b, 34) & 7 {
            3 if size <= l_ad as u64 => vec![(at + start as u64, size)],
            3 => return Err(bad("bad UDF embedded data")),
            t => self.ads(f, ads, t, part, size)?,
        };
        Ok(Node { at, kind: b[27], size, ext })
    }

    /// Short (0), long (1) or extended (2) allocation descriptors, following continuation extents.
    fn ads(&self, f: &File, ads: &[u8], t: u16, part: u16, size: u64) -> io::Result<Vec<(u64, u64)>> {
        let sz = match t {
            0 => 8,
            1 => 16,
            2 => 20,
            _ => return Err(bad("bad UDF allocation type")),
        };
        let (mut out, mut left, mut buf, mut hops) = (Vec::new(), size, ads.to_vec(), 0);
        'next: while left > 0 {
            for d in buf.chunks_exact(sz) {
                let raw = le32(d, 0);
                let (ty, len) = (raw >> 30, (raw & 0x3fff_ffff) as u64);
                if len == 0 {
                    break;
                }
                let (lbn, p) = match t {
                    0 => (le32(d, 4), part),
                    1 => (le32(d, 4), le16(d, 8)),
                    _ => (le32(d, 12), le16(d, 16)),
                };
                if ty == 3 {
                    hops += 1;
                    if hops > 4096 {
                        return Err(bad("too many UDF allocation extents"));
                    }
                    let (_, b) = self.block(f, p, lbn)?;
                    tag(&b, 258)?;
                    let l = le32(&b, 20) as usize;
                    buf = b.get(24..24usize.saturating_add(l)).ok_or_else(|| bad("bad UDF allocation extent"))?.to_vec();
                    continue 'next;
                }
                let n = len.min(left);
                if ty == 0 {
                    out.extend(self.map(p, lbn, n)?);
                } else {
                    out.push((ZERO, n));
                }
                left -= n;
                if left == 0 || out.len() > 1 << 20 {
                    break;
                }
            }
            break;
        }
        if left > 0 {
            return Err(bad("UDF file extents shorter than its size"));
        }
        Ok(out)
    }
}

fn udf_link(d: &[u8]) -> String {
    let (mut s, mut p) = (String::new(), 0);
    while let (Some(&t), Some(&l)) = (d.get(p), d.get(p + 1)) {
        let id = d.get(p + 4..p + 4 + l as usize).unwrap_or(&[]);
        if !s.is_empty() && !s.ends_with('/') {
            s.push('/');
        }
        match t {
            1 | 2 => s = "/".into(),
            3 => s.push_str(".."),
            4 => s.push('.'),
            _ => s.push_str(&cs0(id)),
        }
        p += 4 + l as usize;
    }
    s
}

impl Iso {
    fn open_udf(f: File) -> io::Result<Iso> {
        let mut b = [0u8; 2048];
        let mut boot = ElTorito::default();
        for s in 16..64 {
            if read_exact_at(&f, &mut b, s * BLK).is_err() {
                break;
            }
            if &b[1..6] == b"CD001" {
                match b[0] {
                    0 if &b[7..30] == b"EL TORITO SPECIFICATION" => boot = boot_catalog(&f, le32(&b, 71)),
                    255 => break,
                    _ => {}
                }
            }
        }
        read_exact_at(&f, &mut b, 256 * BLK)?;
        tag(&b, 2).map_err(|_| bad("not an ISO9660 or UDF image"))?;
        let (vds_len, vds_loc) = (le32(&b, 16) as u64, le32(&b, 20) as u64);
        let (mut pds, mut lvd, mut vol) = (Vec::<(u16, u64)>::new(), None, String::new());
        for s in vds_loc..vds_loc + (vds_len / BLK).min(256) {
            read_exact_at(&f, &mut b, s * BLK)?;
            let id = le16(&b, 0);
            if tag(&b, id).is_err() {
                break;
            }
            match id {
                1 if vol.is_empty() => vol = dstring(&b[24..56]),
                5 => pds.push((le16(&b, 22), le32(&b, 188) as u64 * BLK)),
                6 if lvd.is_none() => lvd = Some(b),
                8 => break,
                _ => {}
            }
        }
        let lvd = lvd.ok_or_else(|| bad("no UDF logical volume descriptor"))?;
        if le32(&lvd, 212) != 2048 {
            return Err(bad("unsupported UDF block size"));
        }
        let lv = dstring(&lvd[84..212]);
        if !lv.is_empty() {
            vol = lv;
        }
        let maps = lvd.get(440..440 + le32(&lvd, 264) as usize).ok_or_else(|| bad("bad UDF partition maps"))?;
        let (mut udf, mut meta, mut p) = (Udf { parts: vec![], root: (0, 0) }, vec![], 0);
        for _ in 0..le32(&lvd, 268).min(64) {
            let m = maps.get(p..).unwrap_or(&[]);
            let (num, len) = match (m.first(), m.get(1)) {
                (Some(1), Some(&l)) if l >= 6 => (le16(m, 4), l),
                (Some(2), Some(&l)) if l >= 64 && m.len() >= 64 => {
                    let ident = &m[5..28];
                    if ident.starts_with(b"*UDF Metadata Partition") {
                        meta.push((udf.parts.len(), le32(m, 40)));
                    } else if ident.starts_with(b"*UDF Virtual Partition") {
                        return Err(bad("UDF virtual (VAT) partitions are not supported"));
                    }
                    (le16(m, 38), l)
                }
                _ => return Err(bad("bad UDF partition map")),
            };
            let start = pds.iter().find(|d| d.0 == num).ok_or_else(|| bad("missing UDF partition descriptor"))?.1;
            udf.parts.push(Part::Phys(start));
            p += len as usize;
        }
        for (i, lbn) in meta {
            let n = udf.icb(&f, (i as u16, lbn))?;
            udf.parts[i] = Part::Meta(n.ext);
        }
        let fsd = &lvd[248..264];
        let (_, b) = udf.block(&f, le16(fsd, 8), le32(fsd, 4))?;
        tag(&b, 256)?;
        udf.root = (le16(&b, 408), le32(&b, 404));
        Ok(Iso { f, vol, root: (0, 0), names: Names::Plain, boot, udf: Some(udf) })
    }

    fn read_node(&self, n: &Node, cap: u64) -> io::Result<Vec<u8>> {
        if n.size > cap {
            return Err(bad("UDF directory too large"));
        }
        let mut v = vec![0; n.size as usize];
        ExtReader { f: &self.f, ext: n.ext.clone(), i: 0, pos: 0 }.read_exact(&mut v)?;
        Ok(v)
    }

    fn udf_walk(&self, u: &Udf, dir: Node, prefix: &str, out: &mut Vec<Entry>, seen: &mut HashSet<u64>, depth: u32) -> io::Result<()> {
        if depth > 64 {
            return Err(bad("directory tree too deep"));
        }
        if !seen.insert(dir.at) {
            return Ok(());
        }
        let buf = self.read_node(&dir, 1 << 28)?;
        let (mut pos, mut subdirs) = (0, vec![]);
        while pos + 38 <= buf.len() {
            let d = &buf[pos..];
            tag(d, 257)?;
            let (ch, l_fi, l_iu) = (d[18], d[19] as usize, le16(d, 36) as usize);
            let name = cs0(d.get(38 + l_iu..38 + l_iu + l_fi).ok_or_else(|| bad("bad UDF file identifier"))?);
            pos += (38 + l_iu + l_fi + 3) & !3;
            if ch & 0x0c != 0 || name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0']) {
                continue; // deleted, parent, or unusable name
            }
            let n = u.icb(&self.f, (le16(d, 28), le32(d, 24)))?;
            let path = format!("{prefix}{name}");
            let mut e = Entry { path, is_dir: ch & 2 != 0, size: n.size, extents: vec![], link: None };
            if e.is_dir {
                e.size = 0;
                subdirs.push((out.len(), n));
            } else if n.kind == 12 {
                (e.size, e.link) = (0, Some(udf_link(&self.read_node(&n, 1 << 16)?)));
            } else {
                e.extents = n.ext;
            }
            out.push(e);
        }
        for (i, n) in subdirs {
            let p = format!("{}/", out[i].path);
            self.udf_walk(u, n, &p, out, seen, depth + 1)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rustfus-iso-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("src")).unwrap();
        d
    }

    /// Builds `dir/out.iso` from `dir/src`; None if xorriso is missing.
    fn mkiso(dir: &Path, args: &[&str]) -> Option<Iso> {
        let iso = dir.join("out.iso");
        let st = Command::new("xorriso").args(["-as", "mkisofs", "-quiet", "-o"]).arg(&iso).args(args).arg(dir.join("src")).stderr(std::process::Stdio::null()).status().ok()?;
        assert!(st.success());
        Some(Iso::open(&iso).unwrap())
    }

    fn check(iso: &Iso, src: &Path) -> Vec<Entry> {
        let es = iso.entries().unwrap();
        for (i, e) in es.iter().enumerate() {
            if let Some((parent, _)) = e.path.rsplit_once('/') {
                assert!(es[..i].iter().any(|p| p.is_dir && p.path == parent), "{} before parent", e.path);
            }
            let p = src.join(&e.path);
            if e.is_dir {
                assert!(p.is_dir(), "{}", e.path);
            } else if e.symlink().is_none() && e.path != "boot.catalog" {
                let mut got = Vec::new();
                iso.reader(e).unwrap().read_to_end(&mut got).unwrap();
                assert_eq!(got, fs::read(&p).unwrap(), "{}", e.path);
                assert_eq!(e.size, got.len() as u64);
            }
        }
        es
    }

    fn populate(src: &Path) -> String {
        let long = "A Very Long Mixed-Case File Name That Exceeds Joliet Limits Maybe.txt".to_string();
        fs::create_dir_all(src.join("EFI/Boot/deep/er")).unwrap();
        fs::write(src.join(&long), b"long").unwrap();
        fs::write(src.join("EFI/Boot/bootx64.efi"), vec![7u8; 5000]).unwrap();
        fs::write(src.join("EFI/Boot/deep/er/x.cfg"), b"").unwrap();
        long
    }

    #[test]
    fn iso_rock_ridge() {
        let d = tmp("rr");
        let long = populate(&d.join("src"));
        #[cfg(unix)]
        std::os::unix::fs::symlink("EFI/Boot/bootx64.efi", d.join("src/link")).unwrap();
        let Some(iso) = mkiso(&d, &["-R", "-V", "RR_TEST"]) else { return };
        assert_eq!(iso.volume_id(), "RR_TEST");
        let es = check(&iso, &d.join("src"));
        let paths: Vec<&str> = es.iter().map(|e| e.path.as_str()).collect();
        assert!(paths.contains(&long.as_str()) && paths.contains(&"EFI/Boot/deep/er/x.cfg"), "{paths:?}");
        #[cfg(unix)]
        assert_eq!(es.iter().find(|e| e.path == "link").unwrap().symlink(), Some("EFI/Boot/bootx64.efi"));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn iso_joliet() {
        let d = tmp("joliet");
        populate(&d.join("src"));
        let Some(iso) = mkiso(&d, &["--norock", "-J", "-joliet-long"]) else { return };
        assert_eq!(iso.names as u8, Names::Joliet as u8);
        let es = check(&iso, &d.join("src"));
        assert!(es.iter().any(|e| e.path == "EFI/Boot/bootx64.efi"));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn iso_el_torito() {
        let d = tmp("boot");
        fs::write(d.join("src/boot.img"), vec![1u8; 2048]).unwrap();
        fs::write(d.join("src/efi.img"), vec![2u8; 4096]).unwrap();
        let efi = ["-R", "-b", "boot.img", "-no-emul-boot", "-eltorito-alt-boot", "-e", "efi.img", "-no-emul-boot"];
        let Some(iso) = mkiso(&d, &efi) else { return };
        let et = iso.el_torito();
        assert!(et.bios && et.efi, "{et:?}");
        check(&iso, &d.join("src"));
        fs::remove_file(d.join("out.iso")).unwrap();
        let iso = mkiso(&d, &efi[..4]).unwrap();
        let et = iso.el_torito();
        assert!(et.bios && !et.efi, "{et:?}");
        let _ = fs::remove_dir_all(&d);
    }

    fn rec(ext: u32, len: u32, flags: u8, id: &[u8]) -> Vec<u8> {
        let mut r = vec![0u8; 33];
        r[2..6].copy_from_slice(&ext.to_le_bytes());
        r[10..14].copy_from_slice(&len.to_le_bytes());
        (r[25], r[32]) = (flags, id.len() as u8);
        r.extend(id);
        if r.len() % 2 == 1 {
            r.push(0);
        }
        r[0] = r.len() as u8;
        r
    }

    // xorriso can't split small files into multiple extents, so synthesize one.
    #[test]
    fn iso_multi_extent_and_malformed() {
        let mut img = vec![0u8; 21 * 2048];
        let pvd = &mut img[16 * 2048..17 * 2048];
        (pvd[0], pvd[6]) = (1, 1);
        pvd[1..6].copy_from_slice(b"CD001");
        pvd[40..72].copy_from_slice(&[b' '; 32]);
        pvd[40..44].copy_from_slice(b"MULT");
        pvd[156..190].copy_from_slice(&rec(18, 2048, 2, &[0]));
        img[17 * 2048] = 255;
        img[17 * 2048 + 1..17 * 2048 + 6].copy_from_slice(b"CD001");
        let dir: Vec<u8> = [rec(18, 2048, 2, &[0]), rec(18, 2048, 2, &[1]), rec(19, 2048, 0x80, b"BIG.BIN;1"), rec(20, 100, 0, b"BIG.BIN;1"), rec(20, 1, 4, b"ASSOC.;1")].concat();
        img[18 * 2048..18 * 2048 + dir.len()].copy_from_slice(&dir);
        img[19 * 2048..20 * 2048].fill(0xaa);
        img[20 * 2048..20 * 2048 + 100].fill(0xbb);
        let d = tmp("multi");
        let p = d.join("m.iso");
        fs::write(&p, &img).unwrap();
        let iso = Iso::open(&p).unwrap();
        assert_eq!(iso.volume_id(), "MULT");
        let es = iso.entries().unwrap();
        assert_eq!(es.len(), 1);
        assert_eq!((es[0].path.as_str(), es[0].size), ("BIG.BIN", 2148));
        let mut got = Vec::new();
        iso.reader(&es[0]).unwrap().read_to_end(&mut got).unwrap();
        assert!(got.len() == 2148 && got[..2048].iter().all(|&b| b == 0xaa) && got[2048..].iter().all(|&b| b == 0xbb));
        let mut buf = [0u8; 8];
        assert_eq!(iso.read_at(&es[0], 2044, &mut buf).unwrap(), 8);
        assert_eq!(buf, [0xaa, 0xaa, 0xaa, 0xaa, 0xbb, 0xbb, 0xbb, 0xbb]);
        assert_eq!(iso.read_at(&es[0], 2100, &mut buf).unwrap(), 8);
        assert_eq!(buf, [0xbb; 8]);
        assert_eq!(iso.read_at(&es[0], 2144, &mut buf).unwrap(), 4);
        assert_eq!(iso.read_at(&es[0], 9999, &mut buf).unwrap(), 0);
        fs::write(&p, &img[..19 * 2048 + 10]).unwrap();
        let iso = Iso::open(&p).unwrap();
        let mut got = Vec::new();
        assert!(iso.reader(&iso.entries().unwrap()[0]).unwrap().read_to_end(&mut got).is_err());
        img[18 * 2048 + 100] = 250;
        fs::write(&p, &img).unwrap();
        assert_eq!(Iso::open(&p).unwrap().entries().unwrap_err().kind(), io::ErrorKind::InvalidData);
        fs::write(&p, &img[..17 * 2048]).unwrap();
        assert!(Iso::open(&p).is_err());
        let _ = fs::remove_dir_all(&d);
    }

    fn sorted(es: &[Entry]) -> Vec<(String, bool, u64)> {
        let mut v: Vec<_> = es.iter().map(|e| (e.path.clone(), e.is_dir, e.size)).collect();
        v.sort();
        v
    }

    #[test]
    fn iso_udf_genisoimage() {
        let d = tmp("udfgen");
        let src = d.join("src");
        let long = populate(&src);
        let uni = "ünïcødé ✓ 名前 with a rather long name to exceed 64 chars of joliet.bin";
        fs::create_dir_all(src.join("sources/Ünï")).unwrap();
        fs::write(src.join("sources").join(uni), (0..3_000_000u32).map(|i| (i % 251) as u8).collect::<Vec<_>>()).unwrap();
        fs::write(src.join("sources/Ünï/empty"), b"").unwrap();
        let out = d.join("out.iso");
        let args = ["-quiet", "-udf", "-R", "-input-charset", "utf-8", "-V", "GEN_UDF", "-o"];
        let Ok(st) = Command::new("genisoimage").args(args).arg(&out).arg(&src).stderr(std::process::Stdio::null()).status() else { return };
        assert!(st.success());
        let udf = Iso::open_udf(File::open(&out).unwrap()).unwrap();
        assert!(udf.udf.is_some());
        assert_eq!(udf.volume_id(), "GEN_UDF");
        let es = check(&udf, &src);
        assert!(es.iter().any(|e| e.path == long) && es.iter().any(|e| e.path == format!("sources/{uni}")), "{es:?}");
        let iso = Iso::open(&out).unwrap();
        assert!(iso.udf.is_none());
        assert_eq!(sorted(&es), sorted(&iso.entries().unwrap()));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn iso_udf_mkudffs() {
        let d = tmp("mkudffs");
        let src = d.join("src");
        populate(&src);
        let out = d.join("out.udf");
        fs::write(&out, vec![0u8; 4 << 20]).unwrap();
        let ok = Command::new("mkudffs").args(["--blocksize=2048", "--label=MKUDF"]).arg(&out).stdout(std::process::Stdio::null()).status().is_ok_and(|s| s.success());
        if !ok {
            return; // mkudffs not installed
        }
        let iso = Iso::open(&out).unwrap();
        assert_eq!((iso.volume_id().as_str(), iso.entries().unwrap().len()), ("MKUDF", 0));
        let _ = fs::remove_dir_all(&d);
    }

    fn set_tag(b: &mut [u8], id: u16) {
        b[..2].copy_from_slice(&id.to_le_bytes());
        b[4] = (0..16).filter(|&i| i != 4).fold(0u8, |s, i| s.wrapping_add(b[i]));
    }

    fn put(b: &mut [u8], o: usize, v: &[u8]) {
        b[o..o + v.len()].copy_from_slice(v);
    }

    fn fid(ch: u8, name: &[u8], lbn: u32) -> Vec<u8> {
        let mut f = vec![0u8; 38];
        (f[18], f[19]) = (ch, name.len() as u8);
        put(&mut f, 20, &2048u32.to_le_bytes());
        put(&mut f, 24, &lbn.to_le_bytes());
        f.extend(name);
        f.resize(f.len().div_ceil(4) * 4, 0);
        set_tag(&mut f, 257);
        f
    }

    fn fe(id: u16, kind: u8, flags: u16, size: u64, ad: &[u8]) -> Vec<u8> {
        let mut b = vec![0u8; 2048];
        let hdr = if id == 261 { 176 } else { 216 };
        b[27] = kind;
        put(&mut b, 34, &flags.to_le_bytes());
        put(&mut b, 56, &size.to_le_bytes());
        put(&mut b, hdr - 4, &(ad.len() as u32).to_le_bytes());
        put(&mut b, hdr, ad);
        set_tag(&mut b, id);
        b
    }

    /// Tiny UDF-only image: embedded dir/file data, EFE + short_ad, long_ad + unrecorded extent, symlink, 16-bit name, loop.
    fn udf_image() -> Vec<u8> {
        const P: usize = 300;
        let mut img = vec![0u8; (P + 8) * 2048];
        for (s, id) in [(16, b"BEA01"), (17, b"NSR02"), (18, b"TEA01")] {
            put(&mut img, s * 2048 + 1, id);
        }
        let sec = |img: &mut Vec<u8>, s: usize, b: &[u8]| put(img, s * 2048, b);
        let mut b = vec![0u8; 2048];
        put(&mut b, 16, &(4 * 2048u32).to_le_bytes());
        put(&mut b, 20, &32u32.to_le_bytes());
        set_tag(&mut b, 2);
        sec(&mut img, 256, &b);
        let mut b = vec![0u8; 2048];
        put(&mut b, 24, b"\x08PVDNAME");
        b[55] = 8;
        set_tag(&mut b, 1);
        sec(&mut img, 32, &b);
        let mut b = vec![0u8; 2048];
        put(&mut b, 188, &(P as u32).to_le_bytes());
        set_tag(&mut b, 5);
        sec(&mut img, 33, &b);
        let mut b = vec![0u8; 2048];
        put(&mut b, 84, b"\x08UDF_ONLY");
        b[211] = 9;
        put(&mut b, 212, &2048u32.to_le_bytes());
        put(&mut b, 248, &2048u32.to_le_bytes());
        put(&mut b, 264, &6u32.to_le_bytes());
        put(&mut b, 268, &1u32.to_le_bytes());
        put(&mut b, 440, &[1, 6, 1, 0, 0, 0]);
        set_tag(&mut b, 6);
        sec(&mut img, 34, &b);
        let mut b = vec![0u8; 2048];
        set_tag(&mut b, 8);
        sec(&mut img, 35, &b);
        let mut b = vec![0u8; 2048];
        put(&mut b, 404, &1u32.to_le_bytes());
        set_tag(&mut b, 256);
        sec(&mut img, P, &b);
        let dir16: Vec<u8> = std::iter::once(16).chain("Dïr✓".encode_utf16().flat_map(u16::to_be_bytes)).collect();
        let root = [fid(10, b"", 1), fid(2, &dir16, 2), fid(0, b"\x08a.txt", 3), fid(4, b"\x08gone", 3)].concat();
        sec(&mut img, P + 1, &fe(261, 4, 3, root.len() as u64, &root));
        let sub = [fid(10, b"", 1), fid(0, b"\x08big.bin", 5), fid(0, b"\x08ln", 7), fid(2, b"\x08loop", 1)].concat();
        let short = [(sub.len() as u32).to_le_bytes(), 4u32.to_le_bytes()].concat();
        sec(&mut img, P + 2, &fe(266, 4, 0, sub.len() as u64, &short));
        sec(&mut img, P + 3, &fe(261, 5, 3, 5, b"hello"));
        sec(&mut img, P + 4, &sub);
        let mut long = vec![0u8; 32];
        put(&mut long, 0, &2048u32.to_le_bytes());
        put(&mut long, 4, &6u32.to_le_bytes());
        put(&mut long, 16, &(100u32 | 1 << 30).to_le_bytes());
        sec(&mut img, P + 5, &fe(261, 5, 1, 2148, &long));
        sec(&mut img, P + 6, &[0xcc; 2048]);
        let link = [&[2u8, 0, 0, 0][..], &[5, 6, 0, 0], b"\x08a.txt"].concat();
        sec(&mut img, P + 7, &fe(261, 12, 3, link.len() as u64, &link));
        img
    }

    fn exercise(p: &Path) {
        if let Ok(iso) = Iso::open(p) {
            for e in iso.entries().unwrap_or_default() {
                if let Ok(r) = iso.reader(&e) {
                    let _ = io::copy(&mut r.take(1 << 20), &mut io::sink());
                }
            }
        }
    }

    #[test]
    fn iso_udf_only_and_malformed() {
        let mut img = udf_image();
        let d = tmp("udfonly");
        let p = d.join("u.img");
        fs::write(&p, &img).unwrap();
        let iso = Iso::open(&p).unwrap();
        assert_eq!(iso.volume_id(), "UDF_ONLY");
        assert!(!iso.el_torito().bios && !iso.el_torito().efi);
        let es = iso.entries().unwrap();
        let got: Vec<_> = es.iter().map(|e| (e.path.as_str(), e.is_dir, e.size, e.symlink())).collect();
        let want = [("Dïr✓", true, 0, None), ("a.txt", false, 5, None), ("Dïr✓/big.bin", false, 2148, None), ("Dïr✓/ln", false, 0, Some("/a.txt")), ("Dïr✓/loop", true, 0, None)];
        assert_eq!(got, want);
        let read = |e: &Entry| {
            let mut v = Vec::new();
            iso.reader(e).unwrap().read_to_end(&mut v).unwrap();
            v
        };
        assert_eq!(read(&es[1]), b"hello");
        let big = read(&es[2]);
        assert!(big.len() == 2148 && big[..2048].iter().all(|&b| b == 0xcc) && big[2048..].iter().all(|&b| b == 0));
        let mut buf = [1u8; 4];
        assert_eq!((iso.read_at(&es[2], 2046, &mut buf).unwrap(), buf), (4, [0xcc, 0xcc, 0, 0]));
        assert_eq!((iso.read_at(&es[1], 3, &mut buf).unwrap(), &buf[..2]), (2, &b"lo"[..]));
        // Malformed: broken FID name length, bad root ICB tag, bad AVDP.
        let fid0 = 300 * 2048 + 2048 + 176;
        img[fid0 + 40 + 19] = 200;
        fs::write(&p, &img).unwrap();
        assert_eq!(Iso::open(&p).unwrap().entries().unwrap_err().kind(), io::ErrorKind::InvalidData);
        let mut img = udf_image();
        img[301 * 2048] = 7;
        fs::write(&p, &img).unwrap();
        assert_eq!(Iso::open(&p).unwrap().entries().unwrap_err().kind(), io::ErrorKind::InvalidData);
        let mut img = udf_image();
        img[256 * 2048 + 4] ^= 1;
        fs::write(&p, &img).unwrap();
        assert_eq!(Iso::open(&p).err().unwrap().kind(), io::ErrorKind::InvalidData);
        // Truncations and single-byte corruptions must not panic or hang.
        let img = udf_image();
        for len in [0, 100 * 2048, 257 * 2048, 301 * 2048 + 500, 305 * 2048] {
            fs::write(&p, &img[..len]).unwrap();
            exercise(&p);
        }
        fs::write(&p, &img).unwrap();
        let mut f = fs::OpenOptions::new().write(true).open(&p).unwrap();
        for s in [256, 32, 33, 34, 300, 301, 302, 303, 304, 305, 307] {
            for o in (0..400).chain(2040..2048) {
                let at = (s * 2048 + o) as u64;
                for v in [img[at as usize] ^ 0xff, 0x80] {
                    use std::io::{Seek, SeekFrom, Write};
                    f.seek(SeekFrom::Start(at)).unwrap();
                    f.write_all(&[v]).unwrap();
                    exercise(&p);
                    f.seek(SeekFrom::Start(at)).unwrap();
                    f.write_all(&[img[at as usize]]).unwrap();
                }
            }
        }
        let _ = fs::remove_dir_all(&d);
    }

    /// RUSTFUS_ISO=/path/to.iso cargo test iso_real -- --ignored --nocapture
    #[test]
    #[ignore]
    fn iso_real() {
        let iso = Iso::open(Path::new(&std::env::var("RUSTFUS_ISO").unwrap())).unwrap();
        let es = iso.entries().unwrap();
        let total: u64 = es.iter().filter(|e| !e.is_dir).map(|e| io::copy(&mut iso.reader(e).unwrap(), &mut io::sink()).unwrap()).sum();
        assert_eq!(total, es.iter().map(|e| e.size).sum::<u64>());
        println!("{} {:?} {} entries {total} bytes {:?}", iso.volume_id(), iso.el_torito(), es.len(), &es[..es.len().min(5)]);
    }
}
