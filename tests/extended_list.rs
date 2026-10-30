use std::collections::BTreeMap;

use geesefs_rs::backend::extended_list::{MAX_PAGE_BYTES, decode_page, permits_standard_fallback};

#[test]
fn authoritative_extended_metadata_tokens_and_unknown_subtrees() {
    let page = decode_page(
        include_bytes!("fixtures/ext-v1.xml"),
        "pkgs/",
        Some("previous"),
    )
    .unwrap();
    assert_eq!(page.next_token.as_deref(), Some("next&token"));
    assert_eq!(page.common_prefixes, ["pkgs/sub/"]);
    assert_eq!(page.objects.len(), 2);
    assert_eq!(page.objects[0].key, "pkgs/a%20b&c");
    assert_eq!(page.objects[0].size, 42);
    assert_eq!(page.objects[0].etag.as_deref(), Some("\"etag\""));
    assert_eq!(page.objects[0].last_modified.unwrap().secs(), 1792931696);
    assert_eq!(
        page.objects[0].metadata,
        Some(BTreeMap::from([
            ("mtime".into(), b"789".to_vec()),
            ("empty".into(), vec![])
        ]))
    );
    assert_eq!(page.objects[1].metadata, Some(BTreeMap::new()));
}

#[test]
fn rejects_incomplete_unconfined_or_malformed_pages() {
    for xml in [
        "<ListBucketResult><IsTruncated>false</IsTruncated>",
        "<ListBucketResult><IsTruncated>bad</IsTruncated></ListBucketResult>",
        "<ListBucketResult><IsTruncated>true</IsTruncated></ListBucketResult>",
        "<ListBucketResult><IsTruncated>false</IsTruncated><Contents><Key>other/a</Key><Size>1</Size></Contents></ListBucketResult>",
        "<ListBucketResult><IsTruncated>false</IsTruncated><Contents><Key>pkgs/a</Key><Size>-1</Size></Contents></ListBucketResult>",
        "<ListBucketResult><IsTruncated>false</IsTruncated><Contents><Key>pkgs/a</Key><Size>0</Size><LastModified>bad</LastModified></Contents></ListBucketResult>",
        "<ListBucketResult><IsTruncated>false</IsTruncated><Contents><Key>pkgs/a</Key><Size>1</Size><Metadata><Name>x</Name></Metadata></Contents></ListBucketResult>",
        "<ListBucketResult><IsTruncated>false</IsTruncated></ListBucketResult><Other/>",
        "<Other><IsTruncated>false</IsTruncated></Other>",
        "<!DOCTYPE x><ListBucketResult><IsTruncated>false</IsTruncated></ListBucketResult>",
    ] {
        assert!(
            decode_page(xml.as_bytes(), "pkgs/", None).is_err(),
            "accepted {xml}"
        );
    }
    assert!(
        decode_page(
            include_bytes!("fixtures/ext-v1.xml"),
            "pkgs/",
            Some("next&token")
        )
        .is_err()
    );
    assert!(decode_page(&vec![b' '; MAX_PAGE_BYTES + 1], "", None).is_err());
}

#[test]
fn fallback_requires_exact_modeled_unsupported_error() {
    for code in ["InvalidArgument", "NotImplemented"] {
        assert!(permits_standard_fallback(code));
    }
    for code in [
        "AccessDenied",
        "InternalError",
        "SerializationError",
        "403",
        "501",
        "notimplemented",
    ] {
        assert!(
            !permits_standard_fallback(code),
            "incorrect fallback for {code}"
        );
    }
}

#[test]
fn page_entry_and_unknown_subtree_depth_limits_are_enforced() {
    let mut xml = String::from("<ListBucketResult><IsTruncated>false</IsTruncated>");
    for i in 0..1000 {
        xml.push_str(&format!(
            "<Contents><Key>pkgs/{i:04}</Key><Size>0</Size></Contents>"
        ));
    }
    let valid = format!("{xml}</ListBucketResult>");
    let page = decode_page(valid.as_bytes(), "pkgs/", None).unwrap();
    assert_eq!(page.objects.len(), 1000);
    assert_eq!(page.objects[999].key, "pkgs/0999");
    xml.push_str("<Contents><Key>pkgs/1000</Key><Size>0</Size></Contents></ListBucketResult>");
    assert!(decode_page(xml.as_bytes(), "pkgs/", None).is_err());

    let deep = format!(
        "<ListBucketResult><IsTruncated>false</IsTruncated>{}{}</ListBucketResult>",
        "<Future>".repeat(33),
        "</Future>".repeat(33)
    );
    assert!(decode_page(deep.as_bytes(), "", None).is_err());
}
