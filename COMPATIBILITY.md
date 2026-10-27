# Rustfus OS compatibility

🟢 works &nbsp;&nbsp; 🟡 partial / needs an extra tool &nbsp;&nbsp; 🔴 not available

"Works" means implemented and covered by automated tests on disk-image files.
Nothing has been tested on a physical USB drive yet, and the Windows and macOS
builds compile cleanly but have not been run on those systems.

## Drives and images

| Feature | Linux | Windows | macOS | Notes |
|---|:-:|:-:|:-:|---|
| List USB / removable drives | 🟢 | 🟢 | 🟢 | `lsblk` / `Get-Disk` / `diskutil` |
| List USB hard drives (fixed disks) | 🟢 | 🟢 | 🟢 | system disks always hidden |
| Hot-plug refresh | 🟢 | 🟢 | 🟢 | 2 s background scan |
| DD write + verify (.iso .img .bin .raw) | 🟢 | 🟢 | 🟢 | macOS uses fast `/dev/rdisk` |
| Compressed images (.gz .bz2 .xz .lzma .zst .zip) | 🟢 | 🟢 | 🟢 | |
| VHD (fixed + dynamic), VHDX (dynamic) | 🟢 | 🟢 | 🟢 | differencing disks rejected |
| FFU images (write) | 🔴 | 🟡 | 🔴 | DISM only exists on Windows |
| Save drive to .img / .vhd / .vhdx | 🟢 | 🟢 | 🟢 | |
| Save drive to .ffu | 🔴 | 🟡 | 🔴 | DISM |
| Image checksums (MD5/SHA-1/SHA-256/SHA-512) | 🟢 | 🟢 | 🟢 | |
| Bad blocks check (all Rufus modes) | 🟢 | 🟢 | 🟢 | |

## Bootable media (ISO mode)

| Feature | Linux | Windows | macOS | Notes |
|---|:-:|:-:|:-:|---|
| ISO9660 / Joliet / Rock Ridge extraction | 🟢 | 🟢 | 🟢 | |
| UDF images | 🟢 | 🟢 | 🟢 | read-only, 2048-byte blocks |
| ISOHybrid ISO/DD prompt | 🟢 | 🟢 | 🟢 | |
| Windows install USB, FAT32 (UEFI + BIOS) | 🟢 | 🟢 | 🟢 | own FAT32 formatter + ms-sys boot code |
| Windows install USB, NTFS + UEFI:NTFS (>4 GB files) | 🟡 | 🟢 | 🔴 | Linux needs `mkfs.ntfs`; macOS can't write NTFS |
| Windows install USB, exFAT | 🔴 | 🔴 | 🔴 | refused: UEFI:NTFS loads bootmgfw.efi, but Windows Boot Manager can't read the BCD from exFAT (0xc0000272) |
| Windows User Experience (unattend.xml) | 🟢 | 🟢 | 🟢 | |
| Windows To Go | 🔴 | 🟡 | 🔴 | DISM + bcdboot; untested |
| Linux ISO, UEFI boot | 🟢 | 🟢 | 🟢 | |
| Linux ISO, BIOS boot via GRUB 2 | 🟢 | 🟢 | 🟢 | embedded core.img 2.14 |
| GRUB version download (signed, rufus.ie) | 🟢 | 🟢 | 🟢 | needs `curl` (built into Windows 10+/macOS) |
| Linux ISO, BIOS boot via Syslinux, FAT32 | 🟢 | 🟢 | 🟢 | embedded v4/v6, others downloaded (signed) |
| Linux ISO, BIOS boot via Syslinux, NTFS | 🟡 | 🔴 | 🔴 | Syslinux 6+ only, like Rufus; Linux needs `mkfs.ntfs` and maps ldlinux.sys with FIEMAP (ntfs3) or FIBMAP (ntfs-3g) |
| Linux ISO, BIOS boot via Syslinux, exFAT | 🔴 | 🔴 | 🔴 | refused: Syslinux has no exFAT driver (GRUB 2 images fall back to GRUB) |
| Knoppix `/boot/syslinux` symlink (EFI Syslinux configs) | 🟢 | 🟢 | 🟢 | replaced by a directory of `syslnx32/64.cfg` redirects, like Rufus |
| Persistent partition (Ubuntu / Debian live) | 🟢 | 🟢 | 🟢 | own ext3 formatter |
| Runtime UEFI media validation (uefi-md5sum) | 🟢 | 🟢 | 🟢 | |
| FreeDOS | 🟢 | 🟢 | 🟢 | |
| GPT "UEFI-only media" BIOS message | 🟢 | 🟢 | 🟢 | |
| Old-BIOS fixes (63-sector alignment) | 🟢 | 🟢 | 🟢 | |

## Formatting (Non bootable)

| File system | Linux | Windows | macOS | Notes |
|---|:-:|:-:|:-:|---|
| FAT / FAT32 (any size) | 🟢 | 🟢 | 🟢 | own formatter, no 32 GB limit |
| NTFS | 🟡 | 🟢 | 🔴 | Linux: `mkfs.ntfs` (ntfs-3g) |
| exFAT | 🟡 | 🟢 | 🟢 | Linux: `mkfs.exfat` (exfatprogs) |
| UDF | 🟡 | 🟢 | 🟢 | Linux: `mkudffs` (udftools) |
| ReFS | 🔴 | 🟡 | 🔴 | Windows editions with ReFS support only |
| ext2 / ext3 / ext4 (empty) | 🟢 | 🟢 | 🟢 | own formatter |
| ext2/3/4 with ISO files copied | 🟢 | 🔴 | 🔴 | needs the OS to mount ext |
| Quick / full format, extended label + icon | 🟢 | 🟢 | 🟢 | |
| MBR / GPT partitioning | 🟢 | 🟢 | 🟢 | own writer |

## Application

| Feature | Linux | Windows | macOS | Notes |
|---|:-:|:-:|:-:|---|
| Rufus-style egui window | 🟢 | 🟢 | 🟢 | |
| 38 languages (rufus.loc) | 🟢 | 🟢 | 🟢 | |
| Log, About, dark mode | 🟢 | 🟢 | 🟢 | |
| Drag and drop images | 🟢 | 🟢 | 🟢 | |
| Windows ISO download (Fido) | 🟢 | 🟢 | 🟢 | Runs Fido.ps1 when PowerShell (`pwsh`) is installed, else built-in |
| Update check (GitHub releases) | 🟢 | 🟢 | 🟢 | asks on first launch; daily/weekly/monthly, optional betas; needs `curl` |
| Runs without root / Administrator | 🔴 | 🔴 | 🔴 | raw disk access needs elevation |
