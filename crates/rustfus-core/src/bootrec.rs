// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Ported from ms-sys (via Rufus): Copyright © 2009-2015 Henrik Carlqvist,
// Copyright © 2012 Pete Batard (GPL-2.0-or-later).
//! Boot record writers ported from Rufus' ms-sys (src/ms-sys/{br,fat32,ntfs,partition_info}.c).
//! Offsets are relative to the stream start (disk for MBRs, partition for PBRs).
use crate::bytes::put;
use std::io::{self, Read, Seek, SeekFrom, Write};

const MBR_WIN7: &[u8] = include_bytes!("../../../res/ms-sys/mbr_win7.bin");
const MBR_RUFUS: &[u8] = include_bytes!("../../../res/ms-sys/mbr_rufus.bin");
const MBR_SYSLINUX: &[u8] = include_bytes!("../../../res/ms-sys/mbr_syslinux.bin");
// ms-sys mbr_grub2.h is byte-identical to boot.img[..0x1b0] (jump @0x66 already NOPed).
const GRUB2_BOOT_IMG: &[u8] = include_bytes!("../../../res/grub2/boot.img");
const FAT32_0X0: &[u8] = include_bytes!("../../../res/ms-sys/br_fat32_0x0.bin");
const FAT32PE_0X52: &[u8] = include_bytes!("../../../res/ms-sys/br_fat32pe_0x52.bin");
const FAT32PE_0X3F0: &[u8] = include_bytes!("../../../res/ms-sys/br_fat32pe_0x3f0.bin");
const FAT32PE_0X1800: &[u8] = include_bytes!("../../../res/ms-sys/br_fat32pe_0x1800.bin");
const FAT32FD_0X52: &[u8] = include_bytes!("../../../res/ms-sys/br_fat32fd_0x52.bin");
const FAT32FD_0X3F0: &[u8] = include_bytes!("../../../res/ms-sys/br_fat32fd_0x3f0.bin");
const NTFS_0X0: &[u8] = include_bytes!("../../../res/ms-sys/br_ntfs_0x0.bin");
const NTFS_0X54: &[u8] = include_bytes!("../../../res/ms-sys/br_ntfs_0x54.bin");

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mbr {
    Win7,
    Rufus,
    Syslinux,
    Grub2,
    /// GPT protective MBR that prints the "UEFI-only media" message stored at LBA 34.
    RufusMsg,
    Zero,
}

/// Text shown by `Mbr::RufusMsg`, written by the caller at LBA 34 (after the primary GPT).
pub static MSG_TEXT: &[u8] = include_bytes!("../../../res/ms-sys/mbr_msg.txt");
const MBR_MSG_RUFUS: &[u8] = include_bytes!("../../../res/ms-sys/mbr_msg_rufus.bin");

/// Reads `len` bytes at `off` (both sector multiples), lets `edit` patch them, writes them back.
/// Nothing is written if `edit` fails.
fn rmw<F: Read + Write + Seek>(
    f: &mut F,
    off: u64,
    len: usize,
    edit: impl FnOnce(&mut [u8]) -> io::Result<()>,
) -> io::Result<()> {
    let mut buf = vec![0; len];
    f.seek(SeekFrom::Start(off))?;
    f.read_exact(&mut buf)?;
    edit(&mut buf)?;
    f.seek(SeekFrom::Start(off))?;
    f.write_all(&buf)?;
    f.flush()
}

const FSINFO_FREE: usize = 0x3e8;

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

/// Writes MBR boot code (bytes 0..440), preserving disk signature, partition table and 0x55AA.
pub fn write_mbr<F: Read + Write + Seek>(disk: &mut F, kind: Mbr) -> io::Result<()> {
    let code = match kind {
        Mbr::Win7 => MBR_WIN7,
        Mbr::Rufus => MBR_RUFUS,
        Mbr::Syslinux => MBR_SYSLINUX,
        Mbr::RufusMsg => MBR_MSG_RUFUS,
        // TODO: Rufus also writes core.img at LBA 1 (WriteSBR); boot.img's kernel_sector (0x5C) is 1.
        Mbr::Grub2 => &GRUB2_BOOT_IMG[..0x1b0],
        // ms-sys zeroes 0..0x1BE (incl. disk signature); we keep the signature.
        Mbr::Zero => &[0; 440],
    };
    rmw(disk, 0, 512, |b| {
        put(b, 0, code);
        put(b, 0x1fe, &[0x55, 0xaa]); // ms-sys write_bootmark
        Ok(())
    })
}

