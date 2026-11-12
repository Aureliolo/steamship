//! `steamship languages`.

use crate::steamship;

#[test]
fn every_locale_is_listed_with_steams_name_for_its_language() {
    let (as_json, listed, told) = steamship(&["languages", "--json"], None, &[]);
    assert_eq!(as_json, Some(0_i32), "{told}");
    let table: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&listed).unwrap();
    assert_eq!(table.len(), 34);
    for (locale, language) in [
        ("pt-BR", "brazilian"),
        ("zh-Hans", "schinese"),
        ("es-419", "latam"),
        ("ko", "koreana"),
    ] {
        assert_eq!(
            table.get(locale).and_then(|named| named.as_str()),
            Some(language)
        );
    }

    let (code, shown, why) = steamship(&["languages"], None, &[]);
    assert_eq!(code, Some(0_i32), "{why}");
    assert!(shown.contains("  pt-BR     brazilian\n"), "{shown}");
}
