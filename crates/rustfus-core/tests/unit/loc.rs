use super::*;

// CURRENT is global: keep language-switching assertions in one test.
#[test]
fn loc_translations() {
    let langs = languages();
    assert_eq!(langs[0].code, "en-US");
    assert!(langs.len() >= 30);
    assert!(
        langs
            .iter()
            .any(|l| l.code == "fr-FR" && l.name == "French (Français)")
    );
    set_language("en-US");
    assert_eq!(t("IDS_DEVICE_TXT"), "Device");
    assert_eq!(
        t("MSG_002"),
        "Another Rufus application is running.\nPlease close the first application before running another one."
    );
    assert_eq!(t("NO_SUCH_ID"), "NO_SUCH_ID");
    set_language("fr-FR");
    assert_eq!(current(), "fr-FR");
    assert_eq!(t("IDS_DEVICE_TXT"), "Périphérique");
    set_language("xx-XX");
    assert_eq!(current(), "en-US");
}

#[test]
fn loc_fallback_to_en_us() {
    let p = parse(
        "l \"en-US\" \"English\" 0x0409\nt A \"en a\"\nt B \"en b\"\nl \"fr-FR\" \"French\" 0x040c\nb \"en-US\"\nt A \"fr a\"\n",
    );
    assert_eq!(p.strings[1].get("A").unwrap(), "fr a");
    assert!(!p.strings[1].contains_key("B"));
}

#[test]
fn loc_printf() {
    assert_eq!(printf("%d of %s (%%)", &["3", "x"]), "3 of x (%)");
    assert_eq!(printf("0x%02X (Disk %d)", &["10", "2"]), "0x10 (Disk 2)");
    assert_eq!(printf("%0.2f%% %s", &["1.5"]), "1.5% ");
    assert_eq!(strip_amp("&Start && go"), "Start & go");
    assert_eq!(tf("NO_SUCH_%s", &[]), "NO_SUCH_");
}

#[test]
fn loc_system_mapping() {
    let codes = [
        "en-US", "fr-FR", "pt-BR", "pt-PT", "zh-CN", "zh-TW", "nb-NO", "es-ES",
    ];
    assert_eq!(closest("fr-FR", &codes), "fr-FR");
    assert_eq!(closest("fr-CA", &codes), "fr-FR");
    assert_eq!(closest("pt-PT", &codes), "pt-PT");
    assert_eq!(closest("pt-AO", &codes), "pt-BR");
    assert_eq!(closest("es-MX", &codes), "es-ES");
    assert_eq!(closest("zh-HK", &codes), "zh-TW");
    assert_eq!(closest("zh-Hans-CN", &codes), "zh-CN");
    assert_eq!(closest("nn-NO", &codes), "nb-NO");
    assert_eq!(closest("en-GB", &codes), "en-US");
    assert_eq!(closest("xx-YY", &codes), "en-US");
    assert_eq!(closest("en-US", &codes), "en-US");
}
