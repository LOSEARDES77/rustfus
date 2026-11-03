use super::*;

#[test]
fn regular_image_storage_sync() {
    let path = std::env::temp_dir().join(format!("rustfus-sync-{}", std::process::id()));
    let file = File::create(&path).unwrap();
    (&file).write_all(b"image storage").unwrap();
    sync_storage(&file).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"image storage");
    drop(file);
    std::fs::remove_file(path).unwrap();
}

#[cfg(target_os = "macos")]
#[test]
fn unsupported_device_sync_error_is_not_ignored() {
    use std::os::fd::AsRawFd;
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/null")
        .unwrap();
    // SAFETY: the borrowed descriptor is live and this ioctl has no payload.
    assert_eq!(
        unsafe { libc::ioctl(file.as_raw_fd(), 0x2000_6416 as libc::c_ulong) },
        -1
    );
    let expected = io::Error::last_os_error().raw_os_error();
    let error = sync_storage(&file).unwrap_err();
    assert_eq!(error.raw_os_error(), expected);
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "attaches a newly created disposable disk image, never a physical USB"]
fn macos_raw_disk_image_flushes_partition_map_and_bad_blocks() {
    use std::process::Command;

    let directory = tempfile::tempdir().unwrap();
    let image = directory.path().join("disposable.img");
    let size = 32 << 20;
    File::create(&image).unwrap().set_len(size).unwrap();
    let output = Command::new("/usr/bin/hdiutil")
        .args([
            "attach",
            "-nomount",
            "-plist",
            "-imagekey",
            "diskimage-class=CRawDiskImage",
        ])
        .arg(&image)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let plist = plist::Value::from_reader(io::Cursor::new(output.stdout)).unwrap();
    let device = plist.as_dictionary().unwrap()["system-entities"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| entry.as_dictionary()?.get("dev-entry")?.as_string())
        .find(|path| {
            path.strip_prefix("/dev/disk")
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit()))
        })
        .expect("hdiutil must return a whole disk for this disposable image")
        .to_string();
    struct Detach(String);
    impl Drop for Detach {
        fn drop(&mut self) {
            let _ = Command::new("/usr/bin/hdiutil")
                .args(["detach", &self.0])
                .status();
        }
    }
    let detach = Detach(device.clone());
    let raw = device.replacen("/dev/disk", "/dev/rdisk", 1);
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&raw)
        .unwrap();
    // Reproduce the old flush failure on the disposable raw descriptor.
    assert_eq!(
        file.sync_all().unwrap_err().raw_os_error(),
        Some(libc::ENOTTY)
    );
    {
        let mut disk = BlockDev::new(&file, 0, size);
        crate::part::write_mbr(&mut disk, size, &[]).unwrap();
        disk.flush().unwrap();
    }
    let mut sector = [0u8; 512];
    read_at(&file, &mut sector, 0).unwrap();
    assert_eq!(&sector[510..], &[0x55, 0xaa]);

    drop(file);
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let report = crate::image::bad_blocks(&raw, 4096, 0, &|_, _| {}, &cancel).unwrap();
    assert_eq!(report.bad_sectors, 0);
    let block = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&device)
        .unwrap();
    sync_storage(&block).unwrap();
    drop(block);
    drop(detach);
}

#[test]
fn unaligned_io_round_trips() {
    let path = std::env::temp_dir().join(format!("rustfus-bd-{}", std::process::id()));
    std::fs::write(&path, vec![0xaau8; 3 * BLOCK]).unwrap();
    let f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    {
        let mut d = BlockDev::new(&f, 4096, 2 * BLOCK as u64);
        d.seek(SeekFrom::Start(BLOCK as u64 - 3)).unwrap();
        d.write_all(b"hello").unwrap();
        let big: Vec<u8> = (0..BLOCK + 100).map(|i| i as u8).collect();
        d.seek(SeekFrom::Start(700)).unwrap();
        d.write_all(&big[..BLOCK - 800]).unwrap();
        assert!(d.write_all(&[0; 1]).is_ok());
        d.seek(SeekFrom::End(0)).unwrap();
        assert!(d.write_all(&[0; 1]).is_err());
        d.flush().unwrap();
        let mut back = [0u8; 5];
        d.seek(SeekFrom::Start(BLOCK as u64 - 3)).unwrap();
        d.read_exact(&mut back).unwrap();
        assert_eq!(&back, b"hello");
    }
    let raw = std::fs::read(&path).unwrap();
    assert!(raw[..4096 + 700].iter().all(|&b| b == 0xaa));
    assert_eq!(raw[4096 + 700], 0);
    assert_eq!(&raw[4096 + BLOCK - 3..4096 + BLOCK + 2], b"hello");
    assert!(raw[4096 + 2 * BLOCK..].iter().all(|&b| b == 0xaa));
    std::fs::remove_file(path).ok();
}
