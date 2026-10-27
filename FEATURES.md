# Rufus feature parity

Every Rufus feature, and whether Rustfus has it on each platform.

🟢 implemented &nbsp;&nbsp; 🟡 partial / needs an extra tool &nbsp;&nbsp; 🔴 not available

"Implemented" means the code exists and is covered by automated tests on
disk-image files. Nothing has been tested on a physical USB drive yet, and the
Windows and macOS builds compile but have not been run on those systems.
[COMPATIBILITY.md](COMPATIBILITY.md) has the same data grouped by what
Rustfus itself does.

## Drive properties

| Rufus feature | Linux | Windows | macOS | Notes |
|---|:-:|:-:|:-:|---|
| Device list (removable USB drives) | 🟢 | 🟢 | 🟢 | `lsblk` / `Get-Disk` / `diskutil` |
| Hot-plug refresh | 🟢 | 🟢 | 🟢 | 2 s background scan |
| Save drive to VHD / VHDX / IMG | 🟢 | 🟢 | 🟢 | |
| Save drive to FFU | 🔴 | 🟡 | 🔴 | DISM only exists on Windows |
| Boot selection: Disk or ISO image | 🟢 | 🟢 | 🟢 | |
| Boot selection: FreeDOS | 🟢 | 🟢 | 🟢 | |
| Boot selection: Non bootable | 🟢 | 🟢 | 🟢 | |
| Boot selection: MS-DOS | 🔴 | 🔴 | 🔴 | Rufus takes it from Windows' diskcopy.dll |
| Boot selection: Syslinux / ReactOS / GRUB 2 / Grub4DOS / UEFI:NTFS (advanced) | 🔴 | 🔴 | 🔴 | only used automatically when an ISO needs them |
| Image download (Fido: Windows ISOs, UEFI Shell) | 🟢 | 🟢 | 🟢 | runs Fido.ps1 when `pwsh` is installed, else built-in |
| Image checksums (MD5 / SHA-1 / SHA-256 / SHA-512) | 🟢 | 🟢 | 🟢 | |
| Image option: Standard Windows installation | 🟢 | 🟢 | 🟢 | |
| Image option: Windows To Go | 🔴 | 🟡 | 🔴 | DISM + bcdboot; untested |
| Partition scheme: MBR / GPT | 🟢 | 🟢 | 🟢 | own writer |
| Target system: BIOS / UEFI / BIOS + UEFI | 🟢 | 🟢 | 🟢 | |
| Advanced: List USB Hard Drives | 🟢 | 🟢 | 🟢 | system disks always hidden |
| Advanced: Add fixes for old BIOSes | 🟢 | 🟢 | 🟢 | 63-sector alignment |
| Advanced: Use Rufus MBR with BIOS ID | 🔴 | 🔴 | 🔴 | Rufus MBR is written, but no option to pick it or the BIOS ID |
| Advanced: Enable runtime UEFI media validation | 🟢 | 🟢 | 🟢 | uefi-md5sum |

## Format options

| Rufus feature | Linux | Windows | macOS | Notes |
|---|:-:|:-:|:-:|---|
| Volume label | 🟢 | 🟢 | 🟢 | |
| Cluster size | 🟢 | 🟢 | 🟢 | |
| File system: FAT / FAT32 (any size, "Large FAT32") | 🟢 | 🟢 | 🟢 | own formatter, no 32 GB limit |
| File system: NTFS | 🟡 | 🟢 | 🔴 | Linux needs `mkfs.ntfs` (ntfs-3g); macOS can't write NTFS |
| File system: exFAT | 🟡 | 🟢 | 🟢 | Linux needs `mkfs.exfat` (exfatprogs) |
| File system: UDF | 🟡 | 🟢 | 🟢 | Linux needs `mkudffs` (udftools) |
| File system: ReFS | 🔴 | 🟡 | 🔴 | Windows editions with ReFS support only |
| File system: ext2 / ext3 / ext4 (empty) | 🟢 | 🟢 | 🟢 | own formatter |
| File system: ext2 / ext3 / ext4 with ISO files copied | 🟢 | 🔴 | 🔴 | needs the OS to mount ext |
| Advanced: Quick format | 🟢 | 🟢 | 🟢 | |
| Advanced: Create extended label and icon files | 🟢 | 🟢 | 🟢 | `autorun.inf` |
| Advanced: Check device for bad blocks (1 / 2 / 4 passes, SLC/MLC/TLC, random) | 🟢 | 🟢 | 🟢 | |

## Images

