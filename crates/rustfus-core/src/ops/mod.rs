// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
use crate::drives::Drive;
#[cfg(not(windows))]
use crate::drives::run;
#[cfg(windows)]
use crate::fsos;
use crate::{image, iso, syslinux, wue};
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

mod dd;
mod files;
mod format;
mod layout;
#[cfg(test)]
#[path = "../../tests/unit/ops.rs"]
mod tests;
mod to_go;

use dd::write_dd;
use format::write_fs;
use to_go::write_wtg;

pub enum Msg {
    Progress(f32, String),
    Log(String),
    Done(Result<String, String>),
}

#[derive(Clone, Copy, PartialEq, Debug)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum Scheme {
    Mbr,
    Gpt,
}

#[derive(Clone, Copy, PartialEq, Debug)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum Target {
    BiosUefi,
    Uefi,
    Bios,
}

impl Target {
    pub fn name(self) -> String {
        crate::loc::t(match self {
            Target::BiosUefi => "MSG_033",
            Target::Uefi => "MSG_032",
            Target::Bios => "MSG_031",
        })
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum Fs {
    Fat,
    Fat32,
    Ntfs,
    Udf,
    Exfat,
    Refs,
    Ext2,
    Ext3,
    Ext4,
}

impl Fs {
    pub const ALL: [Fs; 9] = [
        Fs::Fat,
        Fs::Fat32,
        Fs::Ntfs,
        Fs::Udf,
        Fs::Exfat,
        Fs::Refs,
        Fs::Ext2,
        Fs::Ext3,
        Fs::Ext4,
    ];

    pub fn name(self) -> &'static str {
        [
            "FAT", "FAT32", "NTFS", "UDF", "exFAT", "ReFS", "ext2", "ext3", "ext4",
        ][self as usize]
    }

    pub fn is_fat(self) -> bool {
        matches!(self, Fs::Fat | Fs::Fat32)
    }

    fn mbr_type(self) -> u8 {
        match self {
            Fs::Fat => 0x0e,
            Fs::Fat32 => 0x0c,
            Fs::Ext2 | Fs::Ext3 | Fs::Ext4 => 0x83,
            _ => 0x07,
        }
    }

    /// Cluster sizes offered in the UI; 0 is "(Default)".
    pub fn clusters(self) -> Vec<u32> {
        let range = |lo: u32, hi: u32| {
            std::iter::once(0)
                .chain((lo.ilog2()..=hi.ilog2()).map(|s| 1 << s))
                .collect()
        };
        match self {
            Fs::Fat | Fs::Fat32 => range(512, 65536),
            Fs::Ntfs => range(512, 2 << 20),
            Fs::Exfat => range(512, 32 << 20),
            Fs::Refs => vec![0, 4096, 65536],
            Fs::Ext2 | Fs::Ext3 | Fs::Ext4 => range(1024, 65536),
            Fs::Udf => vec![0],
        }
    }

    /// Filesystems the host can create (FAT is built in everywhere).
    pub fn supported(self) -> bool {
        match self {
            Fs::Fat | Fs::Fat32 | Fs::Exfat | Fs::Udf => true,
            Fs::Refs => cfg!(windows),
            Fs::Ext2 | Fs::Ext3 | Fs::Ext4 => true,
            Fs::Ntfs => !cfg!(target_os = "macos"),
        }
    }

    pub fn max_label(self) -> usize {
        match self {
            Fs::Fat | Fs::Fat32 | Fs::Exfat => 11,
            Fs::Ext2 | Fs::Ext3 | Fs::Ext4 => 16,
            _ => 32,
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum Boot {
    NonBootable,
    FreeDos,
    Image(PathBuf),
}

#[derive(Clone)]
pub struct Job {
    pub drive: Drive,
    pub boot: Boot,
    pub dd: bool,
    pub scheme: Scheme,
    pub target: Target,
    pub fs: Fs,
    pub cluster: u32,
    pub label: String,
    pub quick: bool,
    pub ext_label: bool,
    pub bad_blocks: Option<u8>,
    pub verify: bool,
    pub old_bios: bool,
    pub uefi_validation: bool,
    pub persistence: u64,
    pub wue: wue::Options,
    /// User agreed to fetch boot files matching the image (GRUB core.img, Syslinux).
    pub allow_downloads: bool,
    /// Obsolete menu.c32/vesamenu.c32 in the image (from `obsolete_c32`) the user
    /// agreed to replace with downloaded copies.
    pub replace_c32: Vec<String>,
    /// Windows To Go: install.wim image index to apply instead of copying files.
    pub wtg: Option<u32>,
}

/// What Rufus' image scan reports, enough to pick defaults and boot code.
#[derive(Clone, Default, Debug)]
pub struct Report {
    pub is_iso: bool,
    pub label: String,
    pub windows: bool,
    pub efi: bool,
    pub bios: bool,
    pub grub2: bool,
    pub grub2_version: String,
    /// Obsolete menu.c32/vesamenu.c32 of a Syslinux < 5 image (ISO paths).
    pub old_c32: Vec<String>,
    /// Syslinux the image boots with, never set for Windows images.
    pub syslinux: Option<syslinux::Info>,
    pub large_file: bool,
    pub hybrid: bool,
    pub casper: bool,
    pub debian_live: bool,
    pub size: u64,
    pub arch: &'static str,
}

impl Report {
    pub fn persistence_capable(&self) -> bool {
        self.casper || self.debian_live
    }

    /// Rufus' file system for an ISO: NTFS when a file is too big for FAT32.
    pub fn default_fs(&self) -> Fs {
        if self.large_file { Fs::Ntfs } else { Fs::Fat32 }
    }

    /// Rufus' partition scheme and target system for an ISO.
    pub fn default_layout(&self) -> (Scheme, Target) {
        if self.efi && (self.windows || !self.bios) {
            (Scheme::Gpt, Target::Uefi)
        } else if self.efi {
            (Scheme::Mbr, Target::BiosUefi)
        } else {
            (Scheme::Mbr, Target::Bios)
        }
    }
}

const FAT_MAX_FILE: u64 = (4 << 30) - 1;
const MB: u64 = 1 << 20;
const CHUNK: usize = 4 << 20;
const SECTOR: usize = 512;
static GRUB2_CORE: &[u8] = include_bytes!("../../../../res/grub2/core.img");
const GRUB2_VERSION: &str = "2.14";
static UEFI_NTFS: &[u8] = include_bytes!("../../../../res/uefi/uefi-ntfs.img");
static AUTORUN_ICO: &[u8] = include_bytes!("../../../../res/rustfus.ico");
static MD5_LOADERS: [(&str, &[u8]); 6] = [
    ("x64", include_bytes!("../../../../res/md5/bootx64.efi")),
    ("ia32", include_bytes!("../../../../res/md5/bootia32.efi")),
    ("aa64", include_bytes!("../../../../res/md5/bootaa64.efi")),
    ("arm", include_bytes!("../../../../res/md5/bootarm.efi")),
    (
        "riscv64",
        include_bytes!("../../../../res/md5/bootriscv64.efi"),
    ),
    (
        "loongarch64",
        include_bytes!("../../../../res/md5/bootloongarch64.efi"),
    ),
];
macro_rules! embed {
    ($dir:literal: $($n:literal),*) => { [$(($n, include_bytes!(concat!("../../../../res/", $dir, "/", $n)) as &[u8])),*] };
}
static FREEDOS: [(&str, &[u8]); 27] = embed!("freedos":
    "COMMAND.COM", "KERNEL.SYS", "DISPLAY.EXE", "KEYB.EXE", "MODE.COM", "KEYBOARD.SYS", "KEYBRD2.SYS",
    "KEYBRD3.SYS", "KEYBRD4.SYS", "EGA.CPX", "EGA2.CPX", "EGA3.CPX", "EGA4.CPX", "EGA5.CPX", "EGA6.CPX",
    "EGA7.CPX", "EGA8.CPX", "EGA9.CPX", "EGA10.CPX", "EGA11.CPX", "EGA12.CPX", "EGA13.CPX", "EGA14.CPX",
    "EGA15.CPX", "EGA16.CPX", "EGA17.CPX", "EGA18.CPX"
);

fn e(err: impl std::fmt::Display) -> String {
    err.to_string()
}

pub fn analyze(path: &Path) -> Result<Report, String> {
    let mut head = [0u8; 512];
    File::open(path)
        .and_then(|mut f| f.read_exact(&mut head))
        .map_err(e)?;
    let hybrid = head[510..] == [0x55, 0xaa] && (0..4).any(|i| head[446 + i * 16 + 4] != 0);
    let Ok(iso) = iso::Iso::open(path) else {
        return Ok(Report {
            hybrid,
            ..Default::default()
        });
    };
    let entries = iso.entries().map_err(e)?;
    let et = iso.el_torito();
    let lower: Vec<String> = entries
        .iter()
        .map(|x| x.path.to_ascii_lowercase())
        .collect();
    let has = |p: &str| lower.iter().any(|x| x == p);
    let efi_loader = |arch: &str| has(&format!("efi/boot/boot{arch}.efi"));
    let grub2_version = entries
        .iter()
        .find(|x| x.path.eq_ignore_ascii_case("boot/grub/i386-pc/normal.mod"))
        .and_then(|x| {
            let mut buf = Vec::new();
            iso.reader(x).ok()?.read_to_end(&mut buf).ok()?;
            grub_version(&buf)
        })
        .unwrap_or_default();
    let windows = has("sources/install.wim") || has("sources/install.esd") || has("bootmgr");
    // Syslinux is never installed for Windows images; extra_files only replaces c32 for v4.
    let sl = if windows {
        None
    } else {
        syslinux::detect(&iso)
    };
    let old_c32 = sl
        .as_ref()
        .filter(|sl| sl.major < 5)
        .map(|sl| sl.old_c32.clone())
        .unwrap_or_default();
    Ok(Report {
        grub2_version,
        old_c32,
        syslinux: sl,
        is_iso: true,
        label: iso.volume_id(),
        windows,
        efi: et.efi
            || lower
                .iter()
                .any(|p| p.starts_with("efi/boot/boot") && p.ends_with(".efi")),
        bios: et.bios,
        grub2: lower.iter().any(|p| p.starts_with("boot/grub/i386-pc/")),
        large_file: entries.iter().any(|x| x.size > FAT_MAX_FILE),
        hybrid,
        casper: has("casper"),
        debian_live: has("live"),
        size: entries.iter().map(|x| x.size).sum(),
        arch: if efi_loader("aa64") && !efi_loader("x64") {
            "arm64"
        } else if efi_loader("ia32") && !efi_loader("x64") {
            "x86"
        } else {
            "amd64"
        },
    })
}

/// Whether BIOS boot of this image would need a GRUB version Rustfus doesn't embed.
pub fn needs_grub_download(rep: &Report, scheme: Scheme, target: Target) -> bool {
    rep.grub2
        && !rep.windows
        && scheme == Scheme::Mbr
        && target != Target::Uefi
        && !rep.grub2_version.is_empty()
        && rep.grub2_version != GRUB2_VERSION
}

/// Obsolete Syslinux menu modules a write with these settings would offer to replace:
/// Syslinux is only installed on FAT, MBR, BIOS-capable targets (see `write_fs`).
pub fn obsolete_c32(rep: &Report, fs: Fs, scheme: Scheme, target: Target) -> &[String] {
    if fs.is_fat() && scheme == Scheme::Mbr && target != Target::Uefi {
        &rep.old_c32
    } else {
        &[]
    }
}

/// The image's Syslinux when a write with these settings would install it and
/// need ldlinux.sys/ldlinux.bss downloaded (Rufus' "Download required" prompt).
pub fn syslinux_download(
    rep: &Report,
    fs: Fs,
    scheme: Scheme,
    target: Target,
) -> Option<&syslinux::Info> {
    let sl = rep
        .syslinux
        .as_ref()
        .filter(|sl| syslinux::needs_download(sl))?;
    let installed = format::syslinux_for(rep, fs, scheme, target, Some(sl.clone()));
    matches!(installed, Ok(Some(_))).then_some(sl)
}

/// GRUB embeds "GNU GRUB  version X" in its modules, as Rufus' scan relies on.
/// Newer builds store the printf format "GNU GRUB  version %s\0" instead, with
/// the version as the next string, which is where Rufus reads it from.
fn grub_version(module: &[u8]) -> Option<String> {
    let tag = b"GNU GRUB  version ";
    let token = |at: usize| {
        let end = module[at..]
            .iter()
            .position(|&b| b == 0 || b.is_ascii_whitespace())?;
        Some((&module[at..at + end], at + end))
    };
    let mut from = 0;
    while let Some(p) = module[from..].windows(tag.len()).position(|w| w == tag) {
        let at = from + p + tag.len();
        let (mut v, end) = token(at)?;
        if v == b"%s" {
            v = token(end + 1)?.0;
        }
        if v.first().is_some_and(u8::is_ascii_digit) {
            return String::from_utf8(v.to_vec()).ok();
        }
        from = at;
    }
    None
}

/// Keeps labels within the filesystem's limit and free of shell
/// metacharacters, since they are spliced into OS tool command lines.
pub fn sanitize_label(s: &str, fs: Fs) -> String {
    let l: String = s
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '-'))
        .take(fs.max_label())
        .collect();
    let l = if fs.is_fat() {
        l.to_ascii_uppercase()
    } else {
        l
    };
    match l.trim() {
        "" => "USB".into(),
        t => t.to_string(),
    }
}

fn raw_path(d: &Drive) -> String {
    if cfg!(target_os = "macos") {
        d.path.replacen("/dev/disk", "/dev/rdisk", 1)
    } else {
        d.path.clone()
    }
}

fn open_dev(d: &Drive) -> Result<File, String> {
    let dev = raw_path(d);
    let mut opts = OpenOptions::new();
    opts.read(true).write(true);
    // O_EXCL on a block device makes the kernel refuse the open while any
    // partition is still mounted or held by swap, LVM or RAID.
    #[cfg(target_os = "linux")]
    std::os::unix::fs::OpenOptionsExt::custom_flags(&mut opts, libc::O_EXCL);
    opts.open(&dev).map_err(|err| {
        #[cfg(target_os = "linux")]
        if err.raw_os_error() == Some(libc::EBUSY) {
            return format!("{dev} is in use (mounted, swap, LVM or RAID); release it and retry");
        }
        format!("Cannot open {dev}: {err} (run as root/Administrator)")
    })
}

/// Releases every mounted volume on the drive so raw writes are allowed.
pub fn unmount(d: &Drive) -> Result<(), String> {
    // Every write path starts here, so this is the last moment to catch a swapped drive.
    crate::drives::verify(d)?;
    #[cfg(target_os = "linux")]
    {
        let out = run("lsblk", &["-ln", "-o", "PATH,MOUNTPOINT", &d.path])?;
        for line in out.lines() {
            if let Some((part, mnt)) = line.split_once(' ')
                && !mnt.trim().is_empty()
            {
                run("umount", &[part])?;
            }
        }
    }
    #[cfg(target_os = "macos")]
    run("diskutil", &["unmountDisk", "force", &d.path])?;
    #[cfg(windows)]
    // Windows refuses raw writes over mounted volumes; dropping the partition
    // table is the only reliable way to release them all.
    fsos::powershell(&format!(
        "Clear-Disk -Number {} -RemoveData -RemoveOEM -Confirm:$false -ErrorAction SilentlyContinue",
        d.path.trim_start_matches(r"\\.\PhysicalDrive")
    ))?;
    Ok(())
}

struct Ctx<'a> {
    tx: &'a Sender<Msg>,
    cancel: &'a AtomicBool,
}

impl Ctx<'_> {
    fn progress(&self, p: f32, s: impl Into<String>) {
        self.tx.send(Msg::Progress(p, s.into())).ok();
    }
    fn log(&self, s: impl Into<String>) {
        self.tx.send(Msg::Log(s.into())).ok();
    }
    fn check(&self) -> Result<(), String> {
        if self.cancel.load(Ordering::Relaxed) {
            Err("Cancelled".into())
        } else {
            Ok(())
        }
    }
}