/// ms-sys write_fat_32_*_br(fp, bKeepLabel=1) + write_partition_physical_disk_drive_id_fat32,
/// applied to the primary and the backup (sector 6) boot sectors like Rufus' WritePBR.
fn write_fat32<F: Read + Write + Seek>(
    part: &mut F,
    code_0x52: &[u8],
    code_0x3f0: &[u8],
    code_0x1800: Option<&[u8]>,
) -> io::Result<()> {
    let mut bps = [0; 2];
    part.seek(SeekFrom::Start(0x0b))?;
    part.read_exact(&mut bps)?;
    let backup = 6 * u16::from_le_bytes(bps) as u64;
    for base in [0, backup] {
        rmw(part, base, 0x600, |b| {
            if &b[0x52..0x5a] != b"FAT32   " {
                return Err(bad("not a FAT32 boot sector"));
            }
            put(b, 0, FAT32_0X0);
            b[0x40] = 0x80; // drive number: C:
            // ms-sys' fat32pe array runs on to 0x3ED, zeroing the FSInfo free cluster count
            // (0x3E8); stop before it like the fat32fd array does, so fsck stays happy.
            put(
                b,
                0x52,
                &code_0x52[..code_0x52.len().min(FSINFO_FREE - 0x52)],
            );
            put(b, 0x3f0, code_0x3f0);
            Ok(())
        })?;
        if let Some(code) = code_0x1800 {
            rmw(part, base + 0x1800, 0x200, |b| {
                put(b, 0, code);
                Ok(())
            })?;
        }
    }
    Ok(())
}

/// FAT32 PBR that loads BOOTMGR (Windows PE / install media): ms-sys "fat32pe", preserving the BPB.
pub fn write_fat32_pe_pbr<F: Read + Write + Seek>(part: &mut F) -> io::Result<()> {
    write_fat32(part, FAT32PE_0X52, FAT32PE_0X3F0, Some(FAT32PE_0X1800))
}

/// FAT32 PBR for FreeDOS (KERNEL.SYS): ms-sys "fat32fd".
pub fn write_fat32_freedos_pbr<F: Read + Write + Seek>(part: &mut F) -> io::Result<()> {
    write_fat32(part, FAT32FD_0X52, FAT32FD_0X3F0, None)
}