| Rufus feature | Linux | Windows | macOS | Notes |
|---|:-:|:-:|:-:|---|
| ISO9660 / Joliet / Rock Ridge extraction | 🟢 | 🟢 | 🟢 | |
| UDF ISOs | 🟢 | 🟢 | 🟢 | read-only, 2048-byte blocks |
| El Torito boot image detection | 🟢 | 🟢 | 🟢 | |
| ISOHybrid: ISO mode / DD mode prompt | 🟢 | 🟢 | 🟢 | |
| DD write (.img .bin .raw ...) | 🟢 | 🟢 | 🟢 | Rustfus also verifies the write |
| Compressed images (.gz .bz2 .xz .lzma .zst .zip) | 🟢 | 🟢 | 🟢 | |
| VHD (fixed + dynamic), VHDX (dynamic) | 🟢 | 🟢 | 🟢 | differencing disks rejected |
| FFU images | 🔴 | 🟡 | 🔴 | DISM |

## Windows media

| Rufus feature | Linux | Windows | macOS | Notes |
|---|:-:|:-:|:-:|---|
| Windows install USB, FAT32 (UEFI + BIOS) | 🟢 | 🟢 | 🟢 | ms-sys boot code |
| Windows install USB, NTFS + UEFI:NTFS (files > 4 GB) | 🟡 | 🟢 | 🔴 | Linux needs `mkfs.ntfs` |
| Windows install USB, exFAT | 🔴 | 🔴 | 🔴 | refused: Windows Boot Manager can't read the BCD from exFAT (0xc0000272) |
| Windows 7 x64 UEFI boot (EFI loader extracted from install.wim) | 🔴 | 🔴 | 🔴 | |
| WUE: Remove requirement for 4 GB+ RAM, Secure Boot and TPM 2.0 | 🟢 | 🟢 | 🟢 | |
| WUE: Remove requirement for an online Microsoft account | 🟢 | 🟢 | 🟢 | |
| WUE: Create a local account | 🟢 | 🟢 | 🟢 | |
| WUE: Set regional options to the same values as this user's | 🟢 | 🟢 | 🟢 | |
| WUE: Disable data collection (skip privacy questions) | 🟢 | 🟢 | 🟢 | |
| WUE: Disable BitLocker automatic device encryption | 🟢 | 🟢 | 🟢 | |
| WUE: Prevent Windows To Go from accessing internal disks | 🔴 | 🟡 | 🔴 | follows Windows To Go |

## Linux and other boot media

| Rufus feature | Linux | Windows | macOS | Notes |
|---|:-:|:-:|:-:|---|
| UEFI boot of Linux ISOs | 🟢 | 🟢 | 🟢 | |
| BIOS boot via GRUB 2 | 🟢 | 🟢 | 🟢 | embedded core.img 2.14 |
| BIOS boot via Syslinux / Isolinux | 🟢 | 🟢 | 🟢 | embedded v4 / v6 |
| Download matching GRUB / Syslinux version (signed, rufus.ie) | 🟢 | 🟢 | 🟢 | needs `curl` (built into Windows 10+ / macOS) |
| Prompt before downloading GRUB / Syslinux files | 🔴 | 🔴 | 🔴 | Rustfus downloads signed files silently |
| Syslinux on NTFS / exFAT | 🔴 | 🔴 | 🔴 | |
| Knoppix EFI symlink workaround | 🔴 | 🔴 | 🔴 | |
| Persistent partition (Ubuntu casper / Debian live) | 🟢 | 🟢 | 🟢 | own ext3 formatter |
| ReactOS ISOs | 🟢 | 🟢 | 🟢 | booted through Syslinux v4 + mboot.c32 |
| Grub4DOS ISOs | 🔴 | 🔴 | 🔴 | |
| KolibriOS ISOs | 🔴 | 🔴 | 🔴 | |
| GPT "UEFI-only media" BIOS message | 🟢 | 🟢 | 🟢 | |
| Secure Boot revoked bootloader warning (DBX / SBAT / SVN) | 🔴 | 🔴 | 🔴 | |

## Application

| Rufus feature | Linux | Windows | macOS | Notes |
|---|:-:|:-:|:-:|---|
| Rufus-style window | 🟢 | 🟢 | 🟢 | egui |
| Languages (rufus.loc, 38) | 🟢 | 🟢 | 🟢 | |
| Log window (clear / save) | 🟢 | 🟢 | 🟢 | |
| About dialog | 🟢 | 🟢 | 🟢 | |
| Dark mode | 🟢 | 🟢 | 🟢 | |
| Drag and drop images | 🟢 | 🟢 | 🟢 | |
| Command line | 🟢 | 🟢 | 🟢 | `list`, `info`, `write`, `save`, `download`, `hash` |
| Privilege elevation | 🟢 | 🟢 | 🟡 | polkit / UAC manifest; macOS needs `sudo` |
| Settings remembered between runs (registry / rufus.ini) | 🔴 | 🔴 | 🔴 | |
| Update check | 🔴 | 🔴 | 🔴 | Rustfus has no release server yet |
| Alt-key cheat modes (Alt-Z zero drive, Alt-S skip size checks, ...) | 🔴 | 🔴 | 🔴 | |
