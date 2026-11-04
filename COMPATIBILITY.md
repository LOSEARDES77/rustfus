# Rustfus OS compatibility

🟢 implemented &nbsp;&nbsp; 🟡 partial / extra tool / validation pending &nbsp;&nbsp; 🔴 unavailable or unsafe (see notes)

Coverage reviewed against source at `6b25880` on 2026-11-04. The seven CI jobs
passed, including Intel/Apple Silicon macOS, Linux and Windows. CI success does
not prove physical-disk safety, firmware boot, installer behavior or UAC prompts.
Green describes the implemented path, not a claim that every configuration has
an end-to-end test. The coverage and test tables below identify remaining gaps.

macOS GUI and local disk-image tests have run, but physical-USB raw writes have
reported EPERM/ENOTTY failures. The device-flush fix passed a disposable virtual
disk test; physical USB remains unverified. The Windows-UEFI `diskutil` path also
requires physical-USB and boot validation. This raw-backend caveat applies to
macOS BIOS media, formatting and other raw operations, even where their shared
formatters or bootloader writers are implemented.

## Safety blocker: Windows drive capture

**Do not use Windows Save on valuable media.** `ops::save_drive()` currently calls
`ops::unmount()`, whose Windows implementation runs `Clear-Disk -RemoveData
-RemoveOEM`. That can remove source partition metadata before capture. IMG, VHD,
VHDX and FFU saves share this path. This is a source-code finding, not a
destructive reproduction on a physical drive.

P0 work: separate read-only capture preparation from destructive write
preparation; replace destructive unmounting with checked volume locking and
dismounting. Propagate failures instead of `-ErrorAction SilentlyContinue`.
Validate source preservation on disposable virtual disks before enabling normal
backup testing. The current UI does not enforce this documentation warning.

## Drives and images

| Feature | Linux | Windows | macOS | Notes |
|---|:-:|:-:|:-:|---|
| List USB / removable drives | 🟢 | 🟢 | 🟢 | `lsblk` / `Get-Disk` / `diskutil` |
| List USB hard drives (fixed disks) | 🟢 | 🟢 | 🟢 | system disks always hidden |
| Hot-plug refresh | 🟢 | 🟢 | 🟢 | 2 s background scan |
| DD write + verify (.iso .img .bin .raw) | 🟢 | 🟢 | 🟡 | macOS raw `/dev/rdisk` writes have unresolved user-reported failures |
| Compressed image input (.gz .bz2 .xz .lzma .zst .zip) | 🟢 | 🟢 | 🟢 | decoding support; destination writes inherit raw-backend limitations |
| VHD (fixed + dynamic), VHDX (dynamic) input | 🟢 | 🟢 | 🟢 | file-based round-trip/interoperability tests; differencing disks rejected |
| FFU images (restore) | 🔴 | 🟡 | 🔴 | Windows DISM wrapper exists; native capture/restore round trip, cancellation and progress handling need validation/work |
| Save drive to .img / .vhd / .vhdx | 🟢 | 🔴 | 🟡 | Windows unsafe: see safety blocker; VHD output is fixed, VHDX output is dynamic; macOS physical capture unverified |
| Save drive to .ffu | 🔴 | 🔴 | 🔴 | Windows capture wrapper exists but shares the unsafe save preparation; DISM-only, round trip untested |
| Image checksums (MD5/SHA-1/SHA-256/SHA-512) | 🟢 | 🟢 | 🟢 | |
| Bad blocks check (all Rufus modes) | 🟢 | 🟢 | 🟢 | |
| Fake USB capacity / address-wrap detection | 🟡 | 🟡 | 🟡 | sector-address patterns exist in destructive bad-block checks; simulated wraparound/failure tests missing; no dedicated capacity report |
| Exclusive drive access / volume locking | 🟡 | 🟡 | 🟡 | Linux raw writer uses O_EXCL, not every device path; Windows needs non-destructive volume locks; macOS force-unmount is not a complete exclusive-access guarantee |
| Identify conflicting processes | 🔴 | 🔴 | 🔴 | no process-holder enumeration; implement actionable errors without terminating processes automatically |

## Bootable media (ISO mode)