/// NTFS PBR that loads BOOTMGR: ms-sys write_ntfs_br, preserving the BPB.
/// Like ms-sys, the backup boot sector at the end of the volume is left alone.
pub fn write_ntfs_pbr<F: Read + Write + Seek>(part: &mut F) -> io::Result<()> {
    rmw(
        part,
        0,
        (0x54 + NTFS_0X54.len()).next_multiple_of(512),
        |b| {
            if &b[3..11] != b"NTFS    " {
                return Err(bad("not an NTFS boot sector"));
            }
            put(b, 0, NTFS_0X0);
            put(b, 0x54, NTFS_0X54);
            Ok(())
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::path::Path;
    use std::process::Command;

    fn noise(n: usize, mut s: u32) -> Vec<u8> {
        (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                s as u8
            })
            .collect()
    }

    #[test]
    fn mbr_preserves_table() {
        let orig = noise(1024, 0x1234_5678);
        let kinds = [
            (Mbr::Win7, MBR_WIN7),
            (Mbr::Rufus, MBR_RUFUS),
            (Mbr::Syslinux, MBR_SYSLINUX),
            (Mbr::Grub2, &GRUB2_BOOT_IMG[..0x1b0]),
            (Mbr::Zero, &[0; 440]),
        ];
        for (kind, code) in kinds {
            let mut c = Cursor::new(orig.clone());
            write_mbr(&mut c, kind).unwrap();
            let d = c.into_inner();
            assert_eq!(&d[..code.len()], code, "{kind:?}");
            assert_eq!(d[code.len()..0x1fe], orig[code.len()..0x1fe], "{kind:?}");
            assert_eq!(d[0x1fe..0x200], [0x55, 0xaa]);
            assert_eq!(d[0x200..], orig[0x200..]);
        }
    }

    #[test]
    fn fat32_pbr_on_mkfs_image() {
        let (mkfs, fsck) = ("/usr/bin/mkfs.vfat", "/usr/bin/fsck.vfat");
        if !Path::new(mkfs).exists() || !Path::new(fsck).exists() {
            eprintln!("skipping: dosfstools missing");
            return;
        }
        let img = std::env::temp_dir().join(format!("rustfus-bootrec-{}.img", std::process::id()));
        type Writer = fn(&mut Cursor<Vec<u8>>) -> io::Result<()>;
        let cases = [
            (
                write_fat32_pe_pbr as Writer,
                FAT32PE_0X52,
                FAT32PE_0X3F0,
                Some(FAT32PE_0X1800),
            ),
            (write_fat32_freedos_pbr, FAT32FD_0X52, FAT32FD_0X3F0, None),
        ];
        for (write, c52, c3f0, c1800) in cases {
            let _ = std::fs::remove_file(&img);
            let ok = Command::new(mkfs)
                .args(["-F", "32", "-C"])
                .arg(&img)
                .arg("65536")
                .output()
                .unwrap();
            assert!(ok.status.success());
            let orig = std::fs::read(&img).unwrap();
            let mut c = Cursor::new(orig.clone());
            write(&mut c).unwrap();
            let d = c.into_inner();
            for base in [0, 6 * 512] {
                let (o, d) = (&orig[base..], &d[base..]);
                assert_eq!(&d[..11], FAT32_0X0);
                assert_eq!(d[0x0b..0x40], o[0x0b..0x40]);
                assert_eq!(d[0x40], 0x80);
                assert_eq!(d[0x41..0x5a], o[0x41..0x5a]);
                assert_eq!(&d[0x52..FSINFO_FREE], &c52[..FSINFO_FREE - 0x52]);
                assert_eq!(d[FSINFO_FREE..0x3f0], o[FSINFO_FREE..0x3f0]);
                assert_eq!(&d[0x3f0..0x600], c3f0);
                assert_eq!(d[0x1fe..0x200], [0x55, 0xaa]);
                match c1800 {
                    Some(c) => assert_eq!(&d[0x1800..0x1a00], c),
                    None => assert_eq!(d[0x1800..0x1a00], o[0x1800..0x1a00]),
                }
            }
            std::fs::write(&img, &d).unwrap();
            let out = Command::new(fsck).arg("-n").arg(&img).output().unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stdout)
            );
        }
        let _ = std::fs::remove_file(&img);
        let mut c = Cursor::new(vec![0u8; 0x4000]);
        assert!(write_fat32_pe_pbr(&mut c).is_err());
        assert!(c.into_inner().iter().all(|&b| b == 0));
    }

    #[test]
    fn ntfs_pbr() {
        let mut orig = noise(0x2000, 0xdead_beef);
        orig[3..11].copy_from_slice(b"NTFS    ");
        let mut c = Cursor::new(orig.clone());
        write_ntfs_pbr(&mut c).unwrap();
        let d = c.into_inner();
        let end = 0x54 + NTFS_0X54.len();
        assert_eq!(&d[..11], NTFS_0X0);
        assert_eq!(d[0x0b..0x54], orig[0x0b..0x54]);
        assert_eq!(&d[0x54..end], NTFS_0X54);
        assert_eq!(d[0x1fe..0x200], [0x55, 0xaa]);
        assert_eq!(d[end..], orig[end..]);

        orig[3] = b'X';
        let mut c = Cursor::new(orig.clone());
        assert!(write_ntfs_pbr(&mut c).is_err());
        assert_eq!(c.into_inner(), orig);
    }
}
