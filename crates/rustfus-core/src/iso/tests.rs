use super::*;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rustfus-iso-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(d.join("src")).unwrap();
    d
}

/// Builds `dir/out.iso` from `dir/src`; None if xorriso is missing.
fn mkiso(dir: &Path, args: &[&str]) -> Option<Iso> {
    let iso = dir.join("out.iso");
    let st = Command::new("xorriso")
        .args(["-as", "mkisofs", "-quiet", "-o"])
        .arg(&iso)
        .args(args)
        .arg(dir.join("src"))
        .stderr(std::process::Stdio::null())
        .status()
        .ok()?;
    assert!(st.success());
    Some(Iso::open(&iso).unwrap())
}

fn check(iso: &Iso, src: &Path) -> Vec<Entry> {
    let es = iso.entries().unwrap();
    for (i, e) in es.iter().enumerate() {
        if let Some((parent, _)) = e.path.rsplit_once('/') {
            assert!(
                es[..i].iter().any(|p| p.is_dir && p.path == parent),
                "{} before parent",
                e.path
            );
        }
        let p = src.join(&e.path);
        if e.is_dir {
            assert!(p.is_dir(), "{}", e.path);
        } else if e.symlink().is_none() && e.path != "boot.catalog" {
            let mut got = Vec::new();
            iso.reader(e).unwrap().read_to_end(&mut got).unwrap();
            assert_eq!(got, fs::read(&p).unwrap(), "{}", e.path);
            assert_eq!(e.size, got.len() as u64);
        }
    }
    es
}

fn populate(src: &Path) -> String {
    let long = "A Very Long Mixed-Case File Name That Exceeds Joliet Limits Maybe.txt".to_string();
    fs::create_dir_all(src.join("EFI/Boot/deep/er")).unwrap();
    fs::write(src.join(&long), b"long").unwrap();
    fs::write(src.join("EFI/Boot/bootx64.efi"), vec![7u8; 5000]).unwrap();
    fs::write(src.join("EFI/Boot/deep/er/x.cfg"), b"").unwrap();
    long
}

