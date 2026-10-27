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