pub fn run_job(job: &Job, tx: &Sender<Msg>, cancel: &AtomicBool) -> Result<String, String> {
    let ctx = Ctx { tx, cancel };
    ctx.log(format!("Target: {}", job.drive.label()));
    if let Some(passes) = job.bad_blocks {
        unmount(&job.drive)?;
        let report = image::bad_blocks(
            &raw_path(&job.drive),
            job.drive.size,
            usize::from(passes),
            &|p, s| ctx.progress(p, s),
            cancel,
        )
        .map_err(e)?;
        let msg = format!(
            "Bad blocks check: {} bad sectors ({} read, {} write, {} corruption errors)",
            report.bad_sectors, report.read_errors, report.write_errors, report.corruption_errors
        );
        ctx.log(&msg);
        if report.bad_sectors > 0 {
            return Err(msg);
        }
    }
    match &job.boot {
        Boot::Image(img) if job.dd => write_dd(img, &job.drive, job.verify, &ctx),
        Boot::Image(img) if job.wtg.is_some() => write_wtg(job, img, &ctx),
        _ => write_fs(job, &ctx),
    }
}

/// Unmounts the drive, then dumps it to `out` (.img or fixed .vhd).
pub fn save_drive(
    d: &Drive,
    out: &Path,
    tx: &Sender<Msg>,
    cancel: &AtomicBool,
) -> Result<String, String> {
    unmount(d)?;
    let ctx = Ctx { tx, cancel };
    image::save_image(
        &raw_path(d),
        d.size,
        out,
        &|p| ctx.progress(p, format!("Saving image: {:.1}%", p * 100.0)),
        cancel,
    )
    .map_err(e)?;
    Ok(format!("Saved {} to {}", d.path, out.display()))
}
