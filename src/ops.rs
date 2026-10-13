// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
use crate::blockdev::{BlockDev, read_at, write_at};
use crate::drives::run;
use crate::drives::{Drive, human};
#[cfg(windows)]
use crate::wtg;
use crate::{bootrec, fsos, image, iso, part, syslinux, wue};
use md5::{Digest, Md5};
use std::fs::{File, OpenOptions};
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

pub enum Msg {
    Progress(f32, String),
    Log(String),
    Done(Result<String, String>),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Scheme {
    Mbr,
    Gpt,
}

#[derive(Clone, Copy, PartialEq, Debug)]
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
    pub const ALL: [Fs; 9] = [Fs::Fat, Fs::Fat32, Fs::Ntfs, Fs::Udf, Fs::Exfat, Fs::Refs, Fs::Ext2, Fs::Ext3, Fs::Ext4];

    pub fn name(self) -> &'static str {
        ["FAT", "FAT32", "NTFS", "UDF", "exFAT", "ReFS", "ext2", "ext3", "ext4"][self as usize]
    }

    pub fn is_fat(self) -> bool {
        matches!(self, Fs::Fat | Fs::Fat32)
    }

    fn mbr_type(self) -> u8 {
        match self {
            Fs::Fat => 0x0E,
            Fs::Fat32 => 0x0C,
            Fs::Ext2 | Fs::Ext3 | Fs::Ext4 => 0x83,
            _ => 0x07,
        }
    }

    /// Cluster sizes offered in the UI; 0 is "(Default)".
    pub fn clusters(self) -> Vec<u32> {
        let range = |lo: u32, hi: u32| std::iter::once(0).chain((lo.ilog2()..=hi.ilog2()).map(|s| 1 << s)).collect();
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
            Fs::Fat | Fs::Fat32 => 11,
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
}

const FAT_MAX_FILE: u64 = (4 << 30) - 1;
const MB: u64 = 1 << 20;
const CHUNK: usize = 4 << 20;
const SECTOR: usize = 512;
static GRUB2_CORE: &[u8] = include_bytes!("../res/grub2/core.img");
const GRUB2_VERSION: &str = "2.14";
static UEFI_NTFS: &[u8] = include_bytes!("../res/uefi/uefi-ntfs.img");
static AUTORUN_ICO: &[u8] = include_bytes!("../res/rustfus.ico");
static MD5_LOADERS: [(&str, &[u8]); 6] = [
    ("x64", include_bytes!("../res/md5/bootx64.efi")),
    ("ia32", include_bytes!("../res/md5/bootia32.efi")),
    ("aa64", include_bytes!("../res/md5/bootaa64.efi")),
    ("arm", include_bytes!("../res/md5/bootarm.efi")),
    ("riscv64", include_bytes!("../res/md5/bootriscv64.efi")),
    ("loongarch64", include_bytes!("../res/md5/bootloongarch64.efi")),
];
macro_rules! embed {
    ($dir:literal: $($n:literal),*) => { [$(($n, include_bytes!(concat!("../res/", $dir, "/", $n)) as &[u8])),*] };
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
    File::open(path).and_then(|mut f| f.read_exact(&mut head)).map_err(e)?;
    let hybrid = head[510..] == [0x55, 0xAA] && (0..4).any(|i| head[446 + i * 16 + 4] != 0);
    let Ok(iso) = iso::Iso::open(path) else {
        return Ok(Report { hybrid, ..Default::default() });
    };
    let entries = iso.entries().map_err(e)?;
    let et = iso.el_torito();
    let lower: Vec<String> = entries.iter().map(|x| x.path.to_ascii_lowercase()).collect();
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
    Ok(Report {
        grub2_version,
        is_iso: true,
        label: iso.volume_id(),
        windows: has("sources/install.wim") || has("sources/install.esd") || has("bootmgr"),
        efi: et.efi || lower.iter().any(|p| p.starts_with("efi/boot/boot") && p.ends_with(".efi")),
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
    rep.grub2 && !rep.windows && scheme == Scheme::Mbr && target != Target::Uefi
        && !rep.grub2_version.is_empty() && rep.grub2_version != GRUB2_VERSION
}

/// core.img matching the image's GRUB, fetched (signed) from rufus.ie like
/// Rufus does, trimming the version at '.'/'-' until the server has it.
fn grub_core(rep: &Report, allow: bool, ctx: &Ctx) -> std::borrow::Cow<'static, [u8]> {
    let v = &rep.grub2_version;
    if v.is_empty() || v == GRUB2_VERSION {
        return GRUB2_CORE.into();
    }
    if allow {
        let cuts = (1..=v.len()).rev().filter(|&i| i == v.len() || matches!(v.as_bytes()[i], b'.' | b'-'));
        for i in cuts {
            let ver = &v[..i];
            let url = format!("{}/grub-{ver}/core.img", crate::net::FILES_URL);
            match crate::net::fetch_signed(&url, &format!("grub-{ver}/core.img")) {
                Ok(data) => {
                    ctx.log(format!("Using GRUB {ver} core.img from {url}"));
                    return data.into();
                }
                Err(err) => ctx.log(format!("{url}: {err}")),
            }
        }
    }
    ctx.log(format!("WARNING: image uses GRUB {v} but the embedded core.img is {GRUB2_VERSION}; BIOS boot may fail"));
    GRUB2_CORE.into()
}

/// GRUB embeds "GNU GRUB  version X" in its modules, as Rufus' scan relies on.
fn grub_version(module: &[u8]) -> Option<String> {
    let tag = b"GNU GRUB  version ";
    let at = module.windows(tag.len()).position(|w| w == tag)? + tag.len();
    let end = module[at..].iter().position(|&b| b == 0 || b.is_ascii_whitespace())?;
    String::from_utf8(module[at..at + end].to_vec()).ok()
}

/// Keeps labels within the filesystem's limit and free of shell
/// metacharacters, since they are spliced into OS tool command lines.
pub fn sanitize_label(s: &str, fs: Fs) -> String {
    let l: String = s
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '-'))
        .take(fs.max_label())
        .collect();
    let l = if fs.is_fat() { l.to_ascii_uppercase() } else { l };
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
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(&dev)
        .map_err(|err| format!("Cannot open {dev}: {err} (run as root/Administrator)"))
}

