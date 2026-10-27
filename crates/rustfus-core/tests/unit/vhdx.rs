use super::*;

#[test]
fn crc32c_check_value() {
    assert_eq!(crc32c(b"123456789"), 0xe306_9283);
    assert_eq!(guid(BAT_GUID)[..4], [0x66, 0x77, 0xc2, 0x2d]);
}
