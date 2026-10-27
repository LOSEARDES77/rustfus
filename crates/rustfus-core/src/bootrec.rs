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
#[path = "../tests/unit/bootrec.rs"]
mod tests;