/// Releases every mounted volume on the drive so raw writes are allowed.
pub fn unmount(d: &Drive) -> Result<(), String> {
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

/// Without this, Linux verification would read back the page cache.
fn drop_cache(_f: &File) {
    #[cfg(target_os = "linux")]
    unsafe {
        use std::os::fd::AsRawFd;
        libc::posix_fadvise(_f.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED);
    }
}

fn read_full(r: &mut dyn Read, buf: &mut [u8]) -> Result<usize, String> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..]).map_err(e)? {
            0 => break,
            k => n += k,
        }
    }
    Ok(n)
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
        if self.cancel.load(Ordering::Relaxed) { Err("Cancelled".into()) } else { Ok(()) }
    }
}

pub fn run_job(job: &Job, tx: &Sender<Msg>, cancel: &AtomicBool) -> Result<String, String> {
    let ctx = Ctx { tx, cancel };
    ctx.log(format!("Target: {}", job.drive.label()));
    if let Some(passes) = job.bad_blocks {
        unmount(&job.drive)?;
        let report = image::bad_blocks(&raw_path(&job.drive), job.drive.size, usize::from(passes), &|p, s| ctx.progress(p, s), cancel)
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
pub fn save_drive(d: &Drive, out: &Path, tx: &Sender<Msg>, cancel: &AtomicBool) -> Result<String, String> {
    unmount(d)?;
    let ctx = Ctx { tx, cancel };
    image::save_image(&raw_path(d), d.size, out, &|p| ctx.progress(p, format!("Saving image: {:.1}%", p * 100.0)), cancel)
        .map_err(e)?;
    Ok(format!("Saved {} to {}", d.path, out.display()))
}

/// DD-mode write of `img` (decompressed on the fly), optionally verified.
fn write_dd(img: &Path, d: &Drive, verify: bool, ctx: &Ctx) -> Result<String, String> {
    if image::is_ffu(img) {
        ctx.progress(0.0, "Unmounting...");
        unmount(d)?;
        ctx.progress(0.5, "Applying FFU image with DISM...");
        image::apply_ffu(img, &raw_path(d)).map_err(e)?;
        return Ok(format!("Applied {} to {}", img.display(), d.path));
    }
    let src = image::open(img).map_err(e)?;
    if let Some(size) = src.size.filter(|&s| s > d.size) {
        return Err(format!("Image ({}) is larger than the drive ({})", human(size), human(d.size)));
    }
    ctx.progress(0.0, "Unmounting...");
    unmount(d)?;
    let dev = open_dev(d)?;
    let written = copy(src, &dev, d.size, ctx)?;
    if verify {
        drop_cache(&dev);
        verify_dev(image::open(img).map_err(e)?, &dev, written, ctx)?;
    }
    Ok(format!("Wrote {} to {}", human(written), d.path))
}

fn copy(mut src: image::Source, dev: &File, limit: u64, ctx: &Ctx) -> Result<u64, String> {
    let mut buf = vec![0u8; CHUNK];
    let mut done = 0u64;
    loop {
        ctx.check()?;
        let n = read_full(&mut src.reader, &mut buf)?;
        if n == 0 {
            break;
        }
        // Raw devices need sector-multiple writes; pad the final chunk.
        let len = n.next_multiple_of(SECTOR);
        if done + len as u64 > limit {
            return Err("Image is larger than the drive".into());
        }
        buf[n..len].fill(0);
        write_at(dev, &buf[..len], done).map_err(e)?;
        done += n as u64;
        let pct = src.consumed.load(Ordering::Relaxed) as f32 / src.total.max(1) as f32;
        ctx.progress(pct, format!("Writing image: {:.1}%", pct * 100.0));
    }
    ctx.progress(1.0, "Flushing...");
    dev.sync_all().map_err(e)?;
    Ok(done)
}

fn verify_dev(mut src: image::Source, dev: &File, total: u64, ctx: &Ctx) -> Result<(), String> {
    let (mut buf, mut back) = (vec![0u8; CHUNK], vec![0u8; CHUNK]);
    let mut checked = 0u64;
    while checked < total {
        ctx.check()?;
        let n = read_full(&mut src.reader, &mut buf)?;
        if n == 0 {
            break;
        }
        let len = n.next_multiple_of(SECTOR);
        read_at(dev, &mut back[..len], checked).map_err(e)?;
        if buf[..n] != back[..n] {
            return Err(format!("Verification failed near byte {checked}"));
        }
        checked += n as u64;
        let pct = checked as f32 / total as f32;
        ctx.progress(pct, format!("Verifying: {:.1}%", pct * 100.0));
    }
    Ok(())
}

fn layout(job: &Job, uefi_ntfs: bool) -> Vec<part::Part> {
    let gpt = job.scheme == Scheme::Gpt;
    let start = if job.old_bios && !gpt { 63 * part::SECTOR } else { MB };
    let mut end = (job.drive.size - if gpt { 34 * part::SECTOR } else { 0 }) / MB * MB;
    let mut extra = Vec::new();
    if job.persistence > 0 {
        let size = job.persistence / MB * MB;
        end -= size;
        extra.push(part::Part { start: end, size, mbr_type: 0x83, gpt_type: part::GPT_LINUX, name: "persistence", active: false });
    }
    if uefi_ntfs {
        let size = (UEFI_NTFS.len() as u64).next_multiple_of(MB);
        end -= size;
        for p in &mut extra {
            p.start -= size;
        }
        extra.push(part::Part { start: end + job.persistence / MB * MB, size, mbr_type: 0xEF, gpt_type: part::GPT_ESP, name: "UEFI:NTFS", active: false });
        if job.persistence == 0 {
            extra.last_mut().unwrap().start = end;
        }
    }
    let bootable = job.boot != Boot::NonBootable && job.target != Target::Uefi;
    let main = part::Part {
        start,
        size: end - start,
        mbr_type: job.fs.mbr_type(),
        gpt_type: part::GPT_BASIC_DATA,
        name: "Main Data Partition",
        active: bootable && !gpt,
    };
    std::iter::once(main).chain(extra).collect()
}

fn mbr_kind(job: &Job, rep: &Report, sl: Option<&syslinux::Info>) -> bootrec::Mbr {
    if job.boot == Boot::NonBootable || job.target == Target::Uefi {
        return bootrec::Mbr::Zero;
    }
    if rep.windows {
        return bootrec::Mbr::Win7;
    }
    // Rufus checks Syslinux before GRUB.
    if sl.is_some() {
        return bootrec::Mbr::Syslinux;
    }
    if rep.grub2 {
        return bootrec::Mbr::Grub2;
    }
    bootrec::Mbr::Rufus { bios_id: None }
}

/// Where files land: Rustfus' own FAT driver, or a mounted OS filesystem.
enum Dest<'a, T: fatfs::ReadWriteSeek> {
    Fat(&'a fatfs::FileSystem<T>),
    Dir(PathBuf),
}

/// `Dest::Dir` never touches the FAT type parameter.
type DirDest = Dest<'static, Cursor<Vec<u8>>>;

impl<T: fatfs::ReadWriteSeek> Dest<'_, T> {
    fn mkdir(&self, p: &str) -> io::Result<()> {
        match self {
            Dest::Fat(fs) => fs.root_dir().create_dir(p).map(|_| ()),
            Dest::Dir(root) => std::fs::create_dir_all(root.join(p)),
        }
    }

    fn create(&self, p: &str) -> io::Result<Box<dyn Write + '_>> {
        match self {
            Dest::Fat(fs) => {
                let mut f = fs.root_dir().create_file(p)?;
                f.truncate()?;
                Ok(Box::new(f))
            }
            Dest::Dir(root) => Ok(Box::new(File::create(root.join(p))?)),
        }
    }

    fn put(&self, p: &str, data: &[u8]) -> io::Result<()> {
        self.create(p)?.write_all(data)
    }
}

