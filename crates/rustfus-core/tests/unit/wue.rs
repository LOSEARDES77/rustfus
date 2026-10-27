use super::*;

#[test]
fn xml_contains_selected_options() {
    let o = Options {
        bypass_hw_checks: true,
        no_online_account: true,
        local_account: Some("me<&>".into()),
        regional: false,
        no_data_collection: true,
        no_bitlocker: true,
        wtg_no_internal: false,
    };
    let x = unattend_xml(&o, "amd64");
    assert!(
        x.contains("BypassTPMCheck")
            && x.contains("BypassNRO")
            && x.contains("PreventDeviceEncryption")
    );
    assert!(x.contains("<ProtectYourPC>3</ProtectYourPC>"));
    assert!(x.contains("<Name>me&lt;&amp;&gt;</Name>"));
    assert!(!x.contains("me<&>"));
    assert_eq!(
        x.matches("<settings").count(),
        x.matches("</settings>").count()
    );
    assert!(!unattend_xml(&Options::default(), "amd64").contains("<settings"));
}
