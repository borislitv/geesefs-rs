mod support;

use geesefs_rs::filesystem::Filesystem;
use std::sync::atomic::Ordering;
use support::MemoryStore;

#[tokio::test]
async fn an_active_remainder_keeps_pending_inodes_until_the_frame_is_consumed() {
    let store = MemoryStore::new();
    for index in 0..64 {
        store.seed(&format!("pending-{index:02}"), b"unsent bytes");
    }
    store.seed("z", b"later object");
    let mut config = MemoryStore::config(16);
    config.entry_limit = 65;
    let fs = Filesystem::new(config, store).unwrap();
    let pending = fs.directory_remainder(1, "", "", 64).await.unwrap();
    assert_eq!(pending.entries.len(), 64);
    assert!(matches!(
        fs.lookup(1, "z").await,
        Err(geesefs_rs::error::Error::Fs(libc::ENOMEM))
    ));
    for entry in &pending.entries {
        assert_eq!(fs.node(entry.ino).unwrap().attr().size, 12);
    }
    let cached = fs.directory_remainder(1, "", "", 64).await.unwrap();
    drop(pending);
    assert!(matches!(
        fs.lookup(1, "z").await,
        Err(geesefs_rs::error::Error::Fs(libc::ENOMEM))
    ));
    for entry in &cached.entries {
        assert_eq!(fs.node(entry.ino).unwrap().attr().size, 12);
    }
    drop(cached);
    assert_eq!(fs.lookup(1, "z").await.unwrap().attr().size, 12);
    assert_eq!(fs.namespace.lock().unwrap().nodes.len(), 65);
}

#[tokio::test]
async fn a_standard_listing_does_not_restart_cached_child_metadata_freshness() {
    let store = MemoryStore::new();
    store.standard_listing.store(true, Ordering::Relaxed);
    store.seed("file", b"old");
    store
        .objects
        .lock()
        .unwrap()
        .get_mut("file")
        .unwrap()
        .metadata
        .insert("mode".into(), (libc::S_IFREG | 0o640).to_string());
    let fs = store.filesystem(16);
    let node = fs.lookup(1, "file").await.unwrap();
    let ttl = fs.config.stat_cache_ttl;
    node.state.lock().unwrap().fetched = std::time::Instant::now() - ttl * 3 / 4;
    let heads = store.head_starts.load(Ordering::Relaxed);
    store
        .objects
        .lock()
        .unwrap()
        .get_mut("file")
        .unwrap()
        .metadata
        .insert("mode".into(), (libc::S_IFREG | 0o600).to_string());

    let page = fs.directory_chunk(1, "", 10).await.unwrap();
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].ino, node.ino);
    let (attr, remaining) = node.cached_attr(ttl);
    assert_eq!(attr.size, 3);
    assert_eq!(
        attr.perm, 0o640,
        "LIST supplies no replacement permission metadata"
    );
    assert!(
        remaining <= ttl / 4,
        "a LIST without user metadata must not restart a cached child's TTL: {remaining:?}"
    );
    assert_eq!(store.head_starts.load(Ordering::Relaxed), heads);
}

#[tokio::test]
async fn a_released_consumed_prefix_does_not_restart_the_remaining_backend_page() {
    let store = MemoryStore::new();
    for name in ["a", "b", "c", "d", "z"] {
        store.seed(name, name.as_bytes());
    }
    let mut config = MemoryStore::config(16);
    config.entry_limit = 5;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let page = fs.directory_chunk(1, "", 4).await.unwrap();
    let consumed = page[0].ino;
    let held = fs.lookup(1, "z").await.unwrap();
    assert!(
        fs.node(consumed).is_err(),
        "pressure must reclaim the consumed inode"
    );
    let lists = store.list_calls.load(Ordering::Relaxed);

    let remainder = fs.directory_remainder(1, "", "a", 4).await.unwrap();
    assert_eq!(
        remainder
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["b", "c", "d"]
    );
    assert_eq!(remainder.next_after.as_deref(), Some("d"));
    for entry in remainder.entries {
        assert_eq!(entry.kind, fuser::FileType::RegularFile);
        assert_eq!(fs.node(entry.ino).unwrap().attr().size, 1);
    }
    assert_eq!(held.attr().size, 1);
    assert_eq!(
        store.list_calls.load(Ordering::Relaxed),
        lists,
        "consumed inode eviction must not repeat LIST or rebuild the emitted prefix"
    );
}

