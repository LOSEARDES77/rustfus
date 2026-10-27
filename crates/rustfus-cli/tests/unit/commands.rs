use super::*;

#[test]
fn picks_fido_defaults() {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    assert_eq!(
        default_pick(3, &s(&["Arabic", "English", "Spanish"])),
        "English"
    );
    assert_eq!(
        default_pick(0, &s(&["Windows 11", "Windows 10"])),
        "Windows 11"
    );
    assert_eq!(default_pick(3, &s(&["English (US)"])), "English (US)");
}