fn is_boot_cfg(p: &str) -> bool {
    let p = p.to_ascii_lowercase();
    p.ends_with(".cfg") && (p.starts_with("boot/grub") || p.starts_with("isolinux") || p.starts_with("syslinux"))
}

/// Adds the persistence kernel option, as Rufus does for casper/live ISOs.
fn patch_persistence(cfg: &str) -> String {
    cfg.replace("boot=casper persistent", "boot=casper")
        .replace("boot=live persistence", "boot=live")
        .replace("boot=casper", "boot=casper persistent")
        .replace("boot=live", "boot=live persistence")
}

/// Copies the ISO tree plus Rufus' extras (validation loaders, unattend,
/// FreeDOS, autorun) to `dest`.
fn copy_files<T: fatfs::ReadWriteSeek>(job: &Job, rep: &Report, src: Option<&iso::Iso>, dest: &Dest<T>, ctx: &Ctx) -> Result<(), String> {
    let entries = match src {
        Some(iso) => iso.entries().map_err(e)?,
        None => Vec::new(),
    };
    let total = entries.iter().map(|x| x.size).sum::<u64>().max(1);
    let (mut done, mut md5sums, mut loaders) = (0u64, String::new(), Vec::new());
    let mut buf = vec![0u8; 1 << 20];
    for entry in &entries {
        ctx.check()?;
        if entry.is_dir {
            dest.mkdir(&entry.path).map_err(|err| format!("{}: {err}", entry.path))?;
            continue;
        }
        if let Some(target) = entry.symlink() {
            ctx.log(format!("Skipping symlink {} -> {target}", entry.path));
            continue;
        }
        let iso = src.expect("entries come from an ISO");
        let mut path = entry.path.clone();
        let lower = path.to_ascii_lowercase();
        if job.uefi_validation
            && let Some((_, loader)) = MD5_LOADERS.iter().find(|(a, _)| lower == format!("efi/boot/boot{a}.efi"))
        {
            loaders.push((path.clone(), *loader));
            path = format!("{}_original.efi", &path[..path.len() - 4]);
        }
        let mut reader: Box<dyn Read> = Box::new(iso.reader(entry).map_err(e)?);
        if job.persistence > 0 && is_boot_cfg(&path) && entry.size < MB {
            let mut s = String::new();
            if reader.read_to_string(&mut s).is_ok() {
                ctx.log(format!("Patched {path} for persistence"));
                reader = Box::new(Cursor::new(patch_persistence(&s).into_bytes()));
            } else {
                reader = Box::new(iso.reader(entry).map_err(e)?);
            }
        }
        let mut out = dest.create(&path).map_err(|err| format!("{path}: {err}"))?;
        let mut md5 = Md5::new();
        loop {
            let n = reader.read(&mut buf).map_err(e)?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n]).map_err(|err| format!("{path}: {err}"))?;
            md5.update(&buf[..n]);
            done += n as u64;
            let pct = done as f32 / total as f32;
            ctx.progress(pct, format!("Copying ISO files: {:.1}%", pct * 100.0));
        }
        out.flush().map_err(e)?;
        md5sums += &format!("{}  ./{path}\n", image::hex(&md5.finalize()));
    }

    let has = |p: &str| entries.iter().any(|x| x.path.eq_ignore_ascii_case(p));
    for (path, loader) in loaders {
        dest.put(&path, loader).map_err(e)?;
    }
    if job.uefi_validation && src.is_some() {
        let header = format!("# md5_totalbytes = 0x{done:x}\n");
        dest.put("md5sum.txt", (header + &md5sums).as_bytes()).map_err(e)?;
    }
    if rep.windows && job.wue.any() {
        ctx.log("Writing autounattend.xml");
        dest.put("autounattend.xml", wue::unattend_xml(&job.wue, rep.arch).as_bytes()).map_err(e)?;
    }
    if job.boot == Boot::FreeDos {
        dest.mkdir("LOCALE").map_err(e)?;
        for (name, data) in FREEDOS.iter() {
            let p = if matches!(*name, "COMMAND.COM" | "KERNEL.SYS") { name.to_string() } else { format!("LOCALE/{name}") };
            dest.put(&p, data).map_err(e)?;
        }
        dest.put("AUTOEXEC.BAT", b"@echo off\r\nSET PATH=\\LOCALE\r\n").map_err(e)?;
    }
    if job.ext_label && !has("autorun.inf") {
        let inf = format!("; Created by Rustfus\r\n[autorun]\r\nicon  = autorun.ico\r\nlabel = {}\r\n", job.label);
        dest.put("autorun.inf", inf.as_bytes()).map_err(e)?;
        dest.put("autorun.ico", AUTORUN_ICO).map_err(e)?;
    }
    Ok(())
}