#[tokio::test]
async fn page_remainders_still_refresh_expired_unconsumed_types() {
    let store = MemoryStore::new();
    store.standard_listing.store(true, Ordering::Relaxed);
    for name in ["a", "b", "c"] {
        store.seed(name, name.as_bytes());
    }
    let fs = store.filesystem(16);
    let page = fs.directory_chunk(1, "", 3).await.unwrap();
    let changed = fs.node(page[1].ino).unwrap();
    changed.state.lock().unwrap().fetched =
        std::time::Instant::now() - 2 * fs.config.stat_cache_ttl;
    store
        .objects
        .lock()
        .unwrap()
        .get_mut("b")
        .unwrap()
        .metadata
        .insert("--symlink-target".into(), "target".into());

    let remainder = fs.directory_remainder(1, "", "a", 3).await.unwrap();
    assert_eq!(remainder.next_after.as_deref(), Some("c"));
    assert_eq!(remainder.entries.len(), 2);
    assert_eq!(remainder.entries[0].name, "b");
    assert_eq!(remainder.entries[0].kind, fuser::FileType::Symlink);
    assert_eq!(fs.node(remainder.entries[0].ino).unwrap().attr().size, 1);
    assert_eq!(remainder.entries[1].name, "c");
    assert_eq!(remainder.entries[1].kind, fuser::FileType::RegularFile);
    assert_eq!(fs.node(remainder.entries[1].ino).unwrap().attr().size, 1);
}

#[tokio::test]
async fn exhausted_remainders_advance_past_new_prefixes_instead_of_reporting_eof() {
    let store = MemoryStore::new();
    for name in ["b", "c", "d"] {
        store.seed(name, name.as_bytes());
    }
    let fs = store.filesystem(16);
    assert_eq!(fs.directory_chunk(1, "", 2).await.unwrap().len(), 2);
    for name in ["a0", "a1"] {
        fs.create(1, name, libc::S_IFREG | 0o644, None, 0)
            .await
            .unwrap();
    }

    let exhausted = fs.directory_remainder(1, "", "b", 2).await.unwrap();
    assert!(exhausted.entries.is_empty());
    assert_eq!(exhausted.next_after.as_deref(), Some("b"));
    let next = fs.directory_remainder(1, "b", "b", 2).await.unwrap();
    assert_eq!(
        next.entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["c", "d"]
    );
    assert_eq!(next.next_after.as_deref(), Some("d"));
    let eof = fs.directory_remainder(1, "d", "d", 2).await.unwrap();
    assert!(eof.entries.is_empty());
    assert!(eof.next_after.is_none());
}

#[tokio::test]
async fn bounded_directory_chunks_resume_after_names_instead_of_rescanning_old_pages() {
    let store = MemoryStore::new();
    store.page_size.store(4, Ordering::Relaxed);
    for name in ["a", "b", "c", "d", "e", "f", "g"] {
        store.seed(name, name.as_bytes());
    }
    let fs = store.filesystem(16);
    let mut after = String::new();
    let mut names = Vec::new();
    loop {
        let entries = fs.directory_chunk(1, &after, 2).await.unwrap();
        if entries.is_empty() {
            break;
        }
        for entry in entries {
            assert_eq!(entry.kind, fuser::FileType::RegularFile);
            assert_eq!(fs.node(entry.ino).unwrap().attr().size, 1);
            after = entry.name.as_str().to_owned();
            names.push(entry.name);
        }
    }
    assert_eq!(names, ["a", "b", "c", "d", "e", "f", "g"]);
    assert_eq!(
        store.list_calls.load(Ordering::Relaxed),
        6,
        "one truncated flat attempt, then one delimiter page per chunk including EOF"
    );
}

