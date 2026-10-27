# Rufus feature parity

Per-OS status with colors: see [COMPATIBILITY.md](COMPATIBILITY.md).

Not implemented:
- Update check (Rustfus has no release server yet)
- Syslinux on NTFS from Windows (Linux only: Rustfus maps ldlinux.sys with FIEMAP/FIBMAP;
  Rufus uses FSCTL_GET_RETRIEVAL_POINTERS). Syslinux on exFAT is refused, as Syslinux has
  no exFAT driver and Rufus doesn't offer exFAT for ISOs
- Prompts before Syslinux c32 replacement downloads (Rufus asks; Rustfus downloads signed files silently)
