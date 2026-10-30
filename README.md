<p align="center"><img src="res/rustfus.png" width="128" alt="Rustfus icon"></p>

# Rustfus

**Rustfus** is a Rust rewrite of [Rufus](https://rufus.ie), the Reliable USB
Formatting Utility, with an [egui](https://github.com/emilk/egui) interface
that mirrors the original. Its purpose is to bring the Rufus tool to
platforms where it doesn't run: **Linux and macOS**, as well as Windows.

## Why

I love Rufus, and I wanted a way to run it on Linux specifically. Every time
I needed it, I ended up spinning up a Windows VM just to make a bootable USB.
Rustfus saves me from doing that.

> **This project is AI-assisted.** It was written with an AI coding assistant
> (Claude Code), using the Rufus source code as the reference. It has
> automated tests; the Windows and macOS builds compile but haven't been run
> on those systems. Creating bootable media erases the selected drive: double-check
> the device you pick, and use it at your own risk.

Rustfus is an independent project. It is not affiliated with or endorsed by
Rufus or its author, Pete Batard. All credit for the original design and
behaviour goes to Rufus.

## Features

- Rufus layout: Drive Properties, Format Options, Status, advanced sections
- Write images in **DD mode** (with verification), including compressed
  (`.gz .bz2 .xz .lzma .zst .zip`) and virtual disk (`.vhd .vhdx`) images
- **ISO mode**: copy ISO9660/Joliet/Rock Ridge/UDF images to FAT32, NTFS or exFAT
- Windows install media: UEFI and BIOS boot, UEFI:NTFS for files over 4 GB,
  Windows User Experience options (TPM/Secure Boot/RAM bypass, local account…),
  Windows To Go
- Linux live media: GRUB 2 and Syslinux BIOS boot (matching versions
  downloaded from rufus.ie and signature-checked), persistent partitions
- FreeDOS, runtime UEFI media validation, bad blocks check, checksums
- Own MBR/GPT, FAT16/FAT32 and ext2/3/4 formatters, so they work on every OS
- Windows ISO download through Fido
- Save drives to `.img`, `.vhd` or `.vhdx`
- 38 languages, taken from Rufus' translations
- Update check against GitHub releases, with Rufus' update policy settings

See [COMPATIBILITY.md](COMPATIBILITY.md) for what works on each OS.

## Building

Requires Rust 1.95 or newer (edition 2024).

```sh
cargo build --release
```

The executable is `target/release/rustfus` (`rustfus.exe` on Windows).

The code is split into a workspace:

| Crate | Contents |
|---|---|
| `crates/rustfus-cli` | The `rustfus` executable (the only binary): the command line, or the GUI when run without arguments |
| `crates/rustfus-core` | Engine: drive listing, image analysis, partitioning, file systems, boot code, downloads |
| `crates/rustfus-gui` | The Rufus-style egui interface |

## Command line

Run `rustfus` without arguments for the GUI, or with a command:

```sh
rustfus list                                   # drives Rustfus can write to (--all adds USB hard drives)
rustfus info ubuntu.iso                        # what Rustfus detects and the settings it would pick
sudo rustfus write -d /dev/sdX ubuntu.iso      # ISO mode, with the same defaults as the GUI
sudo rustfus write -d /dev/sdX disk.img.xz     # DD mode (non-ISO images are always written raw)
sudo rustfus write -d /dev/sdX --freedos       # FreeDOS boot drive
sudo rustfus write -d /dev/sdX --fs exfat -l DATA   # format only
sudo rustfus save -d /dev/sdX backup.vhd       # save a drive to an image
rustfus download --win "Windows 11" --lang Spanish  # official Windows ISO
rustfus download --win "Windows 11" --list     # list the choices for the next option
rustfus hash ubuntu.iso
rustfus check-update                           # newer release on GitHub? (--betas includes pre-releases)
```

`rustfus <command> --help` lists every option (partition scheme, target, cluster
size, persistence, bad blocks, Windows customizations, ...). `write` only accepts
drives that `rustfus list` shows, and asks before erasing unless given `--yes`.

## Installing

Download from the [Releases](../../releases) page:

| System | File | Notes |
|---|---|---|
| Windows (x86_64 / ARM64) | `Rustfus-<version>-windows-<arch>.exe` | Portable, asks for Administrator rights (UAC) at launch |
| macOS 11+ (Intel / Apple Silicon) | `Rustfus-<version>-macos-universal.dmg` | Unsigned: right-click → *Open* the first time. Run with `sudo /Applications/Rustfus.app/Contents/MacOS/rustfus` |
| Debian, Ubuntu, Mint… | `rustfus_<version>-1_<arch>.deb` | `sudo apt install ./rustfus_*.deb` |
| Fedora, openSUSE, RHEL… | `rustfus-<version>-1.<arch>.rpm` | `sudo dnf install ./rustfus-*.rpm` |
| Arch, Manjaro, CachyOS… | `rustfus-<version>-1-x86_64.pkg.tar.zst` | `sudo pacman -U rustfus-*.pkg.tar.zst` |
| Any Linux | `Rustfus-<version>-<arch>.AppImage` | `chmod +x` and run; asks for your password via pkexec |

The Linux packages add a menu entry that starts Rustfus through `pkexec`
(a polkit policy is installed), because writing to disks needs root.

Optional tools on Linux: `ntfs-3g`, `exfatprogs` and `udftools` for those
file systems. The Windows ISO download runs Rufus' Fido script when PowerShell 7
(`pwsh`) is installed, and makes the same queries itself otherwise.

## Testing

```sh
cargo test --workspace                # unit and disk-image tests
cargo test --workspace -- --ignored   # also network tests against rufus.ie
```

Unit tests live in each crate's `tests/unit/`, mirroring `src/`; each module
includes its file with `#[path]`, so tests can reach private functions.

Some tests use `xorriso`, `fsck.vfat`, `e2fsprogs`, `sfdisk` or `qemu-img`
when available and skip themselves otherwise.

Tests on real USB drives and firmware follow
[docs/MANUAL_TESTING.md](docs/MANUAL_TESTING.md).

## Releases

Pushing a version tag builds, tests and publishes a GitHub release
(`.github/workflows/release.yml`):

```sh
# bump `version` in Cargo.toml first; the tag must match it
git tag v0.1.0
git push origin v0.1.0
```

The release contains the files listed under [Installing](#installing),
`SHA256SUMS.txt`, and a `*-third-party-sources.tar` with the upstream
source of every GPL binary embedded from `res/` (listed in
[res/SOURCES.txt](res/SOURCES.txt)). Tags with a hyphen (`v0.2.0-beta1`) are
published as pre-releases; the update check only offers them to users who
enabled "Include beta versions".

## Contributing

Code follows the [Google Rust Style Guide](https://google.github.io/styleguide/rustguide.html).
Before sending a pull request, run what CI runs:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Changes are recorded in [CHANGELOG.md](CHANGELOG.md).

## License

Rustfus is free software, licensed under the
[GNU General Public License v3.0 or later](LICENSE), the same license as
Rufus. It is derived from Rufus and embeds GPL boot loaders, so a more
permissive license isn't possible.

Embedded third-party components (ms-sys boot records, Syslinux, GRUB, FreeDOS,
UEFI:NTFS, uefi-md5sum, Rufus translations) and their licenses are listed in
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). Note that the Windows boot
code in `res/ms-sys/` is Microsoft's and is not free software.

## Acknowledgements

- [Rufus](https://github.com/pbatard/rufus) by Pete Batard, and its translators
- ms-sys, Syslinux, GRUB, FreeDOS, ntfs-3g, EfiFs, UEFI:NTFS, uefi-md5sum and Fido
- The Rust crates listed in `Cargo.lock`
