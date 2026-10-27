# Changelog

All notable changes to Rustfus are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [0.1.0] - Unreleased

First public release. Untested on physical drives; see the README.

### Added

- Rufus-style egui interface and a `rustfus` command line (`list`, `info`,
  `write`, `save`, `download`, `hash`).
- DD mode with verification, including compressed and `.vhd`/`.vhdx` images.
- ISO mode for ISO9660/Joliet/Rock Ridge/UDF images onto FAT32, NTFS or exFAT.
- Windows install media: UEFI and BIOS boot, UEFI:NTFS, Windows User Experience
  options, Windows To Go.
- Linux live media: GRUB 2 and Syslinux, persistent partitions. Like Rufus,
  replacing an image's obsolete `menu.c32`/`vesamenu.c32` asks first (or needs
  `--allow-downloads` on the command line).
- FreeDOS, bad blocks check, checksums, runtime UEFI media validation.
- Built-in MBR/GPT, FAT16/FAT32 and ext2/3/4 formatters.
- Windows ISO download through Fido, with or without PowerShell.
- Saving drives to `.img`, `.vhd` or `.vhdx`.
- 38 languages from Rufus' translations.
- Release packages: Windows `.exe`, macOS `.dmg`, AppImage, `.deb`, `.rpm`
  and Arch `.pkg.tar.zst`.

[0.1.0]: https://github.com/LOSEARDES77/rustfus/releases/tag/v0.1.0
