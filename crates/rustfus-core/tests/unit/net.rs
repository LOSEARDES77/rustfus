use super::*;

#[test]
fn rejects_bad_signature() {
    assert!(!verify(b"data", &[0u8; 256]));
    assert!(!verify(b"data", b"short"));
}

/// Network test: run with `cargo test net_live -- --ignored`.
#[test]
fn net_live_grub_core_img_is_signed() {
    let url = format!("{FILES_URL}/grub-2.06/core.img");
    let data = fetch(&url).unwrap();
    let sig = fetch(&format!("{url}.sig")).unwrap();
    assert!(verify(&data, &sig));
    let mut tampered = data.clone();
    tampered[100] ^= 1;
    assert!(!verify(&tampered, &sig));
}
