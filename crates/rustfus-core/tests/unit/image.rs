use super::*;
use std::path::PathBuf;

fn tmp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("rustfus-image-{}-{name}", std::process::id()))
}

fn data() -> Vec<u8> {
    (0..300_000u32)
        .map(|i| (i % 251) as u8 ^ (i >> 10) as u8)
        .collect()
}

fn roundtrip(name: &str, bytes: &[u8]) -> Vec<u8> {
    let p = tmp(name);
    std::fs::write(&p, bytes).unwrap();
    let mut s = open(&p).unwrap();
    let mut out = vec![];
    s.reader.read_to_end(&mut out).unwrap();
    assert_eq!(s.total, bytes.len() as u64);
    assert!(s.consumed.load(Ordering::Relaxed) > 0);
    std::fs::remove_file(&p).ok();
    out
}

#[test]
fn compression_formats_roundtrip() {
    let d = data();
    let mut gz = flate2::write::GzEncoder::new(vec![], flate2::Compression::fast());
    gz.write_all(&d).unwrap();
    assert_eq!(roundtrip("a.img.gz", &gz.finish().unwrap()), d);

    let mut bz = bzip2::write::BzEncoder::new(vec![], bzip2::Compression::fast());
    bz.write_all(&d).unwrap();
    assert_eq!(roundtrip("a.img.bz2", &bz.finish().unwrap()), d);

    let mut xz = lzma_rust2::XzWriter::new(vec![], lzma_rust2::XzOptions::with_preset(1)).unwrap();
    xz.write_all(&d).unwrap();
    assert_eq!(roundtrip("a.img.xz", &xz.finish().unwrap()), d);

    let opts = lzma_rust2::LzmaOptions::with_preset(1);
    let mut lz =
        lzma_rust2::LzmaWriter::new_use_header(vec![], &opts, Some(d.len() as u64)).unwrap();
    lz.write_all(&d).unwrap();
    assert_eq!(roundtrip("a.img.lzma", &lz.finish().unwrap()), d);

    let zst =
        ruzstd::encoding::compress_to_vec(&d[..], ruzstd::encoding::CompressionLevel::Fastest);
    assert_eq!(roundtrip("a.img.zst", &zst), d);

    for method in [
        zip::CompressionMethod::Stored,
        zip::CompressionMethod::Deflated,
    ] {
        let mut z = zip::ZipWriter::new(io::Cursor::new(vec![]));
        let o = zip::write::SimpleFileOptions::default().compression_method(method);
        z.add_directory("dir/", o).unwrap();
        z.start_file("dir/disk.img", o).unwrap();
        z.write_all(&d).unwrap();
        assert_eq!(roundtrip("a.zip", &z.finish().unwrap().into_inner()), d);
    }
    assert!(is_compressed(Path::new("x.IMG.XZ")) && !is_compressed(Path::new("x.vhd")));
}

#[test]
fn vhd_fixed_stripped_differencing_rejected() {
    let d = vec![7u8; 4096];
    let mut v = d.clone();
    v.extend_from_slice(&vhd_footer(d.len() as u64));
    assert_eq!(roundtrip("fixed.vhd", &v), d);

    let n = v.len();
    v[n - 512 + 60..n - 512 + 64].copy_from_slice(&4u32.to_be_bytes());
    let p = tmp("diff.vhd");
    std::fs::write(&p, &v).unwrap();
    let err = open(&p).err().unwrap();
    assert!(err.to_string().contains("Differencing"));
    std::fs::remove_file(&p).ok();
}

