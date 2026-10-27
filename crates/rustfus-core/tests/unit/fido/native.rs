use super::*;

#[test]
fn makes_uuid() {
    let u = uuid();
    assert_eq!(u.len(), 36);
    assert_eq!(u.as_bytes()[14], b'4');
    assert_ne!(u, uuid());
}
