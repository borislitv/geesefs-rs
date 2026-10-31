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

#[tokio::test]
async fn mutations_in_another_directory_preserve_fresh_listing_pages() {
    let store = MemoryStore::new();
    store.seed("stable/a", b"a");
    store.seed("work/", b"");
    let fs = store.filesystem(16);
    let stable = fs.lookup(1, "stable").await.unwrap();
    let work = fs.lookup(1, "work").await.unwrap();
    let original = fs.directory_chunk(stable.ino, "", 10).await.unwrap();
    assert_eq!(original.len(), 1);
    assert_eq!(original[0].name, "a");
    assert_eq!(original[0].kind, fuser::FileType::RegularFile);

    fs.create(work.ino, "temporary", libc::S_IFREG | 0o644, None, 0)
        .await
        .unwrap();
    for operation in 0..3 {
        if operation == 1 {
            fs.rename(work.ino, "temporary", work.ino, "renamed")
                .await
                .unwrap();
        } else if operation == 2 {
            fs.unlink(work.ino, "renamed", false).await.unwrap();
        }
        let before = store.list_calls.load(Ordering::SeqCst);
        let entries = fs.directory_chunk(stable.ino, "", 10).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "a");
        assert_eq!(entries[0].ino, original[0].ino);
        assert_eq!(entries[0].kind, original[0].kind);
        assert_eq!(fs.node(entries[0].ino).unwrap().attr().size, 1);
        assert_eq!(
            store.list_calls.load(Ordering::SeqCst),
            before,
            "unrelated create/rename/unlink must not refetch a fresh directory page"
        );
    }
}

#[tokio::test]
async fn moving_an_entry_invalidates_both_parent_directory_pages() {
    let store = MemoryStore::new();
    store.seed("left/a", b"a");
    store.seed("right/b", b"b");
    let fs = store.filesystem(16);
    let left = fs.lookup(1, "left").await.unwrap();
    let right = fs.lookup(1, "right").await.unwrap();
    assert_eq!(
        fs.directory_chunk(left.ino, "", 10).await.unwrap()[0].name,
        "a"
    );
    assert_eq!(
        fs.directory_chunk(right.ino, "", 10).await.unwrap()[0].name,
        "b"
    );
    fs.rename(left.ino, "a", right.ino, "c").await.unwrap();
    assert!(
        fs.directory_chunk(left.ino, "", 10)
            .await
            .unwrap()
            .is_empty()
    );
    let entries = fs.directory_chunk(right.ino, "", 10).await.unwrap();
    assert_eq!(
        entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        ["b", "c"]
    );
    fs.unlink(right.ino, "b", false).await.unwrap();
    let entries = fs.directory_chunk(right.ino, "", 10).await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "c");
}

#[tokio::test]
async fn unrelated_mutations_do_not_discard_an_inflight_directory_listing() {
    let store = MemoryStore::new();
    store.seed("stable/a", b"a");
    store.seed("work/", b"");
    let fs = store.filesystem(16);
    let stable = fs.lookup(1, "stable").await.unwrap();
    let work = fs.lookup(1, "work").await.unwrap();
    let (entered, release) = store.pause_list();
    let listing = tokio::spawn({
        let fs = fs.clone();
        async move { fs.directory_chunk(stable.ino, "", 10).await }
    });
    tokio::time::timeout(std::time::Duration::from_secs(3), entered)
        .await
        .unwrap()
        .unwrap();
    fs.create(work.ino, "new", libc::S_IFREG | 0o644, None, 0)
        .await
        .unwrap();
    release.send(()).unwrap();
    let entries = tokio::time::timeout(std::time::Duration::from_secs(3), listing)
        .await
        .unwrap()
        .unwrap()
        .expect("an unrelated mutation must not invalidate the listing response");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "a");
}

#[tokio::test]
async fn parent_rename_rejects_old_descendant_listing_and_reuses_live_inode() {
    let store = MemoryStore::new();
    store.seed("old/sub/a", b"a");
    let fs = store.filesystem(16);
    let old = fs.lookup(1, "old").await.unwrap();
    let sub = fs.lookup(old.ino, "sub").await.unwrap();
    let sub_ino = sub.ino;
    let (entered, release) = store.pause_list();
    let listing = tokio::spawn({
        let fs = fs.clone();
        async move { fs.directory_chunk(sub_ino, "", 10).await }
    });
    tokio::time::timeout(std::time::Duration::from_secs(3), entered)
        .await
        .unwrap()
        .unwrap();
    fs.rename(1, "old", 1, "new").await.unwrap();
    release.send(()).unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), listing)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        result,
        Err(geesefs_rs::error::Error::Fs(libc::EAGAIN))
    ));
    let new = fs.lookup(1, "new").await.unwrap();
    assert_eq!(fs.lookup(new.ino, "sub").await.unwrap().ino, sub_ino);
    let entries = fs.directory_chunk(sub_ino, "", 10).await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "a");
    assert_eq!(fs.node(entries[0].ino).unwrap().attr().size, 1);
}

#[tokio::test]
async fn authoritative_directory_removal_rejects_its_inflight_listing() {
    let store = MemoryStore::new();
    store.seed("directory/", b"");
    let mut config = MemoryStore::config(16);
    config.stat_cache_ttl = std::time::Duration::ZERO;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let directory = fs.lookup(1, "directory").await.unwrap();
    let ino = directory.ino;
    let (entered, release) = store.pause_list();
    let listing = tokio::spawn({
        let fs = fs.clone();
        async move { fs.directory_chunk(ino, "", 10).await }
    });
    tokio::time::timeout(std::time::Duration::from_secs(3), entered)
        .await
        .unwrap()
        .unwrap();
    store.objects.lock().unwrap().remove("directory/");
    assert!(matches!(
        fs.refresh_inode_if_expired(ino).await,
        Err(geesefs_rs::error::Error::Fs(libc::ENOENT))
    ));
    release.send(()).unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), listing)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        result,
        Err(geesefs_rs::error::Error::Fs(libc::EAGAIN))
    ));
}
