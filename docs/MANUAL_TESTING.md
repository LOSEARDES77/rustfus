# Manual hardware testing

Automated tests write to disk-image files. This checklist covers what they
can't: real USB drives, real firmware and each host OS. Run it before a
release and whenever a write path changes.

**Every test here erases the target drive.** Use a spare USB stick with
nothing on it you want to keep, and unplug every other external disk first.

## Before you start

- Note the target's path and size as Rustfus shows them, and check both
  against the OS disk utility (`lsblk`, Disk Management, Disk Utility).
- Linux: run as root (`sudo rustfus`, or the menu entry through pkexec).
  Windows: accept the UAC prompt. macOS: run the binary with `sudo`.
- Keep a second machine (or a VM with USB passthrough) to boot the media.
- For each run, record: Rustfus version or commit, host OS and architecture,
  drive model and size, image name and its SHA-256, and the result.

## Safety guards

These must fail without touching the drive. Check the drive is unchanged
afterwards.

- [ ] Pick a drive, unplug it, then press Start. Rustfus refuses with "is no
      longer connected".
- [ ] Pick a drive, swap it for a different stick on the same port, then press
      Start. Rustfus refuses with "is now a different drive" (Linux and
      Windows; macOS has no serial to compare, so only a size change is caught).
- [ ] Linux: hold a partition with device-mapper
      (`sudo dmsetup create rustfus-test --table "0 2048 linear /dev/sdX1 0"`),
      then write. Rustfus refuses with "is in use". Clean up with
      `sudo dmsetup remove rustfus-test`.
- [ ] Internal and system disks never show up in the list, also with
      the option to list USB hard drives on.
- [ ] Cancel the confirmation dialog: nothing is written.
- [ ] CLI: `rustfus write -d <internal disk>` is refused because the disk is not
      in `rustfus list`.

## Write modes

Boot each result on real hardware, in UEFI mode and in BIOS/CSM mode where
the target allows it. Stop Windows Setup at the language screen; don't
install onto a real disk.

| Test | Linux | Windows | macOS |
|---|:-:|:-:|:-:|
| Format only: FAT32, exFAT, NTFS, ext4; drive mounts and reads back | | | |
| DD mode: Linux `.img.xz`, with verify on; boots | | | |
| ISO mode: Ubuntu or Fedora, FAT32, GPT/UEFI; boots | | | |
| ISO mode: Syslinux ISO (e.g. older Debian), MBR/BIOS; boots | | | |
| Linux ISO with persistence; a file survives a reboot | | | |
| Windows 11 ISO, GPT/UEFI (UEFI:NTFS if over 4 GB); reaches Setup | | | |
| Windows 11 ISO, MBR/BIOS+UEFI; reaches Setup in both modes | | | |
| Windows User Experience options (TPM/Secure Boot bypass, local account) applied | | | |
| Windows To Go (Windows host only) boots to the desktop | | | 🔴 |
| FreeDOS boots to a prompt | | | |
| Bad blocks check, 1 pass, finishes and reports 0 bad sectors | | | |
| Save drive to `.img` and `.vhdx`; image matches the drive's SHA-256 | | | |
| Windows ISO download (Fido) finishes and the hash matches Microsoft's | | | |

Mark each cell ✅, ❌ (with a link to the issue) or leave it empty if not run.

## Interruptions

- [ ] Cancel during a write: Rustfus stops, reports "Cancelled", and the drive
      can be written again afterwards.
- [ ] Unplug the drive during a write: Rustfus reports an error and doesn't hang.
- [ ] Close the window during a write, then write the same drive again: the
      second write succeeds.

## Interface

- [ ] Plug and unplug a stick: the list updates within a few seconds.
- [ ] Switch language: labels change and nothing overflows.
- [ ] 100%, 150% and 200% display scaling look right.
- [ ] Keyboard only: Tab reaches every control, and Space or Enter activates
      the focused one.
