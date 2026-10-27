use super::*;

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

#[test]
fn parses_lists() {
    let out = "Please select a Windows Version (-Win):\n - Windows 11\n - Windows 10\n - UEFI Shell 2.2\n";
    assert_eq!(
        parse_list(out).unwrap(),
        s(&["Windows 11", "Windows 10", "UEFI Shell 2.2"])
    );
    let out = "Please select a Windows Release (-Rel) for Windows 11 (or use 'Latest' for most recent):\r\n - 26H2 (Build 26300.9457 - 2026.09)\r\n";
    assert_eq!(
        parse_list(out).unwrap(),
        s(&["26H2 (Build 26300.9457 - 2026.09)"])
    );
    assert_eq!(
        parse_list("Error: This feature is not available on this platform.\n").unwrap_err(),
        "This feature is not available on this platform."
    );
    assert_eq!(
        parse_list("Invalid Windows version provided.\nUse '-Win List'...\n").unwrap_err(),
        "Invalid Windows version provided."
    );
}

#[test]
fn parses_url() {
    let u =
        "https://software.download.prss.microsoft.com/dbazure/Win11_26H2_English_x64.iso?t=abc&e=1";
    assert_eq!(parse_url(&format!("{u}\n")).unwrap(), u);
    assert_eq!(
        parse_url("Error: Your IP address has been banned\n").unwrap_err(),
        "Your IP address has been banned"
    );
}

#[test]
fn builds_args() {
    let a = args(&s(&[
        "Windows 11",
        "26H2 (Build 26300.9457 - 2026.09)",
        "Windows 11 Home/Pro/Edu",
        "English (United States)",
        "x64",
    ]))
    .unwrap();
    assert_eq!(
        a,
        s(&[
            "-Win",
            r"^Windows 11\s*$",
            "-Rel",
            "26H2",
            "-Ed",
            r"^Windows 11 Home/Pro/Edu\s*$",
            "-Lang",
            r"^English (United States)\s*$",
            "-Arch",
            r"^x64\s*$"
        ])
    );
    assert_eq!(
        args(&s(&["UEFI Shell 2.2"])).unwrap()[1],
        r"^UEFI Shell 2\.2\s*$"
    );
    for bad in ["", "a;b", "$(x)", "a\"b", "a`b", "a|b", "a\nb", "a[0]", "*"] {
        assert!(args(&s(&[bad])).is_err(), "{bad:?}");
    }
    assert!(args(&s(&["a"; 6])).is_err());
}
