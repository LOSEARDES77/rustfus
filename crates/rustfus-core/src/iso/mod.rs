// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
use std::collections::HashSet;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

#[cfg(test)]
mod tests;
mod udf;

use udf::*;

const BLK: u64 = 2048;

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

fn le32(b: &[u8], o: usize) -> u32 {
    b.get(o..o + 4)
        .map_or(0, |s| u32::from_le_bytes(s.try_into().unwrap()))
}

fn read_at(f: &File, buf: &mut [u8], off: u64) -> io::Result<usize> {
    #[cfg(unix)]
    return std::os::unix::fs::FileExt::read_at(f, buf, off);
    #[cfg(windows)]
    return std::os::windows::fs::FileExt::seek_read(f, buf, off);
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
    let id = r
        .get(33..33 + n)
        .ok_or_else(|| bad("bad directory record"))?;
    let su = r.get(33 + n + (1 - n % 2)..).unwrap_or(&[]);
    Ok(Rec {
        ext: le32(r, 2) as u64 * BLK,
        len: le32(r, 10) as u64,
        flags: r[25],
        id,
        su,
    })
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
    if crate::blockdev::read_at(f, &mut b, lba as u64 * BLK).is_err()
        || b[0] != 1
        || b[30..32] != [0x55, 0xAA]
    {
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
            crate::blockdev::read_at(&f, &mut vd, s * BLK)?;
            if &vd[1..6] != b"CD001" {
                return Err(bad("not an ISO9660 image"));
            }
            match vd[0] {
                0 if &vd[7..30] == b"EL TORITO SPECIFICATION" => {
                    boot = boot_catalog(&f, le32(&vd, 71))
                }
                1 if pvd.is_none() => {
                    pvd = Some((
                        root_of(&vd)?,
                        String::from_utf8_lossy(&vd[40..72]).trim().to_string(),
                    ))
                }
                2 if vd[88..90] == [0x25, 0x2f] && matches!(vd[90], 0x40 | 0x43 | 0x45) => {
                    joliet = Some(root_of(&vd)?)
                }
                255 => break,
                _ => {}
            }
        }
        let (root, vol) = pvd.ok_or_else(|| bad("no primary volume descriptor"))?;
        let mut iso = Iso {
            f,
            vol,
            root,
            names: Names::Plain,
            boot,
            udf: None,
        };
        let mut sec = [0u8; 2048];
        crate::blockdev::read_at(&iso.f, &mut sec, root.0)?;
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
            Some(u) => self.udf_walk(
                u,
                u.icb(&self.f, u.root)?,
                "",
                &mut out,
                &mut HashSet::new(),
                0,
            )?,
            None => self.walk(self.root, "", &mut out, &mut HashSet::new(), 0)?,
        }
        Ok(out)
    }

    pub fn reader<'a>(&'a self, e: &Entry) -> io::Result<impl Read + 'a> {
        if e.is_dir {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "is a directory",
            ));
        }
        Ok(ExtReader {
            f: &self.f,
            ext: e.extents.clone(),
            i: 0,
            pos: 0,
        })
    }

    /// Reads up to `buf.len()` bytes at `offset` within file `e` (fills `buf` unless EOF; 0 at EOF).
    pub fn read_at(&self, e: &Entry, mut offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        if e.is_dir {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "is a directory",
            ));
        }
        let mut r = ExtReader {
            f: &self.f,
            ext: e.extents.clone(),
            i: 0,
            pos: 0,
        };
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
                    b"NM" if l >= 5 && e[4] & 6 == 0 => {
                        name.get_or_insert_default().extend(&e[5..])
                    }
                    b"SL" if l >= 5 => {
                        let link = link.get_or_insert_default();
                        let mut c = 5;
                        while let (Some(&cf), Some(&cl)) = (e.get(c), e.get(c + 1)) {
                            let Some(body) = e.get(c + 2..c + 2 + cl as usize) else {
                                break;
                            };
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
                    b"CE" if l >= 28 => {
                        ce = Some((
                            le32(e, 4) as u64 * BLK + le32(e, 12) as u64,
                            le32(e, 20) as usize,
                        ))
                    }
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
            crate::blockdev::read_at(&self.f, &mut area, off)?;
        }
        s.name = name.map(|n| String::from_utf8_lossy(&n).into_owned());
        s.link = link;
        Ok(s)
    }

    fn walk(
        &self,
        (ext, len): (u64, u64),
        prefix: &str,
        out: &mut Vec<Entry>,
        seen: &mut HashSet<u64>,
        depth: u32,
    ) -> io::Result<()> {
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
        crate::blockdev::read_at(&self.f, &mut buf, ext)?;
        let (mut local, mut last_id, mut more) = (Vec::<Entry>::new(), &[][..], false);
        let mut pos = 0;
        while pos < buf.len() {
            let n = buf[pos] as usize;
            if n == 0 {
                pos = (pos / 2048 + 1) * 2048;
                continue;
            }
            let r = parse_rec(
                buf.get(pos..pos + n)
                    .ok_or_else(|| bad("bad directory record"))?,
            )?;
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
            let s = if self.names == Names::Rr {
                self.susp(r.su)?
            } else {
                Susp::default()
            };
            if s.relocated {
                continue;
            }
            let name = match (s.name, self.names) {
                (Some(n), _) => n,
                (None, Names::Joliet) => {
                    let u: Vec<u16> =
                        r.id.chunks_exact(2)
                            .map(|c| u16::from_be_bytes([c[0], c[1]]))
                            .collect();
                    let n = String::from_utf16_lossy(&u);
                    n.split(';').next().unwrap_or("").to_string()
                }
                _ => {
                    let n = String::from_utf8_lossy(r.id);
                    n.split(';')
                        .next()
                        .unwrap_or("")
                        .trim_end_matches('.')
                        .to_string()
                }
            };
            if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0']) {
                continue;
            }
            let path = format!("{prefix}{name}");
            let mut e = Entry {
                path,
                is_dir: r.flags & 2 != 0,
                size: r.len,
                extents: vec![(r.ext, r.len)],
                link: s.link,
            };
            if let Some(c) = s.child {
                let mut sec = [0u8; 2048];
                crate::blockdev::read_at(&self.f, &mut sec, c)?;
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