fn ext_kind(fs: Fs) -> Option<crate::ext::Kind> {
    match fs {
        Fs::Ext2 => Some(crate::ext::Kind::Ext2),
        Fs::Ext3 => Some(crate::ext::Kind::Ext3),
        Fs::Ext4 => Some(crate::ext::Kind::Ext4),
        _ => None,
    }
}

fn fat_label(label: &str) -> [u8; 11] {
    let mut l = [b' '; 11];
    for (d, s) in l.iter_mut().zip(label.bytes()) {
        *d = s;
    }
    l
}

fn zero_range(dev: &File, start: u64, len: u64, ctx: Option<&Ctx>) -> Result<(), String> {
    let buf = vec![0u8; CHUNK];
    let mut off = 0;
    while off < len {
        if let Some(c) = ctx {
            c.check()?;
            let pct = off as f32 / len as f32;
            c.progress(pct, format!("Zeroing drive: {:.1}%", pct * 100.0));
        }
        let n = (len - off).min(CHUNK as u64) as usize;
        write_at(dev, &buf[..n], start + off).map_err(e)?;
        off += n as u64;
    }
    Ok(())
}

/// Partition table, MBR boot code and the raw UEFI:NTFS image.
fn write_tables(job: &Job, rep: &Report, sl: Option<&syslinux::Info>, parts: &[part::Part], dev: &File, ctx: &Ctx) -> Result<(), String> {
    let mut disk = BlockDev::new(dev, 0, job.drive.size / part::SECTOR * part::SECTOR);
    match job.scheme {
        Scheme::Mbr => part::write_mbr(&mut disk, job.drive.size, parts),
        Scheme::Gpt => part::write_gpt(&mut disk, job.drive.size, parts),
    }
    .map_err(e)?;
    if job.scheme == Scheme::Mbr {
        let kind = mbr_kind(job, rep, sl);
        bootrec::write_mbr(&mut disk, kind).map_err(e)?;
        // GRUB 2 core.img goes in the gap between the MBR and the first partition.
        if kind == bootrec::Mbr::Grub2 {
            let core = grub_core(rep, job.allow_downloads, ctx);
            if part::SECTOR + core.len() as u64 > parts[0].start {
                ctx.log("WARNING: no room for GRUB core.img; uncheck 'Add fixes for old BIOSes'");
            } else {
                disk.seek(SeekFrom::Start(part::SECTOR)).map_err(e)?;
                disk.write_all(&core).map_err(e)?;
            }
        }
    } else if job.boot != Boot::NonBootable {
        // Like Rufus: BIOS boot of GPT media prints why it can't work.
        bootrec::write_mbr(&mut disk, bootrec::Mbr::RufusMsg).map_err(e)?;
        let mut msg = bootrec::MSG_TEXT.to_vec();
        msg.resize(4096, 0);
        disk.seek(SeekFrom::Start(34 * part::SECTOR)).map_err(e)?;
        disk.write_all(&msg).map_err(e)?;
    }
    if let Some(p) = parts.iter().find(|p| p.name == "UEFI:NTFS") {
        ctx.log("Writing UEFI:NTFS partition");
        disk.seek(SeekFrom::Start(p.start)).map_err(e)?;
        disk.write_all(UEFI_NTFS).map_err(e)?;
    }
    disk.flush().map_err(e)
}