#[test]
fn hashes_abc() {
    let p = tmp("abc");
    std::fs::write(&p, b"abc").unwrap();
    let h = hashes(&p, &|_| {}, &AtomicBool::new(false)).unwrap();
    std::fs::remove_file(&p).ok();
    assert_eq!(h[0], "900150983cd24fb0d6963f7d28e17f72");
    assert_eq!(h[1], "a9993e364706816aba3e25717850c26c9cd0d89d");
    assert_eq!(
        h[2],
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert!(h[3].starts_with("ddaf35a193617aba") && h[3].ends_with("a54ca49f"));
    assert!(hashes(&p, &|_| {}, &AtomicBool::new(true)).is_err());
}

#[test]
fn bad_blocks_clean_file() {
    let p = tmp("bb.bin");
    File::create(&p).unwrap().set_len((5 << 20) + 1536).unwrap();
    for (mode, (label, _)) in BAD_BLOCK_MODES.iter().enumerate() {
        let r = bad_blocks(
            p.to_str().unwrap(),
            (5 << 20) + 1536,
            mode,
            &|_, _| {},
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(r, BadBlocks::default(), "{label}");
    }
    std::fs::remove_file(&p).ok();
    let mut b = [0u8; 7];
    pattern_fill(&mut b, 0x1c71c7);
    assert_eq!(b, [0x1c, 0x71, 0xc7, 0x1c, 0x71, 0xc7, 0x1c]);
    pattern_fill(&mut b, 0);
    assert_eq!(b, [0; 7]);
}

fn qemu_img(args: &[&str]) -> Option<bool> {
    std::process::Command::new("qemu-img")
        .args(args)
        .output()
        .ok()
        .map(|o| o.status.success())
}

fn read_all(p: &Path) -> Vec<u8> {
    let mut s = open(p).unwrap();
    let mut out = vec![];
    s.reader.read_to_end(&mut out).unwrap();
    assert_eq!(s.size, Some(out.len() as u64));
    out
}

#[test]
fn save_vhdx_roundtrip() {
    let (dev, out) = (tmp("vdev.bin"), tmp("out.vhdx"));
    let mut d = vec![0u8; 70 << 20];
    for (at, len) in [(0, 4096), (20 << 20, 1 << 20), ((70 << 20) - 512, 512)] {
        d[at..at + len]
            .iter_mut()
            .enumerate()
            .for_each(|(i, b)| *b = (i % 251) as u8 + 1);
    }
    std::fs::write(&dev, &d).unwrap();
    save_image(
        dev.to_str().unwrap(),
        d.len() as u64,
        &out,
        &|_| {},
        &AtomicBool::new(false),
    )
    .unwrap();
    let v = std::fs::read(&out).unwrap();
    // 3 blocks of 32 MiB, the middle one all zero: 2 payload blocks + 4 MiB of headers/log/metadata/BAT.
    assert_eq!(v.len(), (4 + 64) << 20);
    assert_eq!(&v[..8], b"vhdxfile");
    for off in [64 << 10, 128 << 10] {
        let h = &v[off..off + 4096];
        let crc = u32::from_le_bytes(h[4..8].try_into().unwrap());
        let mut z = h.to_vec();
        z[4..8].fill(0);
        assert_eq!(crc, crate::vhdx::crc32c(&z));
    }
    for off in [192 << 10, 256 << 10] {
        let mut r = v[off..off + (64 << 10)].to_vec();
        let crc = u32::from_le_bytes(r[4..8].try_into().unwrap());
        r[4..8].fill(0);
        assert_eq!(crc, crate::vhdx::crc32c(&r));
    }
    assert!(read_all(&out) == d);
    let (o, ds) = (out.to_str().unwrap(), dev.to_str().unwrap());
    if qemu_img(&["--version"]).is_some() {
        assert_eq!(qemu_img(&["check", "-f", "vhdx", o]), Some(true));
        assert_eq!(
            qemu_img(&["compare", "-f", "vhdx", "-F", "raw", o, ds]),
            Some(true)
        );
    }
    std::fs::remove_file(&dev).ok();
    std::fs::remove_file(&out).ok();
}

#[test]
fn dynamic_vhd_and_qemu_vhdx_read() {
    let (raw, vhd, vhdx) = (tmp("q.raw"), tmp("q.vhd"), tmp("q.vhdx"));
    let mut d = vec![0u8; 9 << 20];
    d[..1000].fill(0xab);
    d[(5 << 20) + 7..(6 << 20) + 9]
        .iter_mut()
        .enumerate()
        .for_each(|(i, b)| *b = i as u8);
    std::fs::write(&raw, &d).unwrap();
    let r = raw.to_str().unwrap();
    if qemu_img(&[
        "convert",
        "-f",
        "raw",
        "-O",
        "vpc",
        "-o",
        "subformat=dynamic,force_size=on",
        r,
        vhd.to_str().unwrap(),
    ]) == Some(true)
    {
        assert!(read_all(&vhd) == d);
        assert_eq!(
            qemu_img(&[
                "convert",
                "-f",
                "raw",
                "-O",
                "vhdx",
                r,
                vhdx.to_str().unwrap()
            ]),
            Some(true)
        );
        assert!(read_all(&vhdx) == d);
    } else {
        // Hand-made dynamic VHD: 2 MiB disk, 1 MiB blocks, only block 1 allocated.
        let mut f = vhd_footer(2 << 20);
        f[16..24].copy_from_slice(&512u64.to_be_bytes());
        f[60..64].copy_from_slice(&3u32.to_be_bytes());
        let mut v = f.to_vec();
        let mut dh = vec![0u8; 1024];
        dh[..8].copy_from_slice(b"cxsparse");
        dh[16..24].copy_from_slice(&1536u64.to_be_bytes());
        dh[28..32].copy_from_slice(&2u32.to_be_bytes());
        dh[32..36].copy_from_slice(&(1u32 << 20).to_be_bytes());
        v.extend_from_slice(&dh);
        let mut bat = vec![0xffu8; 512];
        bat[4..8].copy_from_slice(&4u32.to_be_bytes()); // sector 4 = byte 2048
        v.extend_from_slice(&bat);
        v.extend_from_slice(&[0u8; 512]); // bitmap
        v.extend((0..1u32 << 20).map(|i| (i % 7) as u8));
        v.extend_from_slice(&f);
        std::fs::write(&vhd, &v).unwrap();
        let got = read_all(&vhd);
        assert!(
            got[..1 << 20].iter().all(|&b| b == 0) && got[1 << 20..] == v[2560..2560 + (1 << 20)]
        );
    }
    for p in [raw, vhd, vhdx] {
        std::fs::remove_file(&p).ok();
    }
}

#[test]
fn ffu_needs_windows() {
    assert!(is_ffu(Path::new("a.FFU")) && !is_ffu(Path::new("a.vhdx")));
    #[cfg(not(windows))]
    assert!(apply_ffu(Path::new("a.ffu"), "/dev/null").is_err());
}

#[test]
fn save_vhd_roundtrip() {
    let (dev, out) = (tmp("dev.bin"), tmp("out.vhd"));
    let d: Vec<u8> = (0..(5u32 << 20) + 1024).map(|i| (i % 253) as u8).collect();
    std::fs::write(&dev, &d).unwrap();
    save_image(
        dev.to_str().unwrap(),
        d.len() as u64,
        &out,
        &|_| {},
        &AtomicBool::new(false),
    )
    .unwrap();
    let v = std::fs::read(&out).unwrap();
    assert_eq!(v.len(), d.len() + 512);
    let mut footer = v[d.len()..].to_vec();
    assert_eq!(&footer[..8], b"conectix");
    let stored = u32::from_be_bytes(footer[64..68].try_into().unwrap());
    footer[64..68].fill(0);
    assert_eq!(stored, !footer.iter().map(|&b| b as u32).sum::<u32>());
    assert_eq!(roundtrip("out2.vhd", &v), d);
    assert_eq!(vhd_geometry(4 << 30), (8322, 16, 63));
    std::fs::remove_file(&dev).ok();
    std::fs::remove_file(&out).ok();
}
