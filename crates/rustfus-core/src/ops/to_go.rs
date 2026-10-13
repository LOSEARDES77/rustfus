// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! Windows To Go installs.

use super::*;

#[cfg(not(windows))]
pub(super) fn write_wtg(_job: &Job, _img: &Path, _ctx: &Ctx) -> Result<String, String> {
    Err("Windows To Go needs DISM and bcdboot, which only exist on Windows".into())
}

/// Windows To Go, like Rufus: ESP (GPT) + NTFS, DISM applies the chosen
/// edition from the mounted ISO, bcdboot makes it bootable.
#[cfg(windows)]
pub(super) fn write_wtg(job: &Job, img: &Path, ctx: &Ctx) -> Result<String, String> {
    let index = job.wtg.unwrap();
    let gpt = job.scheme == Scheme::Gpt;
    let end = (job.drive.size - 34 * part::SECTOR) / MB * MB;
    let esp = 260 * MB;
    let mut parts = Vec::new();
    if gpt {
        parts.push(part::Part {
            start: MB,
            size: esp,
            mbr_type: 0xEF,
            gpt_type: part::GPT_ESP,
            name: "EFI System Partition",
            active: false,
        });
    }
    let start = if gpt { MB + esp } else { MB };
    parts.push(part::Part {
        start,
        size: end - start,
        mbr_type: 0x07,
        gpt_type: part::GPT_BASIC_DATA,
        name: "Windows",
        active: !gpt,
    });

    ctx.progress(0.0, "Unmounting...");
    unmount(&job.drive)?;
    let dev = open_dev(&job.drive)?;
    for p in &parts {
        zero_range(&dev, p.start, p.size.min(MB), None)?;
    }
    {
        let mut disk = BlockDev::new(&dev, 0, job.drive.size / part::SECTOR * part::SECTOR);
        if gpt {
            part::write_gpt(&mut disk, job.drive.size, &parts)
        } else {
            part::write_mbr(&mut disk, job.drive.size, &parts)
        }
        .map_err(e)?;
        if !gpt {
            bootrec::write_mbr(&mut disk, bootrec::Mbr::Win7).map_err(e)?;
        }
        disk.flush().map_err(e)?;
    }
    if gpt {
        let mut vol = BlockDev::new(&dev, MB, esp);
        crate::fat::format(
            &mut vol,
            esp,
            true,
            0,
            fat_label("ESP"),
            (MB / part::SECTOR) as u32,
        )
        .map_err(e)?;
        vol.flush().map_err(e)?;
    }
    drop(dev);
    fsos::rescan(&job.drive)?;
    let n = parts.len() as u32;
    ctx.progress(0.0, "Formatting (NTFS)...");
    fsos::mkfs(
        &job.drive,
        n,
        Fs::Ntfs,
        &sanitize_label(&job.label, Fs::Ntfs),
        job.cluster,
    )?;
    let win = fsos::mount(&job.drive, n, Fs::Ntfs)?;
    let sys = if gpt {
        fsos::mount(&job.drive, 1, Fs::Fat32)?
    } else {
        win.clone()
    };

    let iso_path = img.to_string_lossy().replace('\'', "''");
    let letter = fsos::powershell(&format!(
        "(Mount-DiskImage -ImagePath '{iso_path}' -PassThru | Get-Volume).DriveLetter"
    ))?;
    let letter = letter
        .trim()
        .chars()
        .next()
        .ok_or("could not mount the ISO")?;
    let result = (|| {
        let src = ["install.wim", "install.esd"]
            .iter()
            .map(|f| format!(r"{letter}:\sources\{f}"))
            .find(|p| Path::new(p).exists())
            .ok_or("no install.wim/esd in the image")?;
        let win_s = win.to_string_lossy().trim_end_matches('\\').to_string();
        ctx.progress(0.5, "Applying Windows image (this can take a long time)...");
        ctx.log(format!(
            "dism /Apply-Image /ImageFile:{src} /Index:{index} /ApplyDir:{win_s}\\"
        ));
        run(
            "dism",
            &[
                "/Apply-Image",
                &format!("/ImageFile:{src}"),
                &format!("/Index:{index}"),
                &format!(r"/ApplyDir:{win_s}\"),
            ],
        )?;
        if job.wue.wtg_no_internal {
            let xml = wtg::SAN_POLICY_XML.replace("ARCH", "amd64");
            std::fs::write(win.join("san_policy.xml"), xml).map_err(e)?;
            run(
                "dism",
                &[
                    &format!(r"/Image:{win_s}\"),
                    &format!(r"/Apply-Unattend:{win_s}\san_policy.xml"),
                ],
            )?;
        }
        if job.wue.any() {
            let panther = win.join("Windows").join("Panther");
            std::fs::create_dir_all(&panther).map_err(e)?;
            std::fs::write(
                panther.join("unattend.xml"),
                wue::unattend_xml(&job.wue, "amd64"),
            )
            .map_err(e)?;
        }
        ctx.progress(0.9, "Installing boot files...");
        let sys_s = sys.to_string_lossy().trim_end_matches('\\').to_string();
        run(
            "bcdboot",
            &[&format!(r"{win_s}\Windows"), "/s", &sys_s, "/f", "ALL"],
        )?;
        Ok::<(), String>(())
    })();
    fsos::powershell(&format!("Dismount-DiskImage -ImagePath '{iso_path}'")).ok();
    result?;
    Ok(format!("Windows To Go installed on {}", job.drive.path))
}
