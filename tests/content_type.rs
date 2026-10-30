use geesefs_rs::backend::content_type::ContentTypes;
#[test]
fn reference_mime_defaults_case_fallback_and_local_precedence() {
    let mut types = ContentTypes::default();
    assert_eq!(types.guess("bundle.js"), Some("text/javascript"));
    assert_eq!(types.guess("bundle.XML"), Some("text/xml"));
    assert_eq!(types.guess("no-extension"), None);
    assert_eq!(types.guess("dir.ext/file"), None);
    types.add_globs(&b"90:application/first:*.special\n80:application/later:*.special\n99:application/not-javascript:*.js\n30:application/not-a-suffix:*.g?ob\n"[..]).unwrap();
    assert_eq!(types.guess("DATA.SPECIAL"), Some("application/first"));
    assert_eq!(types.guess("bundle.js"), Some("text/javascript"));
    assert_eq!(types.guess("x.glob"), None);
    types
        .add_types(&b"application/new special T1 # comments\ntext/plain;invalid=a t2\n"[..])
        .unwrap();
    assert_eq!(types.guess("DATA.special"), Some("application/new"));
    assert_eq!(types.guess("data.T1"), Some("application/new"));
    assert_eq!(types.guess("data.t1"), Some("application/new"));
    assert_eq!(types.guess("data.t2"), Some("text/plain"));
}
#[test]
fn oversized_mime_input_is_rejected_not_installed() {
    let mut types = ContentTypes::default();
    assert!(types.add_globs(&vec![b'x'; 65537][..]).is_err());
    assert_eq!(types.guess("file.js"), Some("text/javascript"));
}
