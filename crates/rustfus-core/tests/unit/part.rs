use super::*;
use std::process::Command;

fn layout(gpt: bool) -> Option<String> {
    let path = std::env::temp_dir().join(format!("rustfus-part-{}-{gpt}", std::process::id()));
    let size = 64 << 20;
    let mut f = std::fs::File::create(&path).unwrap();
    f.set_len(size).unwrap();
    let parts = [
        Part {
            start: 1 << 20,
            size: 32 << 20,
            mbr_type: 0x0c,
            gpt_type: GPT_BASIC_DATA,
            name: "Main",
            active: true,
        },
        Part {
            start: 33 << 20,
            size: 1 << 20,
            mbr_type: 0xef,
            gpt_type: GPT_BASIC_DATA,
            name: "UEFI:NTFS",
            active: false,
        },
    ];
    if gpt {
        write_gpt(&mut f, size, &parts)
    } else {
        write_mbr(&mut f, size, &parts)
    }
    .unwrap();
    let out = Command::new("sfdisk").arg("-d").arg(&path).output().ok()?;
    std::fs::remove_file(&path).ok();
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

#[test]
fn sfdisk_reads_tables() {
    let Some(mbr) = layout(false) else { return };
    assert!(mbr.contains("label: dos"), "{mbr}");
    assert!(
        mbr.contains("start=        2048, size=       65536, type=c, bootable"),
        "{mbr}"
    );
    assert!(
        mbr.contains("start=       67584, size=        2048, type=ef"),
        "{mbr}"
    );
    let gpt = layout(true).unwrap();
    assert!(gpt.contains("label: gpt"), "{gpt}");
    assert!(
        gpt.contains("type=EBD0A0A2-B9E5-4433-87C0-68B6B72699C7"),
        "{gpt}"
    );
    assert!(
        gpt.contains("name=\"UEFI:NTFS\", attrs=\"GUID:63\""),
        "{gpt}"
    );
    assert!(
        !gpt.contains("C12A7328-F81F-11D2-BA4B-00A0C93EC93B"),
        "{gpt}"
    );
}