/// FAT/FAT32 path: Rustfus formats and fills the partition itself.
fn write_fat(job: &Job, rep: &Report, iso: Option<&iso::Iso>, sl: Option<&syslinux::Info>, main: &part::Part, dev: &File, ctx: &Ctx) -> Result<(), String> {
    ctx.progress(0.0, format!("Formatting ({})...", job.fs.name()));
    let mut vol = BlockDev::new(dev, main.start, main.size);
    let label = fat_label(&sanitize_label(&job.label, job.fs));
    let hidden = (main.start / part::SECTOR) as u32;
    crate::fat::format(&mut vol, main.size, job.fs == Fs::Fat32, job.cluster, label, hidden)
        .map_err(|err| format!("Format failed: {err}"))?;
    {
        let fat = fatfs::FileSystem::new(&mut vol, fatfs::FsOptions::new()).map_err(e)?;
        let dest = Dest::Fat(&fat);
        copy_files(job, rep, iso, &dest, ctx)?;
        if let (Some(iso), Some(sl)) = (iso, sl) {
            for (path, data) in syslinux::extra_files(iso, sl, &|m| ctx.log(m))? {
                dest.put(&path, &data).map_err(e)?;
            }
        }
        drop(dest);
        fat.unmount().map_err(e)?;
    }
    crate::fat::fix_dot_entries(&mut vol).map_err(e)?;
    vol.seek(SeekFrom::Start(0)).map_err(e)?;
    if let Some(sl) = sl {
        ctx.progress(1.0, "Installing Syslinux...");
        syslinux::install(&mut vol, sl, &|m| ctx.log(m))?;
    } else if job.fs == Fs::Fat32 && job.target != Target::Uefi {
        if job.boot == Boot::FreeDos {
            bootrec::write_fat32_freedos_pbr(&mut vol).map_err(e)?;
        } else if rep.windows {
            bootrec::write_fat32_pe_pbr(&mut vol).map_err(e)?;
        }
    }
    vol.flush().map_err(e)
}

#[cfg(not(windows))]
fn write_wtg(_job: &Job, _img: &Path, _ctx: &Ctx) -> Result<String, String> {
    Err("Windows To Go needs DISM and bcdboot, which only exist on Windows".into())
}

