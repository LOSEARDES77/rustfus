// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! DD-mode writes: raw image copy to the device and read-back verification.

use super::*;

/// Without this, Linux verification would read back the page cache.
pub(super) fn drop_cache(_f: &File) {
    #[cfg(target_os = "linux")]
    unsafe {
        use std::os::fd::AsRawFd;
        libc::posix_fadvise(_f.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED);
    }
}

pub(super) fn read_full(r: &mut dyn Read, buf: &mut [u8]) -> Result<usize, String> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..]).map_err(e)? {
            0 => break,
            k => n += k,
        }
    }
    Ok(n)
}

/// DD-mode write of `img` (decompressed on the fly), optionally verified.
pub(super) fn write_dd(img: &Path, d: &Drive, verify: bool, ctx: &Ctx) -> Result<String, String> {
    if image::is_ffu(img) {
        ctx.progress(0.0, "Unmounting...");
        unmount(d)?;
        ctx.progress(0.5, "Applying FFU image with DISM...");
        image::apply_ffu(img, &raw_path(d)).map_err(e)?;
        return Ok(format!("Applied {} to {}", img.display(), d.path));
    }
    let src = image::open(img).map_err(e)?;
    if let Some(size) = src.size.filter(|&s| s > d.size) {
        return Err(format!(
            "Image ({}) is larger than the drive ({})",
            human(size),
            human(d.size)
        ));
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

pub(super) fn copy(
    mut src: image::Source,
    dev: &File,
    limit: u64,
    ctx: &Ctx,
) -> Result<u64, String> {
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

pub(super) fn verify_dev(
    mut src: image::Source,
    dev: &File,
    total: u64,
    ctx: &Ctx,
) -> Result<(), String> {
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
