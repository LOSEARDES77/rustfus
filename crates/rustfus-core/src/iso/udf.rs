// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! UDF (ECMA-167 / OSTA UDF 1.02-2.60), read-only. 2048-byte blocks only.

use super::{BLK, ElTorito, Entry, ExtReader, Iso, Names, bad, boot_catalog, le32};
use std::collections::HashSet;
use std::fs::File;
use std::io::{self, Read};

/// Extent offset meaning "reads as zeros" (allocated-but-unrecorded / sparse extents).
pub(super) const ZERO: u64 = u64::MAX;

pub(super) fn le16(b: &[u8], o: usize) -> u16 {
    b.get(o..o + 2)
        .map_or(0, |s| u16::from_le_bytes([s[0], s[1]]))
}

pub(super) fn le64(b: &[u8], o: usize) -> u64 {
    b.get(o..o + 8).map_or(0, |s| {
        u64::from_le_bytes(s.try_into().expect("8-byte slice"))
    })
}

/// Checks a descriptor tag's identifier and header checksum.
pub(super) fn tag(b: &[u8], id: u16) -> io::Result<()> {
    let ok = b.len() >= 16
        && le16(b, 0) == id
        && (0..16)
            .filter(|&i| i != 4)
            .fold(0u8, |s, i| s.wrapping_add(b[i]))
            == b[4];
    if !ok {
        return Err(bad("bad UDF descriptor tag"));
    }
    Ok(())
}

/// OSTA CS0 compressed Unicode (8- or 16-bit).
pub(super) fn cs0(b: &[u8]) -> String {
    match b.split_first() {
        Some((8 | 254, r)) => r.iter().map(|&c| c as char).collect(),
        Some((16 | 255, r)) => String::from_utf16_lossy(
            &r.as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_be_bytes(*c))
                .collect::<Vec<_>>(),
        ),
        _ => String::new(),
    }
}

pub(super) fn dstring(b: &[u8]) -> String {
    let n = b.last().map_or(0, |&n| n as usize);
    if n < b.len() {
        cs0(&b[..n])
    } else {
        String::new()
    }
}

pub(super) enum Part {
    /// Byte offset of a physical (or sparable) partition.
    Phys(u64),
    /// UDF 2.50+ metadata partition: physical extents of the metadata file.
    Meta(Vec<(u64, u64)>),
}

pub(super) struct Udf {
    pub(super) parts: Vec<Part>,
    pub(super) root: (u16, u32),
}

pub(super) struct Node {
    pub(super) at: u64,
    pub(super) kind: u8,
    pub(super) size: u64,
    pub(super) ext: Vec<(u64, u64)>,
}

