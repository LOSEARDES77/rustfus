use super::*;
use std::io::Cursor;
use std::path::Path;
use std::process::Command;

fn noise(n: usize, mut s: u32) -> Vec<u8> {
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            s as u8
        })
        .collect()
}

#[test]
fn mbr_preserves_table() {
    let orig = noise(1024, 0x1234_5678);
    let kinds = [
        (Mbr::Win7, MBR_WIN7),
        (Mbr::Rufus, MBR_RUFUS),
        (Mbr::Syslinux, MBR_SYSLINUX),
        (Mbr::Grub2, &GRUB2_BOOT_IMG[..0x1b0]),
        (Mbr::Zero, &[0; 440]),
    ];
    for (kind, code) in kinds {
        let mut c = Cursor::new(orig.clone());
        write_mbr(&mut c, kind).unwrap();
        let d = c.into_inner();
        assert_eq!(&d[..code.len()], code, "{kind:?}");
        assert_eq!(d[code.len()..0x1fe], orig[code.len()..0x1fe], "{kind:?}");
        assert_eq!(d[0x1fe..0x200], [0x55, 0xaa]);
        assert_eq!(d[0x200..], orig[0x200..]);
    }
}

#[test]
fn fat32_pbr_on_mkfs_image() {
    let (mkfs, fsck) = ("/usr/bin/mkfs.vfat", "/usr/bin/fsck.vfat");
    if !Path::new(mkfs).exists() || !Path::new(fsck).exists() {
        eprintln!("skipping: dosfstools missing");
        return;
    }
    let img = std::env::temp_dir().join(format!("rustfus-bootrec-{}.img", std::process::id()));
    type Writer = fn(&mut Cursor<Vec<u8>>) -> io::Result<()>;
    let cases = [
        (
            write_fat32_pe_pbr as Writer,
            FAT32PE_0X52,
            FAT32PE_0X3F0,
            Some(FAT32PE_0X1800),
        ),
        (write_fat32_freedos_pbr, FAT32FD_0X52, FAT32FD_0X3F0, None),
    ];
    for (write, c52, c3f0, c1800) in cases {
        let _ = std::fs::remove_file(&img);
        let ok = Command::new(mkfs)
            .args(["-F", "32", "-C"])
            .arg(&img)
            .arg("65536")
            .output()
            .unwrap();
        assert!(ok.status.success());
        let orig = std::fs::read(&img).unwrap();
        let mut c = Cursor::new(orig.clone());
        write(&mut c).unwrap();
        let d = c.into_inner();
        for base in [0, 6 * 512] {
            let (o, d) = (&orig[base..], &d[base..]);
            assert_eq!(&d[..11], FAT32_0X0);
            assert_eq!(d[0x0b..0x40], o[0x0b..0x40]);
            assert_eq!(d[0x40], 0x80);
            assert_eq!(d[0x41..0x5a], o[0x41..0x5a]);
            assert_eq!(&d[0x52..FSINFO_FREE], &c52[..FSINFO_FREE - 0x52]);
            assert_eq!(d[FSINFO_FREE..0x3f0], o[FSINFO_FREE..0x3f0]);
            assert_eq!(&d[0x3f0..0x600], c3f0);
            assert_eq!(d[0x1fe..0x200], [0x55, 0xaa]);
            match c1800 {
                Some(c) => assert_eq!(&d[0x1800..0x1a00], c),
                None => assert_eq!(d[0x1800..0x1a00], o[0x1800..0x1a00]),
            }
        }
        std::fs::write(&img, &d).unwrap();
        let out = Command::new(fsck).arg("-n").arg(&img).output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
    }
    let _ = std::fs::remove_file(&img);
    let mut c = Cursor::new(vec![0u8; 0x4000]);
    assert!(write_fat32_pe_pbr(&mut c).is_err());
    assert!(c.into_inner().iter().all(|&b| b == 0));
}

#[test]
fn ntfs_pbr() {
    let mut orig = noise(0x2000, 0xdead_beef);
    orig[3..11].copy_from_slice(b"NTFS    ");
    let mut c = Cursor::new(orig.clone());
    write_ntfs_pbr(&mut c).unwrap();
    let d = c.into_inner();
    let end = 0x54 + NTFS_0X54.len();
    assert_eq!(&d[..11], NTFS_0X0);
    assert_eq!(d[0x0b..0x54], orig[0x0b..0x54]);
    assert_eq!(&d[0x54..end], NTFS_0X54);
    assert_eq!(d[0x1fe..0x200], [0x55, 0xaa]);
    assert_eq!(d[end..], orig[end..]);

    orig[3] = b'X';
    let mut c = Cursor::new(orig.clone());
    assert!(write_ntfs_pbr(&mut c).is_err());
    assert_eq!(c.into_inner(), orig);
}
