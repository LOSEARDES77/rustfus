use super::*;

#[test]
fn parses_wim_xml() {
    let xml = "\u{feff}<WIM><TOTALBYTES>1</TOTALBYTES><IMAGE INDEX=\"1\"><NAME>Windows 11 Home</NAME></IMAGE>\
               <IMAGE INDEX=\"6\"><NAME>Windows 11 Pro</NAME><DISPLAYNAME>Windows 11 Pro (display)</DISPLAYNAME></IMAGE></WIM>";
    let bytes: Vec<u8> = xml.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    assert_eq!(
        parse_editions(&bytes),
        vec![
            (1, "Windows 11 Home".into()),
            (6, "Windows 11 Pro (display)".into())
        ]
    );
}

#[test]
fn detects_windows_11_from_client_build_or_name_but_not_server() {
    let xml = |body: &str| {
        body.encode_utf16()
            .flat_map(|unit| unit.to_le_bytes())
            .collect::<Vec<_>>()
    };
    assert!(super::windows_11_metadata(&xml(
        "<WIM><IMAGE><NAME>Windows 11 Pro</NAME></IMAGE></WIM>"
    )));
    assert!(super::windows_11_metadata(&xml(
        "<WIM><IMAGE><WINDOWS><INSTALLATIONTYPE>Client</INSTALLATIONTYPE><VERSION><MAJOR>10</MAJOR><BUILD>22631</BUILD></VERSION></WINDOWS></IMAGE></WIM>"
    )));
    assert!(!super::windows_11_metadata(&xml(
        "<WIM><IMAGE><NAME>Windows Server 2025</NAME><WINDOWS><INSTALLATIONTYPE>Server</INSTALLATIONTYPE><VERSION><MAJOR>10</MAJOR><BUILD>26100</BUILD></VERSION></WINDOWS></IMAGE></WIM>"
    )));
    assert!(!super::windows_11_metadata(&xml(
        "<WIM><IMAGE><NAME>Windows 10 Pro</NAME><WINDOWS><INSTALLATIONTYPE>Client</INSTALLATIONTYPE><VERSION><MAJOR>10</MAJOR><BUILD>19045</BUILD></VERSION></WINDOWS></IMAGE></WIM>"
    )));
}

#[test]
fn malformed_or_odd_utf16_metadata_is_conservative() {
    assert!(!super::windows_11_metadata(b"not-utf16"));
    assert!(!super::windows_11_metadata(&[0xff, 0xff, 0x00, 0xd8]));
    assert!(!super::windows_11_metadata(
        &"<WIM><IMAGE><NAME>Windows 11</NAME>"
            .encode_utf16()
            .flat_map(|unit| unit.to_le_bytes())
            .collect::<Vec<_>>()
    ));
}