| Feature | Linux | Windows | macOS | Notes |
|---|:-:|:-:|:-:|---|
| ISO9660 / Joliet / Rock Ridge extraction | 🟢 | 🟢 | 🟢 | |
| UDF images | 🟢 | 🟢 | 🟢 | read-only, 2048-byte blocks |
| ISOHybrid ISO/DD prompt | 🟢 | 🟢 | 🟢 | |
| Windows install USB, FAT32 (UEFI + BIOS) | 🟢 | 🟢 | 🟡 | macOS UEFI/GPT uses diskutil + mounted copy; physical USB/boot unverified, BIOS remains raw |
| Windows install USB, NTFS + UEFI:NTFS (>4 GB files) | 🟡 | 🟢 | 🟡 | bundled FUSE-free NTFS-3G image preparation; temporary partition-image space required; physical USB/boot unverified |
| FAT32 install.wim splitting | 🟢 | 🔴 | 🟡 | bundled wimlib on Linux/macOS; validates parts before erasure; oversized ESD requires NTFS |
| Windows install USB, exFAT | 🔴 | 🔴 | 🔴 | refused: UEFI:NTFS loads bootmgfw.efi, but Windows Boot Manager can't read the BCD from exFAT (0xc0000272) |
| Windows User Experience (autounattend.xml) | 🟡 | 🟡 | 🟡 | generation and selected-option tests exist; installer/combination coverage pending; see customization mapping |
| Windows To Go | 🔴 | 🟡 | 🔴 | DISM + bcdboot pipeline exists; deployment/boot untested; answer-file architecture is hard-coded amd64; experimental, not official Windows support |
| Linux ISO, UEFI boot | 🟢 | 🟢 | 🟡 | macOS still uses the raw backend; reported writes remain unresolved |
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
| NTFS | 🟢 | 🟢 | 🟡 | bundled NTFS-3G formatter and image writer on Linux/macOS, no FUSE; physical macOS validation pending |
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
| Rufus-style GPUI Kit window | 🟢 | 🟢 | 🟢 | |
| 38 languages (rufus.loc) | 🟢 | 🟢 | 🟢 | |
| Log, About, dark mode | 🟢 | 🟢 | 🟢 | |
| Drag and drop images | 🟢 | 🟢 | 🟢 | |
| Windows ISO download (Fido) | 🟢 | 🟢 | 🟢 | Runs Fido.ps1 when PowerShell (`pwsh`) is installed, else built-in |
| Update check (GitHub releases) | 🟢 | 🟢 | 🟢 | asks on first launch; daily/weekly/monthly, optional betas; needs `curl` |
| Runs without root / Administrator | 🔴 | 🔴 | 🔴 | raw disk access needs elevation |

## Windows installation customization coverage

These options configure the created installation media, usually through
`autounattend.xml`; they do not rewrite the source ISO. Normal-install answer
files can be generated on Linux, Windows and macOS. Windows To Go deployment
remains Windows-only. Generated XML tests are not proof that a Windows build
accepts or honors an option. 🟡 means an implementation exists but needs work or
installer validation; 🔴 means the requested option is not implemented.

| Requested option | Status | Current implementation | Work to implement or validate |
|---|:-:|---|---|
| Bypass TPM 2.0 requirement | 🟡 | `bypass_hw_checks` emits LabConfig `BypassTPMCheck` | split the grouped hardware option if independent selection is required; test supported installer builds |
| Bypass Secure Boot requirement | 🟡 | same grouped option emits `BypassSecureBootCheck` | bypasses Setup's requirement, not firmware signature enforcement; test independently |
| Bypass RAM requirement | 🟡 | same grouped option emits `BypassRAMCheck` | independent selection and installer tests |
| Remove Microsoft-account requirement | 🟡 | `no_online_account` emits `BypassNRO` | build-dependent; test disconnected/connected OOBE and explain limitations instead of guaranteeing a universal bypass |
| Create a local account | 🟡 | `local_account` emits account XML and `net user` commands | validate reserved/invalid usernames and harden command construction; XML escaping alone is not shell-argument safety |
| Copy current user's regional settings | 🟡 | `regional` uses one locale for multiple fields | capture keyboard, locale and timezone separately; account for languages available in the target image |
| Disable data-collection questions | 🟡 | `no_data_collection` emits OOBE settings | test actual question suppression; do not describe this as disabling all telemetry |
| Disable automatic BitLocker device encryption | 🟡 | `no_bitlocker` emits a registry command | add/validate the documented `PreventDeviceEncryption` answer-file component; opt-in with an encryption warning; does not decrypt existing disks |
| Disable forced applications / QoL settings | 🔴 | missing | define explicit apps/settings and supported builds; separate opt-ins rather than an unrestricted removal promise |
| Set internal drives offline for Windows To Go | 🟡 | `wtg_no_internal` applies SAN policy 4 | WTG-only; fix architecture selection and verify internal disks stay offline on a disposable test system |
| Restrict installation to Windows S Mode | 🔴 | missing | add image/edition compatibility gates and conflict checks with account customizations; advanced opt-in |
| Copy `SkuSiPolicy.p7b` to ESP | 🔴 | missing | use the installed target's matching policy, not the authoring host's; gate on availability and warn about boot/recovery compatibility; separate from firmware DBX updates |
| Silent installation | 🔴 | missing | implement last: explicit destructive consent, target-disk selection, edition/account prerequisites, ambiguous-target refusal and installer VM tests |

