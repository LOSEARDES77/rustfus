use super::*;
use std::process::Command;

#[test]
fn crc32c_check_value() {
    assert_eq!(!crc32c(!0, b"123456789"), 0xe306_9283);
}

#[test]
fn superblock_in_memory() {
    let mut c = io::Cursor::new(vec![0u8; 8 << 20]);
    format(&mut c, 8 << 20, Kind::Ext4, "casper-rw", 0, &[]).unwrap();
    let b = c.into_inner();
    assert_eq!(&b[1080..1082], &[0x53, 0xef]);
    assert_eq!(&b[1024 + 0x78..1024 + 0x78 + 10], b"casper-rw\0");
}

#[test]
fn e2fsck_clean() {
    let run = |cmd: &str, args: &[&str], img: &std::path::Path| {
        let o = Command::new(cmd).args(args).arg(img).output().unwrap();
        let text =
            String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr);
        (o.status.success(), o.stdout, text)
    };
    if ["e2fsck", "dumpe2fs", "debugfs"]
        .iter()
        .any(|t| Command::new(t).arg("-V").output().is_err())
    {
        eprintln!("e2fsprogs not installed, skipping");
        return;
    }
    let big: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    for kind in [Kind::Ext2, Kind::Ext3, Kind::Ext4] {
        for (size, bs) in [(64u64 << 20, 1024), (300 << 20, 0), (2 << 30, 4096)] {
            let path = std::env::temp_dir().join(format!(
                "rustfus-ext-{}-{kind:?}-{size}",
                std::process::id()
            ));
            let mut f = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(&path)
                .unwrap();
            f.set_len(size).unwrap();
            format(
                &mut f,
                size,
                kind,
                "persistence",
                bs,
                &[("persistence.conf", b"/ union\n"), ("big.bin", &big)],
            )
            .unwrap();
            drop(f);
            let (ok, _, out) = run("e2fsck", &["-fn"], &path);
            assert!(ok && !out.contains("Fix?"), "{kind:?} {size}: {out}");
            let (_, _, out) = run("dumpe2fs", &["-h"], &path);
            assert!(
                out.contains("Filesystem volume name:   persistence\n"),
                "{out}"
            );
            assert_eq!(
                run("debugfs", &["-R", "cat /persistence.conf"], &path).1,
                b"/ union\n"
            );
            assert!(
                run("debugfs", &["-R", "cat /big.bin"], &path).1 == big,
                "{kind:?} {size}: big.bin differs"
            );
            std::fs::remove_file(path).ok();
        }
    }
}
