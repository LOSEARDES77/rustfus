use super::*;

#[test]
fn finds_token() {
    let ver = "# comment\nz0 = https://old\nz1 = https://github.com/pbatard/Fido/releases/download/v1.71/Fido.ps1.lzma\n";
    assert_eq!(
        token(ver, "z1"),
        Some("https://github.com/pbatard/Fido/releases/download/v1.71/Fido.ps1.lzma")
    );
    assert_eq!(token(ver, "z2"), None);
}

#[test]
fn names_iso_from_url() {
    assert_eq!(
        file_name("https://x/dbazure/Win11_26H2_English_x64.iso?t=abc&e=1"),
        "Win11_26H2_English_x64.iso"
    );
    assert_eq!(file_name("https://x/"), "windows.iso");
}

fn walk(list: impl Fn(&[String]) -> Result<Vec<String>, String>, wants: [&str; 5]) -> Vec<String> {
    let mut chosen = Vec::new();
    for want in wants {
        let opts = list(&chosen).unwrap();
        let pick = opts
            .iter()
            .find(|o| want.is_empty() || o.as_str() == want)
            .unwrap_or_else(|| panic!("{want:?} not in {opts:?}"));
        chosen.push(pick.clone());
    }
    chosen
}

/// Network test of the native queries: `cargo test fido_native_live -- --ignored`.
#[test]
#[ignore]
fn fido_native_live() {
    let c = walk(
        native::list,
        [
            "Windows 11",
            "",
            "Windows 11 Home/Pro/Edu",
            "English",
            "x64",
        ],
    );
    let u = native::url(&c).unwrap();
    assert!(u.starts_with("https://") && u.contains(".iso"), "{u}");
    let c = walk(native::list, ["UEFI Shell 2.2", "", "", "", ""]);
    assert!(native::url(&c).unwrap().ends_with("-RELEASE.iso"));
}

/// Network test through PowerShell, when installed: `cargo test fido_pwsh_live -- --ignored`.
#[test]
fn fido_pwsh_live() {
    let Some(sh) = pwsh::shell() else { return };
    let c = walk(|c| pwsh::list(&sh, c), ["UEFI Shell 2.2", "", "", "", ""]);
    assert!(pwsh::url(&sh, &c).unwrap().ends_with("-RELEASE.iso"));
}