## Implementation roadmap

Priorities below are proposed work, not available features. P0 covers safety and
the existing core workflow; P1 adds features after those checks pass; P2 covers
advanced or narrower workflows. Traffic lights describe current coverage, not
priority or completion of the proposed work: 🟢 implemented, 🟡 partial or
validation pending, 🔴 missing or unsafe. A mixed row uses 🔴 when an unsafe path
blocks the workflow; its coverage notes identify any existing components.

| Priority | Feature / gap | Status | Current coverage | Next deliverable |
|---|---|:-:|---|---|
| P0 | Non-destructive capture and drive locking | 🔴 | unsafe Windows save preparation; partial exclusivity elsewhere | separate capture/write preparation; checked locking/dismounting, identity checks and source-preservation regression tests |
| P0 | Conflicting-process diagnostics | 🔴 | missing process identification | report the busy volume and available holder information; fail safely without automatic process termination |
| P0 | Existing Windows customization hardening | 🟡 | generator tests only; grouped hardware options and incomplete regional settings | safe account validation, parsed XML/option-combination tests, supported-build installer matrix and capability gating |
| P0 | Fake USB capacity detection | 🟡 | sector-address patterns implemented, fault coverage missing | simulated alias/wraparound/read/write faults; actionable destructive-test reporting and cancellation tests |
| P0 | macOS media creation validation | 🟡 | managed UEFI and raw paths implemented; physical validation pending | spare-USB copy/readback and firmware boot tests for FAT32/WIM splitting, NTFS/UEFI:NTFS and DD separately |
| P1 | Revoked Secure Boot bootloader detection | 🔴 | missing | read-only preflight warning with a versioned revocation source; state firmware DBX visibility and freshness limits; never claim a universal boot verdict |
| P1 | FFU capture / restore / compressed FFU | 🔴 | Windows DISM wrappers, unsafe capture preparation, no round-trip coverage | fix capture first; validate DISM options, failure/cancellation/progress handling and disposable-disk round trips; do not promise cross-platform support |
| P1 | VHD/VHDX save/restore validation | 🟡 | file tests; fixed VHD and zero-skipping dynamic VHDX output | native disposable-disk round trips and external-reader checks; validate sparse output separately from compression |
| P1 | Compressed backup output | 🔴 | compressed input supported; general compressed saving missing | define output format/extension contracts, streaming cancellation and restoration tests; dynamic VHDX is not general compression |
| P1 | Windows QoL / application suppression | 🔴 | missing | explicit reversible settings where possible, per-build gates and opt-in installer tests |
| P2 | Windows To Go completion | 🟡 | Windows pipeline exists; metadata/XML tests only | fix architecture handling, test deployment/boot and SAN policy; retain experimental status |
| P2 | Existing drive to ISO | 🔴 | no ISO-authoring path | define filesystem/El Torito authoring separately from raw backup; renaming IMG to ISO is not conversion |
| P2 | Standalone bootloader installation | 🟡 | bootloader writers exist within media-creation jobs | define supported loader/filesystem combinations and partition-preservation guarantees before exposing a standalone operation |
| P2 | S Mode installation restriction | 🔴 | missing | implement edition/build detection and incompatible-option validation before exposing the option |
| P2 | `SkuSiPolicy.p7b` deployment | 🔴 | missing | target-matched, opt-in policy application with recovery compatibility tests; no host-policy copying |
| P2 | UEFI DBX / Secure Boot database update | 🔴 | missing | investigate read-only inventory first; firmware mutation requires separate authorization, vendor guidance and recovery planning |
| P2 | Silent Windows installation | 🔴 | missing | explicit installation-target safeguards and destructive-consent flow; never silently select the first internal disk |
| P2 | In-place Windows upgrade restriction bypass | 🔴 | missing; booted Setup LabConfig is not upgrade coverage | separate capability/design and supported-build tests; do not infer it from installation-media customization |
| P2 | Microsoft Dev Drive detection | 🔴 | missing; generic ReFS formatting is not Dev Drive support | detect and report Dev Drive identity and protect it from accidental selection |