impl Udf {
    /// Maps `len` bytes at logical block `lbn` of partition reference `part` to physical extents.
    pub(super) fn map(&self, part: u16, lbn: u32, len: u64) -> io::Result<Vec<(u64, u64)>> {
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

    pub(super) fn block(&self, f: &File, part: u16, lbn: u32) -> io::Result<(u64, [u8; 2048])> {
        let off = self.map(part, lbn, BLK)?[0].0;
        let mut b = [0u8; 2048];
        crate::blockdev::read_at(f, &mut b, off)?;
        Ok((off, b))
    }

    /// Reads a File Entry / Extended File Entry and resolves its data to physical extents.
    pub(super) fn icb(&self, f: &File, (part, lbn): (u16, u32)) -> io::Result<Node> {
        let (at, b) = self.block(f, part, lbn)?;
        let id = le16(&b, 0);
        let hdr = match id {
            261 => 176,
            266 => 216,
            _ => return Err(bad("bad UDF file entry")),
        };
        tag(&b, id)?;
        let (l_ea, l_ad, size) = (
            le32(&b, hdr - 8) as usize,
            le32(&b, hdr - 4) as usize,
            le64(&b, 56),
        );
        let start = hdr
            .checked_add(l_ea)
            .ok_or_else(|| bad("bad UDF file entry"))?;
        let ads = b
            .get(start..start.saturating_add(l_ad))
            .ok_or_else(|| bad("bad UDF file entry"))?;
        let ext = match le16(&b, 34) & 7 {
            3 if size <= l_ad as u64 => vec![(at + start as u64, size)],
            3 => return Err(bad("bad UDF embedded data")),
            t => self.ads(f, ads, t, part, size)?,
        };
        Ok(Node {
            at,
            kind: b[27],
            size,
            ext,
        })
    }

    /// Short (0), long (1) or extended (2) allocation descriptors, following continuation extents.
    pub(super) fn ads(
        &self,
        f: &File,
        ads: &[u8],
        t: u16,
        part: u16,
        size: u64,
    ) -> io::Result<Vec<(u64, u64)>> {
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
                    buf = b
                        .get(24..24usize.saturating_add(l))
                        .ok_or_else(|| bad("bad UDF allocation extent"))?
                        .to_vec();
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

pub(super) fn udf_link(d: &[u8]) -> String {
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
    pub(super) fn open_udf(f: File) -> io::Result<Iso> {
        let mut b = [0u8; 2048];
        let mut boot = ElTorito::default();
        for s in 16..64 {
            if crate::blockdev::read_at(&f, &mut b, s * BLK).is_err() {
                break;
            }
            if &b[1..6] == b"CD001" {
                match b[0] {
                    0 if &b[7..30] == b"EL TORITO SPECIFICATION" => {
                        boot = boot_catalog(&f, le32(&b, 71))
                    }
                    255 => break,
                    _ => {}
                }
            }
        }
        crate::blockdev::read_at(&f, &mut b, 256 * BLK)?;
        tag(&b, 2).map_err(|_| bad("not an ISO9660 or UDF image"))?;
        let (vds_len, vds_loc) = (le32(&b, 16) as u64, le32(&b, 20) as u64);
        let (mut pds, mut lvd, mut vol) = (Vec::<(u16, u64)>::new(), None, String::new());
        for s in vds_loc..vds_loc + (vds_len / BLK).min(256) {
            crate::blockdev::read_at(&f, &mut b, s * BLK)?;
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
        let maps = lvd
            .get(440..440 + le32(&lvd, 264) as usize)
            .ok_or_else(|| bad("bad UDF partition maps"))?;
        let (mut udf, mut meta, mut p) = (
            Udf {
                parts: vec![],
                root: (0, 0),
            },
            vec![],
            0,
        );
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
            let start = pds
                .iter()
                .find(|d| d.0 == num)
                .ok_or_else(|| bad("missing UDF partition descriptor"))?
                .1;
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
        Ok(Iso {
            f,
            vol,
            root: (0, 0),
            names: Names::Plain,
            boot,
            udf: Some(udf),
        })
    }

    pub(super) fn read_node(&self, n: &Node, cap: u64) -> io::Result<Vec<u8>> {
        if n.size > cap {
            return Err(bad("UDF directory too large"));
        }
        let mut v = vec![0; n.size as usize];
        ExtReader {
            f: &self.f,
            ext: n.ext.clone(),
            i: 0,
            pos: 0,
        }
        .read_exact(&mut v)?;
        Ok(v)
    }

    pub(super) fn udf_walk(
        &self,
        u: &Udf,
        dir: Node,
        prefix: &str,
        out: &mut Vec<Entry>,
        seen: &mut HashSet<u64>,
        depth: u32,
    ) -> io::Result<()> {
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
            let name = cs0(d
                .get(38 + l_iu..38 + l_iu + l_fi)
                .ok_or_else(|| bad("bad UDF file identifier"))?);
            pos += (38 + l_iu + l_fi + 3) & !3;
            if ch & 0x0c != 0
                || name.is_empty()
                || name == "."
                || name == ".."
                || name.contains(['/', '\\', '\0'])
            {
                continue; // deleted, parent, or unusable name
            }
            let n = u.icb(&self.f, (le16(d, 28), le32(d, 24)))?;
            let path = format!("{prefix}{name}");
            let mut e = Entry {
                path,
                is_dir: ch & 2 != 0,
                size: n.size,
                extents: vec![],
                link: None,
            };
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
