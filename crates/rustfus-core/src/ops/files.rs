// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! Copying the image tree and Rustfus' extras onto the new filesystem.

use super::*;

/// Where files land: Rustfus' own FAT driver, or a mounted OS filesystem.
pub(super) enum Dest<'a, T: fatfs::ReadWriteSeek> {
    Fat(&'a fatfs::FileSystem<T>),
    Dir(PathBuf),
}

/// `Dest::Dir` never touches the FAT type parameter.
pub(super) type DirDest = Dest<'static, Cursor<Vec<u8>>>;

impl<T: fatfs::ReadWriteSeek> Dest<'_, T> {
    pub(super) fn mkdir(&self, p: &str) -> io::Result<()> {
        match self {
            Dest::Fat(fs) => fs.root_dir().create_dir(p).map(|_| ()),
            Dest::Dir(root) => std::fs::create_dir_all(root.join(p)),
        }
    }

    pub(super) fn create(&self, p: &str) -> io::Result<Box<dyn Write + '_>> {
        match self {
            Dest::Fat(fs) => {
                let mut f = fs.root_dir().create_file(p)?;
                f.truncate()?;
                Ok(Box::new(f))
            }
            Dest::Dir(root) => Ok(Box::new(File::create(root.join(p))?)),
        }
    }

    pub(super) fn put(&self, p: &str, data: &[u8]) -> io::Result<()> {
        self.create(p)?.write_all(data)
    }
}

pub(super) fn is_boot_cfg(p: &str) -> bool {
    let p = p.to_ascii_lowercase();
    p.ends_with(".cfg")
        && (p.starts_with("boot/grub") || p.starts_with("isolinux") || p.starts_with("syslinux"))
}

/// Adds the persistence kernel option, as Rufus does for casper/live ISOs.
pub(super) fn patch_persistence(cfg: &str) -> String {
    cfg.replace("boot=casper persistent", "boot=casper")
        .replace("boot=live persistence", "boot=live")
        .replace("boot=casper", "boot=casper persistent")
        .replace("boot=live", "boot=live persistence")
}

/// Copies the ISO tree plus Rufus' extras (validation loaders, unattend,
/// FreeDOS, autorun) to `dest`.
pub(super) fn copy_files<T: fatfs::ReadWriteSeek>(
    job: &Job,
    rep: &Report,
    src: Option<&iso::Iso>,
    dest: &Dest<T>,
    ctx: &Ctx,
) -> Result<(), String> {
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
            dest.mkdir(&entry.path)
                .map_err(|err| format!("{}: {err}", entry.path))?;
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
            && let Some((_, loader)) = MD5_LOADERS
                .iter()
                .find(|(a, _)| lower == format!("efi/boot/boot{a}.efi"))
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
            out.write_all(&buf[..n])
                .map_err(|err| format!("{path}: {err}"))?;
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
        dest.put("md5sum.txt", (header + &md5sums).as_bytes())
            .map_err(e)?;
    }
    if rep.windows && job.wue.any() {
        ctx.log("Writing autounattend.xml");
        dest.put(
            "autounattend.xml",
            wue::unattend_xml(&job.wue, rep.arch).as_bytes(),
        )
        .map_err(e)?;
    }
    if job.boot == Boot::FreeDos {
        dest.mkdir("LOCALE").map_err(e)?;
        for (name, data) in FREEDOS.iter() {
            let p = if matches!(*name, "COMMAND.COM" | "KERNEL.SYS") {
                name.to_string()
            } else {
                format!("LOCALE/{name}")
            };
            dest.put(&p, data).map_err(e)?;
        }
        dest.put("AUTOEXEC.BAT", b"@echo off\r\nSET PATH=\\LOCALE\r\n")
            .map_err(e)?;
    }
    if job.ext_label && !has("autorun.inf") {
        let inf = format!(
            "; Created by Rustfus\r\n[autorun]\r\nicon  = autorun.ico\r\nlabel = {}\r\n",
            job.label
        );
        dest.put("autorun.inf", inf.as_bytes()).map_err(e)?;
        dest.put("autorun.ico", AUTORUN_ICO).map_err(e)?;
    }
    Ok(())
}