## Validation order and acceptance criteria

Do not use valuable drives for destructive testing. Keep production internal
disks disconnected from installer/WTG tests where possible. Physical USB and
firmware boot results must be recorded separately from CI and image-file tests.

| Order | Environment | Tests / acceptance criteria |
|---|---|---|
| 1 | Automated tests and disposable Windows virtual disks | fix save preparation first; compare source bytes and partition metadata before/after IMG/VHD/VHDX/FFU capture, including failure and cancellation; busy/identity-mismatched targets must fail without destructive fallback |
| 2 | File-backed media and fault-injected I/O | test bad-block address wraparound, read/write errors, cancellation and reporting; preserve existing NTFS/WIM preflight and malformed-input tests |
| 3 | Disposable Windows installer VMs | parse generated XML; test each option and conflicting combinations against recorded ISO builds/editions/architectures; verify account, locale/keyboard/timezone, OOBE and encryption outcomes; distinguish connected and disconnected setup |
| 4 | Disposable Windows disks | FFU capture/apply and VHD/VHDX round trips after the safety fix; verify partitions, files and external-reader interoperability; exercise DISM failure and cancellation |
| 5 | Spare physical USB, then firmware boot | test macOS managed FAT32 with WIM splitting, NTFS/UEFI:NTFS and raw DD separately; check readback, reconnect behavior and actual UEFI/BIOS boot; retain EPERM/ENOTTY diagnostics |
| 6 | Isolated WTG / advanced installer tests | verify target architecture and internal-drive isolation; S Mode, policy deployment and silent install need separate prerequisite/conflict checks and recovery plans |

## Audit references

Implementation and tests:

* [`ops/mod.rs`](crates/rustfus-core/src/ops/mod.rs): `unmount`, `open_dev` and `save_drive` safety paths.
* [`image.rs`](crates/rustfus-core/src/image.rs) and [image tests](crates/rustfus-core/tests/unit/image.rs): bad-block patterns, image capture and DISM wrappers.
* [`wue.rs`](crates/rustfus-core/src/wue.rs), [WUE tests](crates/rustfus-core/tests/unit/wue.rs) and [GUI dialogs](crates/rustfus-gui/src/dialogs.rs): customization fields, generated XML and exposed options.
* [`ops/to_go.rs`](crates/rustfus-core/src/ops/to_go.rs) and [WTG tests](crates/rustfus-core/tests/unit/wtg.rs): deployment pipeline versus metadata/XML coverage.
* [Native helper validation](docs/native-tools.md): bundled NTFS-3G/wimlib builds and image-file test limits.

Upstream behavior and platform constraints:

* [Rufus WUE implementation, audited revision](https://github.com/pbatard/rufus/blob/acf7de640848ab249bedeb6feb59b33001f656b6/src/wue.c).
* [Microsoft PreventDeviceEncryption](https://learn.microsoft.com/en-us/windows-hardware/customize/desktop/unattend/microsoft-windows-securestartup-filterdriver-preventdeviceencryption).
* [Microsoft Windows To Go overview](https://learn.microsoft.com/en-us/windows/deployment/planning/windows-to-go-overview): official Windows To Go was removed in Windows 10 version 2004 and later; a custom portable deployment is not official Windows support.
