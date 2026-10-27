use super::*;

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
