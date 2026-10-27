use super::*;
use std::process::Command;

fn check(fat32: bool, size: u64) {
    let path = std::env::temp_dir().join(format!("rustfus-fat-{}-{fat32}", std::process::id()));
    let mut f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .unwrap();
    f.set_len(size).unwrap();
    format(&mut f, size, fat32, 0, *b"TESTLABEL  ", 2048).unwrap();
    {
        let fs = fatfs::FileSystem::new(&mut f, fatfs::FsOptions::new()).unwrap();
        fs.root_dir()
            .create_file("bootmgr")
            .unwrap()
            .write_all(&vec![7u8; 300_000])
            .unwrap();
        fs.root_dir().create_dir("EFI").unwrap();
        fs.root_dir().create_dir("EFI/BOOT").unwrap();
        fs.root_dir()
            .create_file("EFI/BOOT/BOOTX64.EFI")
            .unwrap()
            .write_all(b"loader")
            .unwrap();
        fs.unmount().unwrap();
    }
    fix_dot_entries(&mut f).unwrap();
    {
        let fs = fatfs::FileSystem::new(&mut f, fatfs::FsOptions::new()).unwrap();
        let mut v = Vec::new();
        fs.root_dir()
            .open_file("bootmgr")
            .unwrap()
            .read_to_end(&mut v)
            .unwrap();
        assert_eq!(v, vec![7u8; 300_000]);
        assert_eq!(fs.volume_label().trim(), "TESTLABEL");
    }
    let mut bs = [0u8; 512];
    f.seek(SeekFrom::Start(0)).unwrap();
    f.read_exact(&mut bs).unwrap();
    if fat32 {
        assert_eq!(get16(&bs, 0x0e), 32);
    }
    assert_eq!(get32(&bs, 0x1c), 2048);
    if let Ok(out) = Command::new("fsck.vfat").arg("-n").arg(&path).output() {
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && !text.contains("Expected") && !text.contains("Bad"),
            "{text}"
        );
    }
    std::fs::remove_file(path).ok();
}

#[test]
fn fat32_formats_and_passes_fsck() {
    check(true, 100 << 20);
}

#[test]
fn fat16_formats_and_passes_fsck() {
    check(false, 200 << 20);
}

#[test]
fn fat32_rejects_tiny_volume() {
    let mut c = io::Cursor::new(vec![0u8; 8 << 20]);
    assert!(format(&mut c, 8 << 20, true, 4096, [b' '; 11], 0).is_err());
}