#[test]
fn iso_rock_ridge() {
    let d = tmp("rr");
    let long = populate(&d.join("src"));
    #[cfg(unix)]
    std::os::unix::fs::symlink("EFI/Boot/bootx64.efi", d.join("src/link")).unwrap();
    let Some(iso) = mkiso(&d, &["-R", "-V", "RR_TEST"]) else {
        return;
    };
    assert_eq!(iso.volume_id(), "RR_TEST");
    let es = check(&iso, &d.join("src"));
    let paths: Vec<&str> = es.iter().map(|e| e.path.as_str()).collect();
    assert!(
        paths.contains(&long.as_str()) && paths.contains(&"EFI/Boot/deep/er/x.cfg"),
        "{paths:?}"
    );
    #[cfg(unix)]
    assert_eq!(
        es.iter().find(|e| e.path == "link").unwrap().symlink(),
        Some("EFI/Boot/bootx64.efi")
    );
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn iso_joliet() {
    let d = tmp("joliet");
    populate(&d.join("src"));
    let Some(iso) = mkiso(&d, &["--norock", "-J", "-joliet-long"]) else {
        return;
    };
    assert_eq!(iso.names as u8, Names::Joliet as u8);
    let es = check(&iso, &d.join("src"));
    assert!(es.iter().any(|e| e.path == "EFI/Boot/bootx64.efi"));
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn iso_el_torito() {
    let d = tmp("boot");
    fs::write(d.join("src/boot.img"), vec![1u8; 2048]).unwrap();
    fs::write(d.join("src/efi.img"), vec![2u8; 4096]).unwrap();
    let efi = [
        "-R",
        "-b",
        "boot.img",
        "-no-emul-boot",
        "-eltorito-alt-boot",
        "-e",
        "efi.img",
        "-no-emul-boot",
    ];
    let Some(iso) = mkiso(&d, &efi) else { return };
    let et = iso.el_torito();
    assert!(et.bios && et.efi, "{et:?}");
    check(&iso, &d.join("src"));
    fs::remove_file(d.join("out.iso")).unwrap();
    let iso = mkiso(&d, &efi[..4]).unwrap();
    let et = iso.el_torito();
    assert!(et.bios && !et.efi, "{et:?}");
    let _ = fs::remove_dir_all(&d);
}

fn rec(ext: u32, len: u32, flags: u8, id: &[u8]) -> Vec<u8> {
    let mut r = vec![0u8; 33];
    r[2..6].copy_from_slice(&ext.to_le_bytes());
    r[10..14].copy_from_slice(&len.to_le_bytes());
    (r[25], r[32]) = (flags, id.len() as u8);
    r.extend(id);
    if r.len() % 2 == 1 {
        r.push(0);
    }
    r[0] = r.len() as u8;
    r
}

// xorriso can't split small files into multiple extents, so synthesize one.
#[test]
fn iso_multi_extent_and_malformed() {
    let mut img = vec![0u8; 21 * 2048];
    let pvd = &mut img[16 * 2048..17 * 2048];
    (pvd[0], pvd[6]) = (1, 1);
    pvd[1..6].copy_from_slice(b"CD001");
    pvd[40..72].copy_from_slice(&[b' '; 32]);
    pvd[40..44].copy_from_slice(b"MULT");
    pvd[156..190].copy_from_slice(&rec(18, 2048, 2, &[0]));
    img[17 * 2048] = 255;
    img[17 * 2048 + 1..17 * 2048 + 6].copy_from_slice(b"CD001");
    let dir: Vec<u8> = [
        rec(18, 2048, 2, &[0]),
        rec(18, 2048, 2, &[1]),
        rec(19, 2048, 0x80, b"BIG.BIN;1"),
        rec(20, 100, 0, b"BIG.BIN;1"),
        rec(20, 1, 4, b"ASSOC.;1"),
    ]
    .concat();
    img[18 * 2048..18 * 2048 + dir.len()].copy_from_slice(&dir);
    img[19 * 2048..20 * 2048].fill(0xaa);
    img[20 * 2048..20 * 2048 + 100].fill(0xbb);
    let d = tmp("multi");
    let p = d.join("m.iso");
    fs::write(&p, &img).unwrap();
    let iso = Iso::open(&p).unwrap();
    assert_eq!(iso.volume_id(), "MULT");
    let es = iso.entries().unwrap();
    assert_eq!(es.len(), 1);
    assert_eq!((es[0].path.as_str(), es[0].size), ("BIG.BIN", 2148));
    let mut got = Vec::new();
    iso.reader(&es[0]).unwrap().read_to_end(&mut got).unwrap();
    assert!(
        got.len() == 2148
            && got[..2048].iter().all(|&b| b == 0xaa)
            && got[2048..].iter().all(|&b| b == 0xbb)
    );
    let mut buf = [0u8; 8];
    assert_eq!(iso.read_at(&es[0], 2044, &mut buf).unwrap(), 8);
    assert_eq!(buf, [0xaa, 0xaa, 0xaa, 0xaa, 0xbb, 0xbb, 0xbb, 0xbb]);
    assert_eq!(iso.read_at(&es[0], 2100, &mut buf).unwrap(), 8);
    assert_eq!(buf, [0xbb; 8]);
    assert_eq!(iso.read_at(&es[0], 2144, &mut buf).unwrap(), 4);
    assert_eq!(iso.read_at(&es[0], 9999, &mut buf).unwrap(), 0);
    fs::write(&p, &img[..19 * 2048 + 10]).unwrap();
    let iso = Iso::open(&p).unwrap();
    let mut got = Vec::new();
    assert!(
        iso.reader(&iso.entries().unwrap()[0])
            .unwrap()
            .read_to_end(&mut got)
            .is_err()
    );
    img[18 * 2048 + 100] = 250;
    fs::write(&p, &img).unwrap();
    assert_eq!(
        Iso::open(&p).unwrap().entries().unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    fs::write(&p, &img[..17 * 2048]).unwrap();
    assert!(Iso::open(&p).is_err());
    let _ = fs::remove_dir_all(&d);
}

fn sorted(es: &[Entry]) -> Vec<(String, bool, u64)> {
    let mut v: Vec<_> = es
        .iter()
        .map(|e| (e.path.clone(), e.is_dir, e.size))
        .collect();
    v.sort();
    v
}

#[test]
fn iso_udf_genisoimage() {
    let d = tmp("udfgen");
    let src = d.join("src");
    let long = populate(&src);
    let uni = "ünïcødé ✓ 名前 with a rather long name to exceed 64 chars of joliet.bin";
    fs::create_dir_all(src.join("sources/Ünï")).unwrap();
    fs::write(
        src.join("sources").join(uni),
        (0..3_000_000u32)
            .map(|i| (i % 251) as u8)
            .collect::<Vec<_>>(),
    )
    .unwrap();
    fs::write(src.join("sources/Ünï/empty"), b"").unwrap();
    let out = d.join("out.iso");
    let args = [
        "-quiet",
        "-udf",
        "-R",
        "-input-charset",
        "utf-8",
        "-V",
        "GEN_UDF",
        "-o",
    ];
    let Ok(st) = Command::new("genisoimage")
        .args(args)
        .arg(&out)
        .arg(&src)
        .stderr(std::process::Stdio::null())
        .status()
    else {
        return;
    };
    assert!(st.success());
    let udf = Iso::open_udf(File::open(&out).unwrap()).unwrap();
    assert!(udf.udf.is_some());
    assert_eq!(udf.volume_id(), "GEN_UDF");
    let es = check(&udf, &src);
    assert!(
        es.iter().any(|e| e.path == long) && es.iter().any(|e| e.path == format!("sources/{uni}")),
        "{es:?}"
    );
    let iso = Iso::open(&out).unwrap();
    assert!(iso.udf.is_none());
    assert_eq!(sorted(&es), sorted(&iso.entries().unwrap()));
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn iso_udf_mkudffs() {
    let d = tmp("mkudffs");
    let src = d.join("src");
    populate(&src);
    let out = d.join("out.udf");
    fs::write(&out, vec![0u8; 4 << 20]).unwrap();
    let ok = Command::new("mkudffs")
        .args(["--blocksize=2048", "--label=MKUDF"])
        .arg(&out)
        .stdout(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        return; // mkudffs not installed
    }
    let iso = Iso::open(&out).unwrap();
    assert_eq!(
        (iso.volume_id().as_str(), iso.entries().unwrap().len()),
        ("MKUDF", 0)
    );
    let _ = fs::remove_dir_all(&d);
}

fn set_tag(b: &mut [u8], id: u16) {
    b[..2].copy_from_slice(&id.to_le_bytes());
    b[4] = (0..16)
        .filter(|&i| i != 4)
        .fold(0u8, |s, i| s.wrapping_add(b[i]));
}

fn put(b: &mut [u8], o: usize, v: &[u8]) {
    b[o..o + v.len()].copy_from_slice(v);
}

fn fid(ch: u8, name: &[u8], lbn: u32) -> Vec<u8> {
    let mut f = vec![0u8; 38];
    (f[18], f[19]) = (ch, name.len() as u8);
    put(&mut f, 20, &2048u32.to_le_bytes());
    put(&mut f, 24, &lbn.to_le_bytes());
    f.extend(name);
    f.resize(f.len().div_ceil(4) * 4, 0);
    set_tag(&mut f, 257);
    f
}

fn fe(id: u16, kind: u8, flags: u16, size: u64, ad: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; 2048];
    let hdr = if id == 261 { 176 } else { 216 };
    b[27] = kind;
    put(&mut b, 34, &flags.to_le_bytes());
    put(&mut b, 56, &size.to_le_bytes());
    put(&mut b, hdr - 4, &(ad.len() as u32).to_le_bytes());
    put(&mut b, hdr, ad);
    set_tag(&mut b, id);
    b
}

/// Tiny UDF-only image: embedded dir/file data, EFE + short_ad, long_ad + unrecorded extent, symlink, 16-bit name, loop.
fn udf_image() -> Vec<u8> {
    const P: usize = 300;
    let mut img = vec![0u8; (P + 8) * 2048];
    for (s, id) in [(16, b"BEA01"), (17, b"NSR02"), (18, b"TEA01")] {
        put(&mut img, s * 2048 + 1, id);
    }
    let sec = |img: &mut Vec<u8>, s: usize, b: &[u8]| put(img, s * 2048, b);
    let mut b = vec![0u8; 2048];
    put(&mut b, 16, &(4 * 2048u32).to_le_bytes());
    put(&mut b, 20, &32u32.to_le_bytes());
    set_tag(&mut b, 2);
    sec(&mut img, 256, &b);
    let mut b = vec![0u8; 2048];
    put(&mut b, 24, b"\x08PVDNAME");
    b[55] = 8;
    set_tag(&mut b, 1);
    sec(&mut img, 32, &b);
    let mut b = vec![0u8; 2048];
    put(&mut b, 188, &(P as u32).to_le_bytes());
    set_tag(&mut b, 5);
    sec(&mut img, 33, &b);
    let mut b = vec![0u8; 2048];
    put(&mut b, 84, b"\x08UDF_ONLY");
    b[211] = 9;
    put(&mut b, 212, &2048u32.to_le_bytes());
    put(&mut b, 248, &2048u32.to_le_bytes());
    put(&mut b, 264, &6u32.to_le_bytes());
    put(&mut b, 268, &1u32.to_le_bytes());
    put(&mut b, 440, &[1, 6, 1, 0, 0, 0]);
    set_tag(&mut b, 6);
    sec(&mut img, 34, &b);
    let mut b = vec![0u8; 2048];
    set_tag(&mut b, 8);
    sec(&mut img, 35, &b);
    let mut b = vec![0u8; 2048];
    put(&mut b, 404, &1u32.to_le_bytes());
    set_tag(&mut b, 256);
    sec(&mut img, P, &b);
    let dir16: Vec<u8> = std::iter::once(16)
        .chain("Dïr✓".encode_utf16().flat_map(u16::to_be_bytes))
        .collect();
    let root = [
        fid(10, b"", 1),
        fid(2, &dir16, 2),
        fid(0, b"\x08a.txt", 3),
        fid(4, b"\x08gone", 3),
    ]
    .concat();
    sec(&mut img, P + 1, &fe(261, 4, 3, root.len() as u64, &root));
    let sub = [
        fid(10, b"", 1),
        fid(0, b"\x08big.bin", 5),
        fid(0, b"\x08ln", 7),
        fid(2, b"\x08loop", 1),
    ]
    .concat();
    let short = [(sub.len() as u32).to_le_bytes(), 4u32.to_le_bytes()].concat();
    sec(&mut img, P + 2, &fe(266, 4, 0, sub.len() as u64, &short));
    sec(&mut img, P + 3, &fe(261, 5, 3, 5, b"hello"));
    sec(&mut img, P + 4, &sub);
    let mut long = vec![0u8; 32];
    put(&mut long, 0, &2048u32.to_le_bytes());
    put(&mut long, 4, &6u32.to_le_bytes());
    put(&mut long, 16, &(100u32 | 1 << 30).to_le_bytes());
    sec(&mut img, P + 5, &fe(261, 5, 1, 2148, &long));
    sec(&mut img, P + 6, &[0xcc; 2048]);
    let link = [&[2u8, 0, 0, 0][..], &[5, 6, 0, 0], b"\x08a.txt"].concat();
    sec(&mut img, P + 7, &fe(261, 12, 3, link.len() as u64, &link));
    img
}

fn exercise(p: &Path) {
    if let Ok(iso) = Iso::open(p) {
        for e in iso.entries().unwrap_or_default() {
            if let Ok(r) = iso.reader(&e) {
                let _ = io::copy(&mut r.take(1 << 20), &mut io::sink());
            }
        }
    }
}

#[test]
fn iso_udf_only_and_malformed() {
    let mut img = udf_image();
    let d = tmp("udfonly");
    let p = d.join("u.img");
    fs::write(&p, &img).unwrap();
    let iso = Iso::open(&p).unwrap();
    assert_eq!(iso.volume_id(), "UDF_ONLY");
    assert!(!iso.el_torito().bios && !iso.el_torito().efi);
    let es = iso.entries().unwrap();
    let got: Vec<_> = es
        .iter()
        .map(|e| (e.path.as_str(), e.is_dir, e.size, e.symlink()))
        .collect();
    let want = [
        ("Dïr✓", true, 0, None),
        ("a.txt", false, 5, None),
        ("Dïr✓/big.bin", false, 2148, None),
        ("Dïr✓/ln", false, 0, Some("/a.txt")),
        ("Dïr✓/loop", true, 0, None),
    ];
    assert_eq!(got, want);
    let read = |e: &Entry| {
        let mut v = Vec::new();
        iso.reader(e).unwrap().read_to_end(&mut v).unwrap();
        v
    };
    assert_eq!(read(&es[1]), b"hello");
    let big = read(&es[2]);
    assert!(
        big.len() == 2148
            && big[..2048].iter().all(|&b| b == 0xcc)
            && big[2048..].iter().all(|&b| b == 0)
    );
    let mut buf = [1u8; 4];
    assert_eq!(
        (iso.read_at(&es[2], 2046, &mut buf).unwrap(), buf),
        (4, [0xcc, 0xcc, 0, 0])
    );
    assert_eq!(
        (iso.read_at(&es[1], 3, &mut buf).unwrap(), &buf[..2]),
        (2, &b"lo"[..])
    );
    // Malformed: broken FID name length, bad root ICB tag, bad AVDP.
    let fid0 = 300 * 2048 + 2048 + 176;
    img[fid0 + 40 + 19] = 200;
    fs::write(&p, &img).unwrap();
    assert_eq!(
        Iso::open(&p).unwrap().entries().unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    let mut img = udf_image();
    img[301 * 2048] = 7;
    fs::write(&p, &img).unwrap();
    assert_eq!(
        Iso::open(&p).unwrap().entries().unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    let mut img = udf_image();
    img[256 * 2048 + 4] ^= 1;
    fs::write(&p, &img).unwrap();
    assert_eq!(
        Iso::open(&p).err().unwrap().kind(),
        io::ErrorKind::InvalidData
    );
    // Truncations and single-byte corruptions must not panic or hang.
    let img = udf_image();
    for len in [0, 100 * 2048, 257 * 2048, 301 * 2048 + 500, 305 * 2048] {
        fs::write(&p, &img[..len]).unwrap();
        exercise(&p);
    }
    fs::write(&p, &img).unwrap();
    let mut f = fs::OpenOptions::new().write(true).open(&p).unwrap();
    for s in [256, 32, 33, 34, 300, 301, 302, 303, 304, 305, 307] {
        for o in (0..400).chain(2040..2048) {
            let at = (s * 2048 + o) as u64;
            for v in [img[at as usize] ^ 0xff, 0x80] {
                use std::io::{Seek, SeekFrom, Write};
                f.seek(SeekFrom::Start(at)).unwrap();
                f.write_all(&[v]).unwrap();
                exercise(&p);
                f.seek(SeekFrom::Start(at)).unwrap();
                f.write_all(&[img[at as usize]]).unwrap();
            }
        }
    }
    let _ = fs::remove_dir_all(&d);
}

/// RUSTFUS_ISO=/path/to.iso cargo test iso_real -- --ignored --nocapture
#[test]
#[ignore]
fn iso_real() {
    let iso = Iso::open(Path::new(&std::env::var("RUSTFUS_ISO").unwrap())).unwrap();
    let es = iso.entries().unwrap();
    let total: u64 = es
        .iter()
        .filter(|e| !e.is_dir)
        .map(|e| io::copy(&mut iso.reader(e).unwrap(), &mut io::sink()).unwrap())
        .sum();
    assert_eq!(total, es.iter().map(|e| e.size).sum::<u64>());
    println!(
        "{} {:?} {} entries {total} bytes {:?}",
        iso.volume_id(),
        iso.el_torito(),
        es.len(),
        &es[..es.len().min(5)]
    );
}
