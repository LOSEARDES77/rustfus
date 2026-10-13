# Third-party notices

Rustfus is licensed under the **GNU General Public License v3.0 or later**
(see [LICENSE](LICENSE)). It contains, embeds or is derived from the
third-party work listed below. Every embedded binary in `res/` is compiled
into the Rustfus executable with `include_bytes!`.

## Source code Rustfus is derived from

| Work                                                                      | Copyright                                                                                                         | License          | Used in                                                       |
| ------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------- | ---------------- | ------------------------------------------------------------- |
| [Rufus](https://github.com/pbatard/rufus)                                 | © 2011-2026 Pete Batard                                                                                           | GPL-3.0-or-later | Overall design, UI, behaviour; ported logic throughout `src/` |
| [ms-sys](https://ms-sys.sourceforge.net/) (as shipped in Rufus)           | © 2009-2015 Henrik Carlqvist, © 2012 Pete Batard                                                                  | GPL-2.0-or-later | `src/bootrec.rs`                                              |
| [Syslinux](https://wiki.syslinux.org/) libinstaller (as shipped in Rufus) | © 1998-2008 H. Peter Anvin, © 2009-2014 Intel Corporation, © 2003 Lars Munch Christensen, © 2012-2024 Pete Batard | GPL-2.0-or-later | `src/syslinux.rs`                                             |

The ISO9660/UDF reader (`src/iso.rs`) and ext2/3/4 formatter (`src/ext.rs`)
were written from the format specifications, using Rufus' bundled libcdio
(GPL-3.0-or-later) and ext2fs (GPL-2.0/LGPL) only as behavioural references.

## Embedded resources (`res/`)

| Path                                                                               | Origin                                                                                                                                                                                                                                     | Copyright                                                  | License                                                                                                                                                                                                                   |
| ---------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `res/ms-sys/mbr_rufus.bin`, `mbr_msg_rufus.bin`, `mbr_msg.txt`                     | Rufus MBR and message MBR ([res/mbr](https://github.com/pbatard/rufus/tree/master/res/mbr))                                                                                                                                                | © 2012-2022 Pete Batard                                    | GPL-3.0-or-later                                                                                                                                                                                                          |
| `res/ms-sys/mbr_syslinux.bin`, `mbr_gpt_syslinux.bin`                              | Syslinux 6.02 `mbr.bin` / `gptmbr.bin`, via ms-sys                                                                                                                                                                                         | © H. Peter Anvin et al.                                    | GPL-2.0-or-later                                                                                                                                                                                                          |
| `res/ms-sys/br_fat32fd_*.bin`                                                      | ms-sys FAT32 boot record modified for FreeDOS                                                                                                                                                                                              | © Henrik Carlqvist, Pete Batard                            | GPL-2.0-or-later                                                                                                                                                                                                          |
| `res/ms-sys/mbr_win7.bin`, `br_fat32_0x0.bin`, `br_fat32pe_*.bin`, `br_ntfs_*.bin` | Microsoft Windows boot code, as distributed by ms-sys and Rufus                                                                                                                                                                            | © Microsoft Corporation                                    | **Not free software** — see note below                                                                                                                                                                                    |
| `res/syslinux/`                                                                    | Syslinux 4.07 and 6.04-pre1 `ldlinux.sys`/`.bss`, 4.07 `mboot.c32` ([kernel.org](https://www.kernel.org/pub/linux/utils/boot/syslinux/))                                                                                                   | © H. Peter Anvin, Intel Corporation et al.                 | GPL-2.0-or-later                                                                                                                                                                                                          |
| `res/grub2/`                                                                       | GRUB 2.14 `boot.img` / `core.img` built by Rufus ([build notes](res/grub2/readme.txt), [source](https://ftp.gnu.org/gnu/grub/grub-2.14.tar.xz))                                                                                            | © Free Software Foundation                                 | GPL-3.0-or-later                                                                                                                                                                                                          |
| `res/freedos/`                                                                     | FreeDOS 1.4 kernel, FreeCOM, DISPLAY, KEYB, MODE, CPIDOS, keyboard layouts ([details](res/freedos/readme.txt), [source](https://www.ibiblio.org/pub/micro/pc-stuff/freedos/files/distributions/1.4/))                                      | © The FreeDOS Project and contributors                     | GPL-2.0-or-later (KEYB layouts and CPI files: GPL; see each package)                                                                                                                                                      |
| `res/uefi/uefi-ntfs.img`                                                           | [UEFI:NTFS](https://github.com/pbatard/uefi-ntfs) 2.8 bootloaders, NTFS drivers from [ntfs-3g](https://github.com/pbatard/ntfs-3g) 1.9, exFAT drivers from [EfiFs](https://github.com/pbatard/efifs) 1.12 ([details](res/uefi/readme.txt)) | © Pete Batard, Tuxera/ntfs-3g authors, EfiFs authors       | GPL-2.0 (UEFI:NTFS), GPL-2.0-or-later (ntfs-3g), GPL-3.0-or-later (EfiFs)                                                                                                                                                 |
| `res/md5/`                                                                         | [uefi-md5sum](https://github.com/pbatard/uefi-md5sum) bootloaders                                                                                                                                                                          | © Pete Batard                                              | GPL-2.0                                                                                                                                                                                                                   |
| `res/loc/rufus.loc`                                                                | Rufus translations (38 languages)                                                                                                                                                                                                          | © Pete Batard and the Rufus translators listed in the file | GPL-3.0-or-later                                                                                                                                                                                                          |
| `res/rufus_pubkey.bin`                                                             | Rufus' RSA public key, used only to verify files downloaded from rufus.ie                                                                                                                                                                  | Pete Batard                                                | Public key (not copyrightable data)                                                                                                                                                                                       |
| `res/rustfus.png`, `res/rustfus.ico`                                               | Rustfus icon                                                                                                                                                                                                                               | Rustfus contributors                                       | GPL-3.0-or-later. The gear-and-R emblem resembles the Rust logo, a trademark of the Rust Foundation (logo licensed CC-BY 4.0); see the [Rust trademark policy](https://rustfoundation.org/policy/rust-trademark-policy/). |

### Why GPL-2.0 binaries can ship inside a GPL-3.0 program

The bootloaders, DOS programs and UEFI drivers in `res/` are separate programs
run by the target PC's firmware or by DOS; Rustfus only stores them as data and
writes them to the drive. They are not linked with Rustfus, so they form an
aggregate ("mere aggregation", GPL-2.0 §2; GPL-3.0 §5) and each keeps its own
license. Rufus distributes the same files the same way.

### Microsoft boot code

The Windows 7 MBR and the Windows FAT32/NTFS boot sector code are Microsoft's
copyrighted code. They are distributed the same way by ms-sys and Rufus, and
are needed to boot Windows installation media in BIOS/CSM mode. They are not
covered by the GPL. If you need a distribution that contains only free
software, remove those files and the `Mbr::Win7` / `write_fat32_pe_pbr` /
`write_ntfs_pbr` paths in `src/bootrec.rs`; UEFI boot of Windows media does not
need them.

### Corresponding source for embedded binaries

GPL binaries in `res/` come with their versions and upstream source locations
above (and in each `res/*/readme.txt`). Anyone distributing Rustfus
executables must also make the corresponding source of those binaries
available (GPL-2.0 §3 / GPL-3.0 §6). The release workflow does this
automatically: it downloads every archive listed in
[res/SOURCES.txt](res/SOURCES.txt) and attaches them to each GitHub release
as `rustfus-<version>-third-party-sources.tar`. If you change a file in
`res/`, update `res/SOURCES.txt` to match.

## Downloaded at runtime (not distributed)

| What                                                             | From                                                                                                                                         | License                                              |
| ---------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------- |
| GRUB `core.img` and Syslinux loaders matching an image's version | `https://rufus.ie/files/` (RSA-signature checked)                                                                                            | GPL-3.0-or-later (GRUB), GPL-2.0-or-later (Syslinux) |
| [Fido](https://github.com/pbatard/Fido) PowerShell script        | GitHub releases listed in `https://rufus.ie/Fido.ver` (signature checked; only its version table is read, its download logic is reimplemented in `src/fido.rs`) | GPL-3.0-or-later                                     |
| Windows ISO images                                               | Microsoft, via Fido                                                                                                                          | Microsoft license terms                              |

## Rust crates

All crates Rustfus links are under licenses compatible with GPL-3.0:
MIT, Apache-2.0, BSD-2/3-Clause, Zlib, ISC, BSL-1.0, Unlicense, 0BSD,
Unicode-3.0, bzip2-1.0.6, and dual-licensed crates for which a permissive
option is chosen (`r-efi`: MIT/Apache-2.0; `self_cell`: Apache-2.0).
egui's bundled fonts are under the SIL Open Font License 1.1 and the Ubuntu
Font Licence 1.0. Run `cargo tree --format "{p} {l}"` for the full list.
