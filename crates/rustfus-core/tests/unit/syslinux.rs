use super::*;
use crate::bytes::get64;
use std::process::Command;

#[test]
fn versions_of_embedded_loaders() {
    assert_eq!(embedded(false), (0x407, "/2013-07-25".into()));
    assert_eq!(embedded(true), (0x604, "/pre1".into()));
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
    install(&mut f, &info, &|_| {}).unwrap();
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
    let extra = extra_files(&iso, &info, &|_| {}).unwrap();
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
#[ignore]
fn download_6_03() {
    let info = super::info(0x603, "/2014-10-06".into());
    let (sys, bss) = loader(&info, &|m| eprintln!("{m}")).unwrap();
    assert_eq!(sl_version(&sys).unwrap().0, 0x603);
    assert_eq!(bss.len(), 512);
}