#[tokio::test]
async fn resuming_after_a_directory_keeps_lexically_adjacent_objects() {
    let store = MemoryStore::new();
    store.page_size.store(2, Ordering::Relaxed);
    for name in ["d/alpha/child", "d/alpha.", "d/alpha0", "d/z"] {
        store.seed(name, b"contents");
    }
    let fs = store.filesystem(16);
    let directory = fs.lookup(1, "d").await.unwrap();
    let mut after = String::new();
    let mut names = Vec::new();
    for _ in 0..8 {
        let entries = fs.directory_chunk(directory.ino, &after, 1).await.unwrap();
        if entries.is_empty() {
            break;
        }
        let entry = &entries[0];
        if entry.name == "alpha" {
            assert_eq!(entry.kind, fuser::FileType::Directory);
        } else {
            assert_eq!(entry.kind, fuser::FileType::RegularFile);
            assert_eq!(fs.node(entry.ino).unwrap().attr().size, 8);
        }
        after = entry.name.as_str().to_owned();
        names.push(entry.name.as_str().to_owned());
    }
    assert_eq!(names, ["alpha", "alpha.", "alpha0", "z"]);
}

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
    assert_eq!(
        store.list_calls.load(Ordering::SeqCst),
        1,
        "one flat page serves both bounded chunks"
    );
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
    assert_eq!(
        store.list_calls.load(Ordering::SeqCst),
        1,
        "fresh flat metadata recreates evicted identities without another LIST"
    );
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
async fn cached_page_rechecks_expired_child_metadata() {
    let store = MemoryStore::new();
    store.standard_listing.store(true, Ordering::Relaxed);
    for name in ["a", "b", "c"] {
        store.seed(name, b"old");
    }
    let fs = store.filesystem(16);
    let first = fs.directory_chunk(1, "", 10).await.unwrap();
    for entry in &first {
        let node = fs.node(entry.ino).unwrap();
        node.state.lock().unwrap().fetched =
            std::time::Instant::now() - 2 * fs.config.stat_cache_ttl;
        store.seed(&entry.name, b"changed");
    }
    store
        .objects
        .lock()
        .unwrap()
        .get_mut("b")
        .unwrap()
        .metadata
        .insert("--symlink-target".into(), "target".into());

    let entries = fs.directory_chunk(1, "", 10).await.unwrap();
    assert_eq!(entries.len(), 3);
    for (old, fresh) in first.iter().zip(&entries) {
        assert_eq!(fresh.ino, old.ino);
        let attr = fs.node(fresh.ino).unwrap().attr();
        if fresh.name == "b" {
            assert_eq!(fresh.kind, fuser::FileType::Symlink);
            assert_eq!(attr.size, 7);
        } else {
            assert_eq!(fresh.kind, fuser::FileType::RegularFile);
            assert_eq!(
                attr.size, 7,
                "a fresh name page must not extend older attributes"
            );
        }
    }
    let calls = store.head_starts.load(Ordering::SeqCst);
    for entry in &entries {
        fs.refresh_inode_if_expired(entry.ino).await.unwrap();
    }
    assert_eq!(store.head_starts.load(Ordering::SeqCst), calls);
}

#[tokio::test]
async fn extended_pages_refresh_existing_metadata_without_redundant_heads() {
    let store = MemoryStore::new();
    store.seed("item", b"old");
    let mut config = MemoryStore::config(16);
    config.stat_cache_ttl = std::time::Duration::ZERO;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let original = fs.lookup(1, "item").await.unwrap();
    store.seed("item", b"new generation");
    let calls = store.head_starts.load(Ordering::SeqCst);
    let entries = fs.directory_chunk(1, "", 10).await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].ino, original.ino);
    assert_eq!(original.attr().size, 14);
    assert_eq!(store.head_starts.load(Ordering::SeqCst), calls);
    assert_eq!(
        fs.read(original.ino, 0, 14).await.unwrap().as_slice(),
        b"new generation"
    );
}

#[tokio::test]
async fn listing_metadata_without_an_etag_requires_an_authoritative_generation() {
    let store = MemoryStore::new();
    store.seed("item", b"old");
    let fs = store.filesystem(16);
    let node = fs.lookup(1, "item").await.unwrap();
    node.state.lock().unwrap().fetched = std::time::Instant::now() - 2 * fs.config.stat_cache_ttl;
    store.seed("item", b"new generation");
    store.omit_list_etag.store(true, Ordering::Relaxed);
    let before = store.head_starts.load(Ordering::Relaxed);
    let entries = fs.directory_chunk(1, "", 10).await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].ino, node.ino);
    assert_eq!(
        fs.read(node.ino, 0, 14).await.unwrap().as_slice(),
        b"new generation"
    );
    assert_eq!(store.head_starts.load(Ordering::Relaxed), before + 1);
}

#[tokio::test]
async fn stale_extended_page_cannot_replace_a_newer_head_or_committed_write() {
    for write in [false, true] {
        let store = MemoryStore::new();
        store.seed("item", b"old");
        let mut config = MemoryStore::config(16);
        config.stat_cache_ttl = std::time::Duration::ZERO;
        let fs = Filesystem::new(config, store.clone()).unwrap();
        let original = fs.lookup(1, "item").await.unwrap();
        let (entered, release) = store.pause_list();
        let listing = tokio::spawn({
            let fs = fs.clone();
            async move { fs.directory_chunk(1, "", 10).await }
        });
        tokio::time::timeout(std::time::Duration::from_secs(3), entered)
            .await
            .unwrap()
            .unwrap();
        if write {
            fs.write(original.ino, 0, b"new generation").await.unwrap();
            fs.fsync(original.ino).await.unwrap();
        } else {
            store.seed("item", b"new generation");
            fs.refresh_inode_if_expired(original.ino).await.unwrap();
        }
        release.send(()).unwrap();
        let entries = tokio::time::timeout(std::time::Duration::from_secs(3), listing)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].ino, original.ino);
        assert_eq!(original.attr().size, 14);
        assert_eq!(
            fs.read(original.ino, 0, 14).await.unwrap().as_slice(),
            b"new generation"
        );
    }
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
