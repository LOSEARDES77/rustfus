# Rufus feature parity

Per-OS status with colors: see [COMPATIBILITY.md](COMPATIBILITY.md).

Not implemented:
- Syslinux on NTFS/exFAT, Knoppix EFI symlink workaround
- Prompts before Syslinux c32 replacement downloads (Rufus asks; Rustfus downloads signed files silently)
- Self-update: Rufus downloads and relaunches the new version; Rustfus opens the
  GitHub release page, since it ships as several package formats
