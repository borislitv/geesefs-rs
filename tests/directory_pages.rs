mod support;

use geesefs_rs::filesystem::Filesystem;
use std::sync::atomic::Ordering;
use support::MemoryStore;

#[tokio::test]
async fn repeated_directory_pages_keep_names_types_and_avoid_relisting() {
    let store = MemoryStore::new();
    for name in ["a", "b", "c", "d"] {
        store.seed(name, name.as_bytes());
    }
    let fs = store.filesystem(16);
    for _ in 0..2 {
        let first = fs.directory_chunk(1, "", 2).await.unwrap();
        let second = fs.directory_chunk(1, "b", 2).await.unwrap();
        assert_eq!(
            first.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert_eq!(
            second.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            ["c", "d"]
        );
        for entry in first.iter().chain(&second) {
            assert_eq!(entry.kind, fuser::FileType::RegularFile);
            assert_eq!(fs.node(entry.ino).unwrap().attr().size, 1);
        }
    }
    assert_eq!(store.list_calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn local_create_unlink_and_rename_invalidate_warm_directory_pages() {
    let store = MemoryStore::new();
    store.seed("a", b"a");
    let fs = store.filesystem(16);
    assert_eq!(fs.directory_chunk(1, "", 10).await.unwrap()[0].name, "a");
    fs.create(1, "b", libc::S_IFREG | 0o644, None, 0)
        .await
        .unwrap();
    let names = |entries: Vec<geesefs_rs::filesystem::DirEntry>| {
        entries.into_iter().map(|e| e.name).collect::<Vec<_>>()
    };
    assert_eq!(
        names(fs.directory_chunk(1, "", 10).await.unwrap()),
        ["a", "b"]
    );
    fs.unlink(1, "a", false).await.unwrap();
    assert_eq!(names(fs.directory_chunk(1, "", 10).await.unwrap()), ["b"]);
    fs.rename(1, "b", 1, "c").await.unwrap();
    assert_eq!(names(fs.directory_chunk(1, "", 10).await.unwrap()), ["c"]);
}

#[tokio::test]
async fn evicted_listing_inodes_are_resolved_again_instead_of_returning_dead_ids() {
    let store = MemoryStore::new();
    for name in ["a", "b", "c", "d"] {
        store.seed(name, name.as_bytes());
    }
    let mut config = MemoryStore::config(16);
    config.entry_limit = 4;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let old = fs.directory_chunk(1, "", 2).await.unwrap();
    let held = fs.lookup(1, "c").await.unwrap();
    fs.lookup(1, "d").await.unwrap();
    assert!(old.iter().any(|entry| fs.node(entry.ino).is_err()));
    let fresh = fs.directory_chunk(1, "", 2).await.unwrap();
    assert_eq!(
        fresh.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        ["a", "b"]
    );
    for entry in fresh {
        assert_eq!(fs.node(entry.ino).unwrap().attr().size, 1);
    }
    assert_eq!(held.attr().size, 1);
    assert_eq!(store.list_calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn zero_stat_ttl_does_not_reuse_even_an_empty_listing_page() {
    let store = MemoryStore::new();
    let mut config = MemoryStore::config(16);
    config.stat_cache_ttl = std::time::Duration::ZERO;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    assert!(fs.directory_chunk(1, "", 10).await.unwrap().is_empty());
    store.seed("new", b"new data");
    let entries = fs.directory_chunk(1, "", 10).await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "new");
    assert_eq!(fs.node(entries[0].ino).unwrap().attr().size, 8);
    assert_eq!(store.list_calls.load(Ordering::SeqCst), 2);
}
