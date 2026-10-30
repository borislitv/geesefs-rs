use std::collections::BTreeMap;

use geesefs_rs::backend::metadata::{Metadata, decode_metadata, encode_metadata};

#[test]
fn reference_percent_decoding_lowercases_before_unescape() {
    let meta = decode_metadata([
        ("Custom%2BKey", "literal+plus%00%25"),
        ("MTIME", "1700000000"),
        ("empty", ""),
        ("bad%", "omit"),
        ("badvalue", "%zz"),
        ("binary", "%FF"),
    ]);
    assert_eq!(
        meta,
        BTreeMap::from([
            ("custom+key".into(), b"literal+plus\0%".to_vec()),
            ("mtime".into(), b"1700000000".to_vec()),
            ("empty".into(), vec![]),
            ("binary".into(), vec![255]),
        ])
    );
}

#[test]
fn ascii_upload_matches_reference_escaping_not_url_form_encoding() {
    let meta = BTreeMap::from([
        ("Mixed CASE%".into(), b"space + slash/\n\0%".to_vec()),
        ("mtime".into(), b"1700000000".to_vec()),
    ]);
    assert_eq!(
        encode_metadata(&meta).unwrap(),
        BTreeMap::from([
            ("mixed case%25".into(), "space + slash/%0A%00%25".into()),
            ("mtime".into(), "1700000000".into()),
        ])
    );
}

#[test]
fn unknown_is_distinct_from_known_empty_and_invalid_utf8_uses_reference_rune_error() {
    let unknown: Metadata = None;
    let known: Metadata = Some(decode_metadata([]));
    assert_ne!(unknown, known);
    assert_eq!(known, Some(BTreeMap::new()));
    assert_eq!(
        encode_metadata(&BTreeMap::from([("key".into(), vec![255])])).unwrap()["key"],
        "\u{fffd}"
    );
}

#[test]
fn unicode_printing_and_nonprinting_follow_reference_codepoint_escaping() {
    let meta = BTreeMap::from([(
        "key".into(),
        "\u{03bb}\u{4e2d}\u{00a0}\u{200b}".as_bytes().to_vec(),
    )]);
    assert_eq!(
        encode_metadata(&meta).unwrap()["key"],
        "\u{03bb}\u{4e2d}%A0%200B"
    );
}
