use super::*;

#[test]
fn parses_version_table() {
    let ps1 = r#"
$WindowsVersions = @(
	@(
		@("Windows 11", "windows11"),
		@(
			"26H2 (Build 26300.9457 - 2026.09)",
			# a comment, with "quotes" and @(parens)
			@("Windows 11 Home/Pro/Edu", @(3813, 3816)),
			@("Windows 11 Home China ", @(3814, 3817))
		)
	),
	@(
		@("Windows 10", "Windows10ISO"),
		@(
			"22H2 v1 (Build 19045.2965 - 2023.05)",
			@("Windows 10 Home/Pro/Edu", 2618)
		)
	)
	@(
		@("UEFI Shell 2.2", "UEFI_SHELL 2.2"),
		@(
			"26H1 (edk2-stable202602)",
			@("Release", 0),
			@("Debug", 1)
		)
	)
)
#endregion
"#;
    let v = parse_versions(ps1).unwrap();
    assert_eq!(v.len(), 3);
    assert_eq!(v[0].name, "Windows 11");
    assert_eq!(
        v[0].releases[0].editions[0],
        ("Windows 11 Home/Pro/Edu".into(), vec![3813, 3816])
    );
    assert_eq!(v[0].releases[0].editions[1].0, "Windows 11 Home China");
    assert_eq!(v[1].releases[0].editions[0].1, vec![2618]);
    assert_eq!(v[2].page, "UEFI_SHELL 2.2");
    assert_eq!(v[2].releases[0].editions[1], ("Debug".into(), vec![1]));
    assert!(parse_versions("$WindowsVersions = @( @( @(\"a\", \"b\") ").is_err());
}
