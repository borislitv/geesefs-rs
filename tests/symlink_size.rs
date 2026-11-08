mod support;

use geesefs_rs::{config::ListType, filesystem::Filesystem};
use std::sync::atomic::Ordering;
use support::MemoryStore;

#[tokio::test]
async fn stored_symlinks_report_object_size_through_head_and_extended_listing() {
    for standard in [false, true] {
        let store = MemoryStore::new();
        store.standard_listing.store(standard, Ordering::Relaxed);
        for (name, body) in [("empty", b"".as_slice()), ("body", b"retained".as_slice())] {
            store.seed(name, body);
            store
                .objects
                .lock()
                .unwrap()
                .get_mut(name)
                .unwrap()
                .metadata
                .insert("--symlink-target".into(), "a-much-longer-target".into());
        }
        let mut config = MemoryStore::config(16);
        config.list_type = Some(ListType::Extended);
        let fs = Filesystem::new(config, store.clone()).unwrap();
        for (name, expected) in [("empty", 0), ("body", 8)] {
            let node = fs.lookup(1, name).await.unwrap();
            assert_eq!(node.attr().kind, fuser::FileType::Symlink);
            assert_eq!(node.attr().size, expected, "{name}, standard={standard}");
        }
        assert_eq!(store.bytes("empty"), b"");
        assert_eq!(store.bytes("body"), b"retained");
    }
}

#[tokio::test]
async fn created_symlink_reports_zero_size_before_and_after_durable_reload() {
    let store = MemoryStore::new();
    let fs = store.filesystem(16);
    let link = fs
        .create(1, "link", libc::S_IFLNK | 0o777, Some(b"target"), 0)
        .await
        .unwrap();
    assert_eq!(link.attr().kind, fuser::FileType::Symlink);
    assert_eq!(link.attr().size, 0);
    fs.fsync(link.ino).await.unwrap();
    assert_eq!(store.bytes("link"), b"");
    let fresh = store.filesystem(16);
    let loaded = fresh.lookup(1, "link").await.unwrap();
    assert_eq!(loaded.attr().kind, fuser::FileType::Symlink);
    assert_eq!(loaded.attr().size, 0);
    assert_eq!(
        store.objects.lock().unwrap()["link"].metadata["--symlink-target"],
        "target"
    );
}
