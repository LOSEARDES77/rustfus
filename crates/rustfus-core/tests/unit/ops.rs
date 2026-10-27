use super::dd::{copy, verify_dev};
use super::files::{patch_label, patch_persistence};
use super::format::write_fat;
use super::layout::{layout, write_tables};
use super::*;
use crate::{bootrec, syslinux};
use std::io::Cursor;

#[test]
fn label_is_sanitized() {
    assert_eq!(sanitize_label("my'; rm -rf", Fs::Ntfs), "my rm -rf");
    assert_eq!(sanitize_label("abcdefghijklmn", Fs::Fat32), "ABCDEFGHIJK");
    assert_eq!(
        sanitize_label("CCCOMA_X64FRE_ES-ES_DV9", Fs::Exfat),
        "CCCOMA_X64F"
    );
    assert_eq!(sanitize_label("$()", Fs::Exfat), "USB");
}

#[test]
fn grub_version_is_found() {
    assert_eq!(
        grub_version(b"\0\0GNU GRUB  version 2.12\0rest").as_deref(),
        Some("2.12")
    );
    assert_eq!(grub_version(b"nothing here"), None);
    assert_eq!(
        grub_version(b"\0GNU GRUB  version %s\x002.14-2ubuntu1\0").as_deref(),
        Some("2.14-2ubuntu1")
    );
}

#[test]
fn persistence_patch_is_idempotent() {
    let once = patch_persistence("linux /casper/vmlinuz boot=casper quiet").unwrap();
    assert_eq!(once, "linux /casper/vmlinuz boot=casper persistent quiet");
    assert_eq!(patch_persistence(&once), None);
    // Ubuntu 23.04+: no boot=casper, no preseed.
    let grub = "linux  /casper/vmlinuz  --- quiet splash\ninitrd /casper/initrd\n";
    let once = patch_persistence(grub).unwrap();
    assert_eq!(
        once,
        "linux  /casper/vmlinuz persistent  --- quiet splash\ninitrd /casper/initrd\n"
    );
    assert_eq!(patch_persistence(&once), None);
    assert_eq!(
        patch_persistence("linux /casper/vmlinuz file=/cdrom/preseed/ubuntu.seed quiet").as_deref(),
        Some("linux /casper/vmlinuz persistent file=/cdrom/preseed/ubuntu.seed quiet")
    );
    assert_eq!(
        patch_persistence("append boot=live components").as_deref(),
        Some("append boot=live persistence components")
    );
    assert_eq!(patch_persistence("menuentry 'Memtest'"), None);
}