/// Windows To Go, like Rufus: ESP (GPT) + NTFS, DISM applies the chosen
/// edition from the mounted ISO, bcdboot makes it bootable.
#[cfg(windows)]
fn write_wtg(job: &Job, img: &Path, ctx: &Ctx) -> Result<String, String> {
    let index = job.wtg.unwrap();
    let gpt = job.scheme == Scheme::Gpt;
    let end = (job.drive.size - 34 * part::SECTOR) / MB * MB;
    let esp = 260 * MB;
    let mut parts = Vec::new();
    if gpt {
        parts.push(part::Part { start: MB, size: esp, mbr_type: 0xEF, gpt_type: part::GPT_ESP, name: "EFI System Partition", active: false });
    }
    let start = if gpt { MB + esp } else { MB };
    parts.push(part::Part { start, size: end - start, mbr_type: 0x07, gpt_type: part::GPT_BASIC_DATA, name: "Windows", active: !gpt });

    ctx.progress(0.0, "Unmounting...");
    unmount(&job.drive)?;
    let dev = open_dev(&job.drive)?;
    for p in &parts {
        zero_range(&dev, p.start, p.size.min(MB), None)?;
    }
    {
        let mut disk = BlockDev::new(&dev, 0, job.drive.size / part::SECTOR * part::SECTOR);
        if gpt { part::write_gpt(&mut disk, job.drive.size, &parts) } else { part::write_mbr(&mut disk, job.drive.size, &parts) }.map_err(e)?;
        if !gpt {
            bootrec::write_mbr(&mut disk, bootrec::Mbr::Win7).map_err(e)?;
        }
        disk.flush().map_err(e)?;
    }
    if gpt {
        let mut vol = BlockDev::new(&dev, MB, esp);
        crate::fat::format(&mut vol, esp, true, 0, fat_label("ESP"), (MB / part::SECTOR) as u32).map_err(e)?;
        vol.flush().map_err(e)?;
    }
    drop(dev);
    fsos::rescan(&job.drive)?;
    let n = parts.len() as u32;
    ctx.progress(0.0, "Formatting (NTFS)...");
    fsos::mkfs(&job.drive, n, Fs::Ntfs, &sanitize_label(&job.label, Fs::Ntfs), job.cluster)?;
    let win = fsos::mount(&job.drive, n, Fs::Ntfs)?;
    let sys = if gpt { fsos::mount(&job.drive, 1, Fs::Fat32)? } else { win.clone() };

    let iso_path = img.to_string_lossy().replace('\'', "''");
    let letter = fsos::powershell(&format!("(Mount-DiskImage -ImagePath '{iso_path}' -PassThru | Get-Volume).DriveLetter"))?;
    let letter = letter.trim().chars().next().ok_or("could not mount the ISO")?;
    let result = (|| {
        let src = ["install.wim", "install.esd"]
            .iter()
            .map(|f| format!(r"{letter}:\sources\{f}"))
            .find(|p| Path::new(p).exists())
            .ok_or("no install.wim/esd in the image")?;
        let win_s = win.to_string_lossy().trim_end_matches('\\').to_string();
        ctx.progress(0.5, "Applying Windows image (this can take a long time)...");
        ctx.log(format!("dism /Apply-Image /ImageFile:{src} /Index:{index} /ApplyDir:{win_s}\\"));
        run("dism", &["/Apply-Image", &format!("/ImageFile:{src}"), &format!("/Index:{index}"), &format!(r"/ApplyDir:{win_s}\")])?;
        if job.wue.wtg_no_internal {
            let xml = wtg::SAN_POLICY_XML.replace("ARCH", "amd64");
            std::fs::write(win.join("san_policy.xml"), xml).map_err(e)?;
            run("dism", &[&format!(r"/Image:{win_s}\"), &format!(r"/Apply-Unattend:{win_s}\san_policy.xml")])?;
        }
        if job.wue.any() {
            let panther = win.join("Windows").join("Panther");
            std::fs::create_dir_all(&panther).map_err(e)?;
            std::fs::write(panther.join("unattend.xml"), wue::unattend_xml(&job.wue, "amd64")).map_err(e)?;
        }
        ctx.progress(0.9, "Installing boot files...");
        let sys_s = sys.to_string_lossy().trim_end_matches('\\').to_string();
        run("bcdboot", &[&format!(r"{win_s}\Windows"), "/s", &sys_s, "/f", "ALL"])?;
        Ok::<(), String>(())
    })();
    fsos::powershell(&format!("Dismount-DiskImage -ImagePath '{iso_path}'")).ok();
    result?;
    Ok(format!("Windows To Go installed on {}", job.drive.path))
}

/// ISO mode / format: partition, create the filesystem, copy files, install boot code.
fn write_fs(job: &Job, ctx: &Ctx) -> Result<String, String> {
    let (iso, rep) = match &job.boot {
        Boot::Image(p) => (Some(iso::Iso::open(p).map_err(e)?), analyze(p)?),
        _ => (None, Report::default()),
    };
    let fs = job.fs;
    if fs.is_fat() && rep.large_file {
        return Err("This image contains a file larger than 4 GB, which FAT32 cannot store. Use NTFS or exFAT.".into());
    }
    if !fs.supported() {
        return Err(format!("{} cannot be created on this OS", fs.name()));
    }
    let label = sanitize_label(&job.label, fs);
    // Like Rufus: Syslinux only for FAT, non-Windows, BIOS-capable targets.
    let sl = iso
        .as_ref()
        .filter(|_| fs.is_fat() && !rep.windows && job.target != Target::Uefi && job.scheme == Scheme::Mbr)
        .and_then(syslinux::detect);
    if let Some(sl) = &sl {
        ctx.log(format!("Syslinux {}{} detected", sl.version, sl.ext));
        // Fetch a non-embedded loader now so a download failure leaves the drive untouched.
        syslinux::loader(sl, &|m| ctx.log(m))?;
    }
    let parts = layout(job, !fs.is_fat() && iso.is_some() && job.target != Target::Bios);

    ctx.progress(0.0, "Unmounting...");
    unmount(&job.drive)?;
    let dev = open_dev(&job.drive)?;
    if !job.quick {
        zero_range(&dev, 0, job.drive.size / part::SECTOR * part::SECTOR, Some(ctx))?;
    }
    // Clear stale filesystem signatures at the start of every partition.
    for p in &parts {
        zero_range(&dev, p.start, p.size.min(MB), None)?;
    }
    ctx.progress(0.0, format!("Partitioning ({:?})...", job.scheme));
    write_tables(job, &rep, sl.as_ref(), &parts, &dev, ctx)?;

    let needs_files = iso.is_some() || job.ext_label || job.boot == Boot::FreeDos;
    if fs.is_fat() {
        write_fat(job, &rep, iso.as_ref(), sl.as_ref(), &parts[0], &dev, ctx)?;
    } else if let Some(kind) = ext_kind(fs) {
        if needs_files && !cfg!(target_os = "linux") {
            return Err(format!("Copying files to {} needs Linux; use FAT32/NTFS/exFAT", fs.name()));
        }
        ctx.progress(0.0, format!("Formatting ({})...", fs.name()));
        let mut vol = BlockDev::new(&dev, parts[0].start, parts[0].size);
        crate::ext::format(&mut vol, parts[0].size, kind, &label, job.cluster, &[]).map_err(|err| format!("Format failed: {err}"))?;
        vol.flush().map_err(e)?;
    }
    // Persistence is created by Rustfus' own ext formatter, so it works on every OS.
    if let Some(p) = parts.iter().find(|p| p.name == "persistence") {
        let (plabel, conf): (&str, &[(&str, &[u8])]) =
            if rep.casper { ("casper-rw", &[]) } else { ("persistence", &[("persistence.conf", b"/ union\n")]) };
        ctx.progress(0.0, "Creating persistent partition...");
        let mut vol = BlockDev::new(&dev, p.start, p.size);
        crate::ext::format(&mut vol, p.size, crate::ext::Kind::Ext3, plabel, 0, conf).map_err(e)?;
        vol.flush().map_err(e)?;
    }
    if fs.is_fat() {
        drop(dev);
        fsos::rescan(&job.drive).ok();
    } else if ext_kind(fs).is_some() {
        drop(dev);
        fsos::rescan(&job.drive)?;
        if needs_files {
            let mnt = fsos::mount(&job.drive, 1, fs)?;
            let r = copy_files(job, &rep, iso.as_ref(), &DirDest::Dir(mnt.clone()), ctx);
            fsos::unmount(&job.drive, 1, &mnt)?;
            r?;
        }
    } else {
        drop(dev);
        fsos::rescan(&job.drive)?;
        ctx.progress(0.0, format!("Formatting ({})...", fs.name()));
        fsos::mkfs(&job.drive, 1, fs, &label, job.cluster)?;
        if iso.is_some() || job.ext_label || job.boot == Boot::FreeDos {
            let mnt = fsos::mount(&job.drive, 1, fs)?;
            let r = copy_files(job, &rep, iso.as_ref(), &DirDest::Dir(mnt.clone()), ctx);
            fsos::unmount(&job.drive, 1, &mnt)?;
            r?;
        }
        if fs == Fs::Ntfs && rep.windows && job.target != Target::Uefi && !cfg!(windows) {
            // mkntfs boot code can't load BOOTMGR; Windows' own format already can.
            let dev = open_dev(&job.drive)?;
            bootrec::write_ntfs_pbr(&mut BlockDev::new(&dev, parts[0].start, parts[0].size)).map_err(e)?;
        }
    }

    Ok(format!("{} ready ({} \"{label}\")", job.drive.path, fs.name()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_is_sanitized() {
        assert_eq!(sanitize_label("my'; rm -rf", Fs::Ntfs), "my rm -rf");
        assert_eq!(sanitize_label("abcdefghijklmn", Fs::Fat32), "ABCDEFGHIJK");
        assert_eq!(sanitize_label("$()", Fs::Exfat), "USB");
    }

    #[test]
    fn grub_version_is_found() {
        assert_eq!(grub_version(b"\0\0GNU GRUB  version 2.12\0rest").as_deref(), Some("2.12"));
        assert_eq!(grub_version(b"nothing here"), None);
    }

    #[test]
    fn persistence_patch_is_idempotent() {
        let once = patch_persistence("linux /casper/vmlinuz boot=casper quiet");
        assert_eq!(once, "linux /casper/vmlinuz boot=casper persistent quiet");
        assert_eq!(patch_persistence(&once), once);
    }

    fn ctx_parts() -> (Sender<Msg>, AtomicBool) {
        let (tx, rx) = std::sync::mpsc::channel();
        std::mem::forget(rx);
        (tx, AtomicBool::new(false))
    }

    #[test]
    fn copy_pads_and_verifies() {
        let dir = std::env::temp_dir().join(format!("rustfus-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (img, devp) = (dir.join("img"), dir.join("dev"));
        let data: Vec<u8> = (0..CHUNK + 1000).map(|i| (i % 251) as u8).collect();
        std::fs::write(&img, &data).unwrap();
        std::fs::write(&devp, vec![0xffu8; CHUNK * 2]).unwrap();
        let (tx, cancel) = ctx_parts();
        let ctx = Ctx { tx: &tx, cancel: &cancel };
        let dev = OpenOptions::new().read(true).write(true).open(&devp).unwrap();
        let n = copy(image::open(&img).unwrap(), &dev, (CHUNK * 2) as u64, &ctx).unwrap();
        verify_dev(image::open(&img).unwrap(), &dev, n, &ctx).unwrap();
        let out = std::fs::read(&devp).unwrap();
        assert_eq!(&out[..data.len()], &data[..]);
        let end = data.len().next_multiple_of(SECTOR);
        assert!(out[data.len()..end].iter().all(|&b| b == 0));
        assert!(out[end..].iter().all(|&b| b == 0xff));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn layout_places_extra_partitions_at_end() {
        let drive = Drive { path: "x".into(), name: "x".into(), size: 1000 * MB + 12345 };
        let mut job = test_job(drive, PathBuf::from("x.iso"));
        job.fs = Fs::Ntfs;
        job.persistence = 100 * MB;
        let p = layout(&job, true);
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].start, MB);
        assert_eq!(p[0].start + p[0].size, p[1].start);
        assert_eq!(p[1].start + p[1].size, p[2].start);
        assert_eq!(p[2].start + p[2].size, 1000 * MB);
        assert_eq!((p[1].name, p[2].name), ("persistence", "UEFI:NTFS"));
        assert!(p[0].active);
    }

    fn test_job(drive: Drive, iso: PathBuf) -> Job {
        Job {
            drive,
            boot: Boot::Image(iso),
            dd: false,
            scheme: Scheme::Mbr,
            target: Target::BiosUefi,
            fs: Fs::Fat32,
            cluster: 0,
            label: "CCCOMA_X64".into(),
            quick: true,
            ext_label: true,
            bad_blocks: None,
            verify: false,
            old_bios: false,
            uefi_validation: true,
            persistence: 0,
            wue: wue::Options { bypass_hw_checks: true, ..Default::default() },
            allow_downloads: false,
            wtg: None,
        }
    }

    /// Linux ISO with Syslinux 4.07 (embedded loader) in ISO mode on FAT32.
    #[test]
    fn syslinux_iso_mode_to_fat32_image() {
        let dir = std::env::temp_dir().join(format!("rustfus-sl-{}", std::process::id()));
        let tree = dir.join("tree");
        std::fs::create_dir_all(tree.join("isolinux")).unwrap();
        let mut bin = vec![0u8; 4096];
        let v = b"ISOLINUX 4.07 2013-07-25\0";
        bin[1000..1000 + v.len()].copy_from_slice(v);
        std::fs::write(tree.join("isolinux/isolinux.bin"), &bin).unwrap();
        std::fs::write(tree.join("isolinux/isolinux.cfg"), "DEFAULT linux\n").unwrap();
        std::fs::write(tree.join("isolinux/vmlinuz"), vec![9u8; 100_000]).unwrap();
        let iso_path = dir.join("linux.iso");
        let ok = std::process::Command::new("xorriso")
            .args(["-as", "mkisofs", "-quiet", "-R", "-J", "-V", "LIVE", "-o"])
            .arg(&iso_path)
            .arg(&tree)
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            return;
        }
        let devp = dir.join("disk.img");
        let size = 128 * MB;
        File::create(&devp).unwrap().set_len(size).unwrap();
        let rep = analyze(&iso_path).unwrap();
        let iso = iso::Iso::open(&iso_path).unwrap();
        let sl = syslinux::detect(&iso).expect("syslinux detected");
        assert_eq!(sl.major, 4);
        let drive = Drive { path: devp.to_string_lossy().into(), name: "img".into(), size };
        let mut job = test_job(drive, iso_path);
        job.uefi_validation = false;
        let (tx, cancel) = ctx_parts();
        let ctx = Ctx { tx: &tx, cancel: &cancel };
        let parts = layout(&job, false);
        let dev = OpenOptions::new().read(true).write(true).open(&devp).unwrap();
        write_tables(&job, &rep, Some(&sl), &parts, &dev, &ctx).unwrap();
        write_fat(&job, &rep, Some(&iso), Some(&sl), &parts[0], &dev, &ctx).unwrap();

        let raw = std::fs::read(&devp).unwrap();
        assert_eq!(&raw[..16], &bootrec_mbr_syslinux()[..16], "Syslinux MBR code");
        let p = &raw[parts[0].start as usize..(parts[0].start + parts[0].size) as usize];
        let part_img = dir.join("part.img");
        std::fs::write(&part_img, p).unwrap();
        let mut f = File::open(&part_img).unwrap();
        let fat = fatfs::FileSystem::new(&mut f, fatfs::FsOptions::new()).unwrap();
        let mut cfg = String::new();
        fat.root_dir().open_file("syslinux.cfg").unwrap().read_to_string(&mut cfg).unwrap();
        assert!(cfg.contains("CONFIG /isolinux/isolinux.cfg"));
        assert!(fat.root_dir().open_file("ldlinux.sys").is_ok());
        std::fs::remove_dir_all(dir).ok();
    }

    fn bootrec_mbr_syslinux() -> Vec<u8> {
        let mut c = Cursor::new(vec![0u8; 512]);
        bootrec::write_mbr(&mut c, bootrec::Mbr::Syslinux).unwrap();
        c.into_inner()
    }

    /// ISO mode onto a disk image file: MBR + FAT32 + files + boot code.
    /// Skips the OS unmount/rescan steps, which need a real block device.
    #[test]
    fn iso_mode_to_fat32_image() {
        let dir = std::env::temp_dir().join(format!("rustfus-iso-{}", std::process::id()));
        let tree = dir.join("tree");
        std::fs::create_dir_all(tree.join("EFI/BOOT")).unwrap();
        std::fs::create_dir_all(tree.join("sources")).unwrap();
        std::fs::write(tree.join("EFI/BOOT/BOOTX64.EFI"), b"loader").unwrap();
        std::fs::write(tree.join("bootmgr"), vec![7u8; 300_000]).unwrap();
        std::fs::write(tree.join("sources/install.wim"), b"wim").unwrap();
        let iso_path = dir.join("win.iso");
        let ok = std::process::Command::new("xorriso")
            .args(["-as", "mkisofs", "-quiet", "-J", "-V", "CCCOMA_X64", "-o"])
            .arg(&iso_path)
            .arg(&tree)
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            return;
        }
        let devp = dir.join("disk.img");
        let size = 96 * MB;
        File::create(&devp).unwrap().set_len(size).unwrap();
        let rep = analyze(&iso_path).unwrap();
        assert!(rep.windows && rep.efi && !rep.large_file);
        assert_eq!(rep.label, "CCCOMA_X64");
        let iso = iso::Iso::open(&iso_path).unwrap();
        let drive = Drive { path: devp.to_string_lossy().into(), name: "img".into(), size };
        let job = test_job(drive, iso_path);
        let (tx, cancel) = ctx_parts();
        let ctx = Ctx { tx: &tx, cancel: &cancel };
        let parts = layout(&job, false);
        let dev = OpenOptions::new().read(true).write(true).open(&devp).unwrap();
        write_tables(&job, &rep, None, &parts, &dev, &ctx).unwrap();
        write_fat(&job, &rep, Some(&iso), None, &parts[0], &dev, &ctx).unwrap();

        let raw = std::fs::read(&devp).unwrap();
        assert_eq!(&raw[510..512], &[0x55, 0xAA]);
        assert_eq!(raw[446], 0x80);
        let part_img = dir.join("part.img");
        std::fs::write(&part_img, &raw[parts[0].start as usize..(parts[0].start + parts[0].size) as usize]).unwrap();
        {
            let mut f = File::open(&part_img).unwrap();
            let fat = fatfs::FileSystem::new(&mut f, fatfs::FsOptions::new()).unwrap();
            let read = |p: &str| {
                let mut s = Vec::new();
                fat.root_dir().open_file(p).unwrap().read_to_end(&mut s).unwrap();
                s
            };
            let b = read("bootmgr");
            assert_eq!(b.len(), 300_000);
            assert!(b.iter().all(|&x| x == 7), "first bad byte at {:?}", b.iter().position(|&x| x != 7));
            assert_eq!(read("EFI/BOOT/BOOTX64_original.efi"), b"loader");
            assert_eq!(read("EFI/BOOT/BOOTX64.EFI"), MD5_LOADERS[0].1);
            assert!(String::from_utf8(read("md5sum.txt")).unwrap().contains("./bootmgr"));
            assert!(String::from_utf8(read("autounattend.xml")).unwrap().contains("BypassTPMCheck"));
            assert!(String::from_utf8(read("autorun.inf")).unwrap().contains("label = CCCOMA_X64"));
        }
        if let Ok(out) = std::process::Command::new("fsck.vfat").arg("-n").arg(&part_img).output() {
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stdout));
        }
        std::fs::remove_dir_all(dir).ok();
    }
}
