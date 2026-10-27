use super::*;
use crate::bytes::get64;
use std::io::Cursor;
use std::process::Command;

#[test]
fn versions_of_embedded_loaders() {
    assert_eq!(embedded(false), (0x407, "/2013-07-25".into()));
    assert_eq!(embedded(true), (0x604, "/pre1".into()));
    assert_eq!(embedded_version(), "6.04/pre1");
    let mut buf = vec![0u8; 300];
    buf[100..].copy_from_slice(
        &b"ISOLINUX 6.03 6.03-pre20* \0"
            .iter()
            .chain(&[0u8; 173])
            .copied()
            .collect::<Vec<_>>(),
    );
    assert_eq!(sl_version(&buf), Some((0x603, "/pre20".into())));
}

fn tmp(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("rustfus-sl-{}-{name}", std::process::id()))
}

fn check_install(major: u8) {
    let path = tmp(&format!("vol{major}"));
    let size = 100u64 << 20;
    let mut f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .unwrap();
    f.set_len(size).unwrap();
    crate::fat::format(&mut f, size, true, 0, *b"SYSLINUX   ", 2048).unwrap();
    {
        let fs = fatfs::FileSystem::new(&mut f, fatfs::FsOptions::new()).unwrap();
        fs.root_dir().create_dir("isolinux").unwrap();
        fs.root_dir()
            .create_file("isolinux/isolinux.cfg")
            .unwrap()
            .write_all(b"DEFAULT linux\n")
            .unwrap();
        fs.root_dir()
            .create_file("big.bin")
            .unwrap()
            .write_all(&vec![9u8; 700_000])
            .unwrap();
        fs.unmount().unwrap();
    }
    crate::fat::fix_dot_entries(&mut f).unwrap();
    let mut orig = [0u8; 512];
    f.seek(SeekFrom::Start(0)).unwrap();
    f.read_exact(&mut orig).unwrap();

    let (v, ext) = embedded(major >= 5);
    let info = Info {
        cfg_path: "/isolinux/isolinux.cfg".into(),
        ..super::info(v, ext)
    };
    f.seek(SeekFrom::Start(12345)).unwrap();
    install(&mut f, &info, /* allow_download= */ false, &|_| {}).unwrap();
    assert_eq!(f.stream_position().unwrap(), 0);

    let (sys, bss) = if major >= 5 {
        (V6_SYS, V6_BSS)
    } else {
        (V4_SYS, V4_BSS)
    };
    let mut bs = [0u8; 512];
    f.read_exact(&mut bs).unwrap();
    assert_eq!(bs[..11], bss[..11]);
    assert_eq!(bs[11..90], orig[11..90]);
    assert_eq!(bs[510..], [0x55, 0xaa]);

    let mut file = Vec::new();
    f.seek(SeekFrom::Start(0)).unwrap();
    {
        let fs = fatfs::FileSystem::new(&mut f, fatfs::FsOptions::new()).unwrap();
        let e = fs
            .root_dir()
            .iter()
            .map(|e| e.unwrap())
            .find(|e| e.file_name() == "ldlinux.sys")
            .unwrap();
        let a = e.attributes();
        assert!(a.contains(
            fatfs::FileAttributes::READ_ONLY
                | fatfs::FileAttributes::HIDDEN
                | fatfs::FileAttributes::SYSTEM
        ));
        e.to_file().read_to_end(&mut file).unwrap();
        let mut big = Vec::new();
        fs.root_dir()
            .open_file("big.bin")
            .unwrap()
            .read_to_end(&mut big)
            .unwrap();
        assert_eq!(big, vec![9u8; 700_000]);
    }
    let padded = sys.len().div_ceil(512) * 512;
    assert_eq!(file.len(), padded + 1024);
    let pa = (0..padded / 4)
        .map(|i| i * 4)
        .find(|&o| get32(&file, o) == LDLINUX_MAGIC)
        .unwrap();
    let dw = get32(&file, pa + 12) as usize;
    assert_eq!(dw, padded / 4);
    assert_eq!(
        (0..dw).fold(0u32, |c, i| c.wrapping_add(get32(&file, i * 4))),
        LDLINUX_MAGIC
    );
    let data_sectors = get16(&file, pa + 8) as usize;
    assert_eq!(data_sectors, padded / 512);

    let epa = get16(&file, pa + 22) as usize;
    let fld = |n: usize| get16(&file, epa + 2 * n) as usize;
    let sector = |s: u64| {
        let mut b = vec![0u8; 512];
        (&f).seek(SeekFrom::Start(s * 512)).unwrap();
        (&f).read_exact(&mut b).unwrap();
        b
    };
    // Sector 0 of the file comes from the boot sector pointer, the rest from the extents.
    let s0 = get32(&bs, fld(7)) as u64 | (get32(&bs, fld(8)) as u64) << 32;
    assert_eq!(sector(s0), file[..512]);
    let (mut k, mut ex) = (1usize, fld(5));
    while k < data_sectors {
        let (lba, len) = (get64(&file, ex), get16(&file, ex + 8) as u64);
        assert!(len > 0, "extents end before sector {k}");
        for j in 0..len {
            assert_eq!(
                sector(lba + j),
                file[k * 512..(k + 1) * 512],
                "file sector {k}"
            );
            k += 1;
        }
        ex += 10;
    }
    let adv = fld(0);
    for n in 0..2 {
        let s = get64(&file, adv + 8 * n);
        assert_eq!(sector(s), file[padded + 512 * n..padded + 512 * (n + 1)]);
        assert_eq!(get32(&sector(s), 0), 0x5a2d_2fa5);
    }
    let dir = &file[fld(1)..fld(1) + 10];
    assert_eq!(dir, b"/isolinux\0");

    if let Ok(out) = Command::new("fsck.vfat").arg("-n").arg(&path).output() {
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success(),
            "{text}{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    std::fs::remove_file(path).ok();
}

#[test]
fn install_v6() {
    check_install(6);
}

#[test]
fn install_v4() {
    check_install(4);
}

#[test]
fn detect_from_iso() {
    let dir = tmp("isoroot");
    let iso_path = tmp("test.iso");
    std::fs::create_dir_all(dir.join("isolinux")).unwrap();
    let mut bin = vec![0u8; 4096];
    let s = b"ISOLINUX 6.03 2014-10-06\0";
    bin[1000..1000 + s.len()].copy_from_slice(s);
    std::fs::write(dir.join("isolinux/isolinux.bin"), &bin).unwrap();
    std::fs::write(dir.join("isolinux/isolinux.cfg"), "DEFAULT linux\n").unwrap();
    std::fs::write(dir.join("isolinux/ldlinux.c32"), [0u8; 64]).unwrap();
    let status = Command::new("xorriso")
        .args(["-as", "mkisofs", "-quiet", "-R", "-J", "-o"])
        .arg(&iso_path)
        .arg(&dir)
        .status();
    if !status.is_ok_and(|s| s.success()) {
        eprintln!("xorriso unavailable, skipping");
        return;
    }
    let iso = crate::iso::Iso::open(&iso_path).unwrap();
    let info = detect(&iso).unwrap();
    assert_eq!(
        (info.major, info.version.as_str(), info.ext.as_str()),
        (6, "6.03", "/2014-10-06")
    );
    assert_eq!(info.cfg_path, "/isolinux/isolinux.cfg");
    assert!(info.reactos.is_none() && !info.overwrite_cfg);
    let extra = extra_files(&iso, &info, &[], &|_| {}).unwrap();
    assert_eq!(extra.len(), 1);
    assert_eq!(extra[0].0, "syslinux.cfg");
    assert_eq!(
        String::from_utf8_lossy(&extra[0].1),
        "DEFAULT loadconfig\n\nLABEL loadconfig\n  CONFIG /isolinux/isolinux.cfg\n  APPEND /isolinux/\n"
    );
    std::fs::remove_dir_all(dir).ok();
    std::fs::remove_file(iso_path).ok();
}

#[test]
fn old_c32_kept_unless_approved() {
    let dir = tmp("isoroot-c32");
    let iso_path = tmp("test-c32.iso");
    std::fs::create_dir_all(dir.join("isolinux")).unwrap();
    let mut bin = vec![0u8; 4096];
    let s = b"ISOLINUX 4.07 2013-07-25\0";
    bin[1000..1000 + s.len()].copy_from_slice(s);
    std::fs::write(dir.join("isolinux/isolinux.bin"), &bin).unwrap();
    std::fs::write(dir.join("isolinux/isolinux.cfg"), "DEFAULT linux\n").unwrap();
    std::fs::write(dir.join("isolinux/menu.c32"), [0u8; 64]).unwrap();
    let status = Command::new("xorriso")
        .args(["-as", "mkisofs", "-quiet", "-R", "-J", "-o"])
        .arg(&iso_path)
        .arg(&dir)
        .status();
    if !status.is_ok_and(|s| s.success()) {
        eprintln!("xorriso unavailable, skipping");
        return;
    }
    let iso = crate::iso::Iso::open(&iso_path).unwrap();
    let info = detect(&iso).unwrap();
    assert_eq!(info.old_c32, ["isolinux/menu.c32"]);
    let logs = std::cell::RefCell::new(Vec::new());
    let extra = extra_files(&iso, &info, &[], &|m| logs.borrow_mut().push(m)).unwrap();
    assert!(extra.iter().all(|(path, _)| path != "isolinux/menu.c32"));
    assert!(
        logs.borrow()
            .iter()
            .any(|m| m.contains("Keeping the obsolete"))
    );
    std::fs::remove_dir_all(dir).ok();
    std::fs::remove_file(iso_path).ok();
}

#[test]
#[ignore]
fn download_6_03() {
    let info = super::info(0x603, "/2014-10-06".into());
    let (sys, bss) = loader(
        &info,
        /* allow_download= */ true,
        &|m| eprintln!("{m}"),
    )
    .unwrap();
    assert_eq!(sl_version(&sys).unwrap().0, 0x603);
    assert_eq!(bss.len(), 512);
}

/// Without permission, nothing is downloaded: a loader no cache can hold is
/// refused, and one only differing in its extra version falls back to embedded.
#[test]
fn loader_download_needs_permission() {
    assert!(!needs_download(&super::info(0x407, String::new())));
    let (v, ext) = embedded(true);
    assert!(!needs_download(&super::info(v, ext)));

    let unknown = super::info(0x563, String::new());
    assert!(needs_download(&unknown));
    let err = loader(&unknown, /* allow_download= */ false, &|_| {}).unwrap_err();
    assert!(err.contains("not allowed"), "{err}");

    let other_ext = super::info(v, "/rustfus-test".into());
    assert!(needs_download(&other_ext));
    let (sys, _) = loader(&other_ext, /* allow_download= */ false, &|_| {}).unwrap();
    assert_eq!(sys, V6_SYS);
}

fn ntfs_bootsect() -> [u8; 512] {
    let mut bs = [0u8; 512];
    bs[..3].copy_from_slice(&[0xeb, 0x52, 0x90]);
    bs[3..11].copy_from_slice(b"NTFS    ");
    put16(&mut bs, 11, 512);
    bs[13] = 8;
    bs[21] = 0xf8;
    put64(&mut bs, 40, 8191);
    bs[84..510].fill(0xcc);
    bs[510..].copy_from_slice(&[0x55, 0xaa]);
    bs
}

#[test]
fn boot_sector_kinds() {
    assert_eq!(check_bootsect(&ntfs_bootsect()), Ok(VolumeKind::Ntfs));
    let mut exfat = ntfs_bootsect();
    exfat[3..11].copy_from_slice(b"EXFAT   ");
    exfat[21] = 0;
    assert!(check_bootsect(&exfat).is_err());
}

#[test]
fn install_staged_on_ntfs() {
    let bs = ntfs_bootsect();
    let mut vol = Cursor::new(vec![0u8; 4 << 20]);
    vol.get_mut()[..512].copy_from_slice(&bs);
    let (v, ext) = embedded(true);
    let info = Info {
        cfg_path: "/boot/isolinux/isolinux.cfg".into(),
        ..super::info(v, ext)
    };
    let (img, bss) = padded_loader(&info, /* allow_download= */ false, &|_| {}).unwrap();
    let n = ldlinux_sectors(&img) as u64;
    // Two runs, the second before the first, like a fragmented NTFS file.
    let sectors: Vec<u64> = (0..n)
        .map(|i| if i < 20 { 6000 + i } else { 1000 + i })
        .collect();
    let staged = Staged {
        img: img.clone(),
        bss: bss.clone(),
        sectors: sectors.clone(),
    };
    install_staged(&mut vol, &info, staged, &|_| {}).unwrap();
    let raw = vol.into_inner();

    let file: Vec<u8> = sectors[..img.len() / 512]
        .iter()
        .flat_map(|&s| raw[s as usize * 512..(s as usize + 1) * 512].to_vec())
        .collect();
    let pa = (0..file.len() / 4)
        .map(|i| i * 4)
        .find(|&o| get32(&file, o) == LDLINUX_MAGIC)
        .unwrap();
    let dw = get32(&file, pa + 12) as usize;
    assert_eq!(
        (0..dw).fold(0u32, |c, i| c.wrapping_add(get32(&file, i * 4))),
        LDLINUX_MAGIC
    );
    let epa = get16(&file, pa + 22) as usize;
    let fld = |n: usize| get16(&file, epa + 2 * n) as usize;
    assert_eq!(&file[fld(1)..fld(1) + 15], b"/boot/isolinux\0");
    assert_eq!(get32(&raw, fld(7)) as u64, sectors[0]);
    let adv = fld(0);
    assert_eq!(get64(&file, adv), sectors[n as usize - 2]);
    assert_eq!(get64(&file, adv + 8), sectors[n as usize - 1]);

    // NTFS keeps its OEM name and BPB; jump and code come from the .bss.
    assert_eq!(raw[..3], bss[..3]);
    assert_eq!(raw[3..80], bs[3..80]);
    let mut code = raw[80..510].to_vec();
    for p in [fld(7), fld(8)] {
        code[p - 80..p - 80 + 4].copy_from_slice(&bss[p..p + 4]);
    }
    assert_eq!(code, bss[80..510]);
    assert_eq!(raw[510..512], [0x55, 0xaa]);
}

#[cfg(target_os = "linux")]
#[test]
fn maps_file_sectors() {
    // Not under temp_dir(), which is often a tmpfs without block mapping.
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!("rustfus-sl-{}-mapped", std::process::id()));
    let mut f = std::fs::File::create(&path).unwrap();
    // Incompressible, so a compressing filesystem (btrfs) can't encode the extent.
    let data: Vec<u8> = (0..64 * 1024u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8)
        .collect();
    f.write_all(&data).unwrap();
    f.sync_all().unwrap();
    match file_sectors(&f, 128) {
        Ok(s) => {
            assert_eq!(s.len(), 128);
            let mut unique = s.clone();
            unique.sort_unstable();
            unique.dedup();
            assert_eq!(unique.len(), 128, "sectors must not repeat");
            // filefrag reads the same FIEMAP; it confirms the units and offsets.
            if let Ok(out) = Command::new("filefrag")
                .args(["-v", "-b512"])
                .arg(&path)
                .output()
            {
                let text = String::from_utf8_lossy(&out.stdout);
                let first = text
                    .lines()
                    .find_map(|l| l.trim().strip_prefix("0:"))
                    .and_then(|l| l.split(':').nth(1))
                    .and_then(|l| l.split("..").next())
                    .and_then(|l| l.trim().parse::<u64>().ok());
                if let Some(first) = first {
                    assert_eq!(s[0], first, "{text}");
                }
            }
        }
        Err(e) => eprintln!("no FIEMAP/FIBMAP on {} ({e}), skipping", path.display()),
    }
    std::fs::remove_file(path).ok();
}

#[test]
fn knoppix_redirects_only_for_syslinux_to_isolinux() {
    let find = |p: &str| {
        p.eq_ignore_ascii_case("boot/isolinux/syslnx64.cfg")
            .then(|| "BOOT/ISOLINUX/SYSLNX64.CFG".to_string())
    };
    let cfgs = knoppix_redirects("boot/syslinux", "isolinux", find).unwrap();
    assert_eq!(cfgs.len(), 1);
    assert_eq!(cfgs[0].0, "boot/syslinux/syslnx64.cfg");
    assert_eq!(
        String::from_utf8_lossy(&cfgs[0].1),
        "DEFAULT loadconfig\n\nLABEL loadconfig\n  CONFIG /BOOT/ISOLINUX/SYSLNX64.CFG\n  APPEND /BOOT/ISOLINUX\n"
    );
    assert!(knoppix_redirects("boot/syslinux", "../isolinux", find).is_none());
    assert!(knoppix_redirects("boot/grub", "isolinux", find).is_none());
}