#[test]
fn label_patch_targets_kernel_lines() {
    let grub =
        "search --label 'Fedora 40'\n\tlinux /vmlinuz root=live:CDLABEL=Fedora\\x2040 quiet\n";
    assert_eq!(
        patch_label("EFI/BOOT/grub.cfg", grub, "Fedora 40", "FEDORA 40").as_deref(),
        Some(
            "search --label 'Fedora 40'\n\tlinux /vmlinuz root=live:CDLABEL=FEDORA\\x2040 quiet\n"
        )
    );
    let isolinux = "label linux\n  append initrd=i.img coreos.liveiso=fedora-coreos-44 rw\n";
    assert_eq!(
        patch_label(
            "isolinux/isolinux.cfg",
            isolinux,
            "fedora-coreos-44",
            "FEDORA-CORE"
        )
        .as_deref(),
        Some("label linux\n  append initrd=i.img coreos.liveiso=FEDORA-CORE rw\n")
    );
    let entry = "options root=live:CDLABEL=ARCH_2026\n";
    assert_eq!(
        patch_label("loader/entries/a.conf", entry, "ARCH_2026", "ARCH_2026"),
        None
    );
    assert_eq!(
        patch_label("README.txt", isolinux, "fedora-coreos-44", "X"),
        None
    );
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
    let ctx = Ctx {
        tx: &tx,
        cancel: &cancel,
    };
    let dev = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&devp)
        .unwrap();
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
    let drive = Drive {
        path: "x".into(),
        name: "x".into(),
        size: 1000 * MB + 12345,
    };
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
        wue: wue::Options {
            bypass_hw_checks: true,
            ..Default::default()
        },
        allow_downloads: false,
        replace_c32: Vec::new(),
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
    let drive = Drive {
        path: devp.to_string_lossy().into(),
        name: "img".into(),
        size,
    };
    let mut job = test_job(drive, iso_path);
    job.uefi_validation = false;
    let (tx, cancel) = ctx_parts();
    let ctx = Ctx {
        tx: &tx,
        cancel: &cancel,
    };
    let parts = layout(&job, false);
    let dev = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&devp)
        .unwrap();
    write_tables(&job, &rep, Some(&sl), &parts, &dev, &ctx).unwrap();
    write_fat(&job, &rep, Some(&iso), Some(&sl), &parts[0], &dev, &ctx).unwrap();

    let raw = std::fs::read(&devp).unwrap();
    assert_eq!(
        &raw[..16],
        &bootrec_mbr_syslinux()[..16],
        "Syslinux MBR code"
    );
    let p = &raw[parts[0].start as usize..(parts[0].start + parts[0].size) as usize];
    let part_img = dir.join("part.img");
    std::fs::write(&part_img, p).unwrap();
    let mut f = File::open(&part_img).unwrap();
    let fat = fatfs::FileSystem::new(&mut f, fatfs::FsOptions::new()).unwrap();
    let mut cfg = String::new();
    fat.root_dir()
        .open_file("syslinux.cfg")
        .unwrap()
        .read_to_string(&mut cfg)
        .unwrap();
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
    let drive = Drive {
        path: devp.to_string_lossy().into(),
        name: "img".into(),
        size,
    };
    let job = test_job(drive, iso_path);
    let (tx, cancel) = ctx_parts();
    let ctx = Ctx {
        tx: &tx,
        cancel: &cancel,
    };
    let parts = layout(&job, false);
    let dev = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&devp)
        .unwrap();
    write_tables(&job, &rep, None, &parts, &dev, &ctx).unwrap();
    write_fat(&job, &rep, Some(&iso), None, &parts[0], &dev, &ctx).unwrap();

    let raw = std::fs::read(&devp).unwrap();
    assert_eq!(&raw[510..512], &[0x55, 0xaa]);
    assert_eq!(raw[446], 0x80);
    let part_img = dir.join("part.img");
    std::fs::write(
        &part_img,
        &raw[parts[0].start as usize..(parts[0].start + parts[0].size) as usize],
    )
    .unwrap();
    {
        let mut f = File::open(&part_img).unwrap();
        let fat = fatfs::FileSystem::new(&mut f, fatfs::FsOptions::new()).unwrap();
        let read = |p: &str| {
            let mut s = Vec::new();
            fat.root_dir()
                .open_file(p)
                .unwrap()
                .read_to_end(&mut s)
                .unwrap();
            s
        };
        let b = read("bootmgr");
        assert_eq!(b.len(), 300_000);
        assert!(
            b.iter().all(|&x| x == 7),
            "first bad byte at {:?}",
            b.iter().position(|&x| x != 7)
        );
        assert_eq!(read("EFI/BOOT/BOOTX64_original.efi"), b"loader");
        assert_eq!(read("EFI/BOOT/BOOTX64.EFI"), MD5_LOADERS[0].1);
        assert!(
            String::from_utf8(read("md5sum.txt"))
                .unwrap()
                .contains("./bootmgr")
        );
        assert!(
            String::from_utf8(read("autounattend.xml"))
                .unwrap()
                .contains("BypassTPMCheck")
        );
        assert!(
            String::from_utf8(read("autorun.inf"))
                .unwrap()
                .contains("label = CCCOMA_X64")
        );
    }
    if let Ok(out) = std::process::Command::new("fsck.vfat")
        .arg("-n")
        .arg(&part_img)
        .output()
    {
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
    }
    std::fs::remove_dir_all(dir).ok();
}
