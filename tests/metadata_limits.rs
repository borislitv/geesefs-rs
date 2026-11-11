#![forbid(unsafe_code)]
mod support;

use clap::Parser;
use geesefs_rs::{config::Config, error::Error, filesystem::Filesystem};
use support::MemoryStore;

fn limited(bytes: usize) -> Config {
    let mut config = MemoryStore::config(32);
    config.metadata_limit = Some(bytes);
    config
}

fn metadata_object(store: &MemoryStore, name: &str, bytes: usize) {
    store.seed(name, b"payload");
    store
        .objects
        .lock()
        .unwrap()
        .get_mut(name)
        .unwrap()
        .metadata
        .insert("label".into(), "x".repeat(bytes));
}

#[test]
fn metadata_limit_rejects_invalid_sizes_and_accepts_multi_gib_totals() {
    for value in ["0", "-1", "not-a-size", "18446744073709551615 GiB"] {
        assert!(
            Config::try_parse_from([
                "geesefs-rs",
                "--metadata-limit",
                value,
                "bucket",
                "/tmp/mount"
            ])
            .is_err()
        );
    }
    let config = Config::try_parse_from([
        "geesefs-rs",
        "--metadata-limit",
        "64 GiB",
        "bucket",
        "/tmp/mount",
    ])
    .unwrap();
    assert_eq!(config.metadata_limit, Some(64 * 1024 * 1024 * 1024));
    assert!(geesefs_rs::metadata_budget::MetadataBudget::new(config.metadata_limit).is_ok());
}

#[tokio::test]
async fn explicit_metadata_limit_rejects_large_metadata_without_publishing_an_inode() {
    let config = Config::try_parse_from([
        "geesefs-rs",
        "-f",
        "--metadata-limit",
        "16 KiB",
        "bucket",
        "/tmp/mount",
    ])
    .expect("explicit separate metadata byte limit must parse");
    let store = MemoryStore::new();
    store.seed("large", b"payload");
    store
        .objects
        .lock()
        .unwrap()
        .get_mut("large")
        .unwrap()
        .metadata
        .insert("label".into(), "x".repeat(32 * 1024));
    let fs = Filesystem::new(config, store).unwrap();
    assert!(matches!(
        fs.lookup(1, "large").await,
        Err(Error::Fs(libc::ENOMEM))
    ));
    assert!(fs.namespace.lock().unwrap().named("large").is_none());
    assert_eq!(
        fs.effective_budgets().payload_limit_bytes,
        1000 * 1024 * 1024
    );
}

#[tokio::test]
async fn unique_names_reclaim_by_bytes_below_the_unchanged_count_limit() {
    let store = MemoryStore::new();
    for n in 0..40 {
        metadata_object(&store, &format!("file-{n}"), 8 * 1024);
    }
    let fs = Filesystem::new(limited(64 * 1024), store).unwrap();
    for n in 0..40 {
        let node = fs.lookup(1, &format!("file-{n}")).await.unwrap();
        assert_eq!(node.attr().size, 7);
        assert!(fs.metadata_usage().charged <= 64 * 1024);
    }
    assert!(fs.namespace.lock().unwrap().nodes.len() < 40);
    assert!(fs.namespace.lock().unwrap().evictions > 0);
}

#[tokio::test]
async fn live_and_kernel_referenced_inodes_are_never_evicted_to_pay_for_admission() {
    let store = MemoryStore::new();
    for name in ["held", "other"] {
        metadata_object(&store, name, 10 * 1024);
    }
    // Compact retained metadata leaves more spare room than the old two-map
    // representation. Keep this fixture under actual admission pressure.
    let fs = Filesystem::new(limited(48 * 1024), store).unwrap();
    let held = fs.lookup(1, "held").await.unwrap();
    fs.reference(&held);
    let ino = held.ino;
    assert!(matches!(
        fs.lookup(1, "other").await,
        Err(Error::Fs(libc::ENOMEM))
    ));
    assert_eq!(fs.node(ino).unwrap().ino, ino);
    drop(held);
    assert!(matches!(
        fs.lookup(1, "other").await,
        Err(Error::Fs(libc::ENOMEM))
    ));
    fs.forget(ino, 1);
    assert_eq!(fs.lookup(1, "other").await.unwrap().attr().size, 7);
    assert!(fs.node(ino).is_err());
}

#[tokio::test]
async fn rejected_refresh_growth_preserves_the_previous_authoritative_metadata() {
    let store = MemoryStore::new();
    metadata_object(&store, "file", 1024);
    let mut config = limited(16 * 1024);
    config.stat_cache_ttl = std::time::Duration::ZERO;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let node = fs.lookup(1, "file").await.unwrap();
    metadata_object(&store, "file", 16 * 1024);
    assert!(matches!(
        fs.refresh_inode_if_expired(node.ino).await,
        Err(Error::Fs(libc::ENOMEM))
    ));
    assert_eq!(
        node.state.lock().unwrap().metadata["label"],
        vec![b'x'; 1024]
    );
    assert!(fs.metadata_usage().charged <= 16 * 1024);
}

#[tokio::test]
async fn rejected_xattr_growth_is_atomic_and_does_not_make_the_file_dirty() {
    let store = MemoryStore::new();
    store.seed("file", b"payload");
    let fs = Filesystem::new(limited(16 * 1024), store.clone()).unwrap();
    let node = fs.lookup(1, "file").await.unwrap();
    let version = node.state.lock().unwrap().version;
    assert!(matches!(
        fs.setxattr(node.ino, "user.large", Some(&vec![b'x'; 20 * 1024]), 0),
        Err(Error::Fs(libc::ENOMEM))
    ));
    {
        let state = node.state.lock().unwrap();
        assert_eq!(state.version, version);
        assert!(!state.metadata.contains_key("large"));
    }
    fs.fsync(node.ino).await.unwrap();
    assert_eq!(
        store.put_calls.load(std::sync::atomic::Ordering::Relaxed),
        0
    );
}

#[tokio::test]
async fn rejected_setattr_preserves_attributes_and_generation() {
    let store = MemoryStore::new();
    store.seed("file", b"payload");
    store.seed("pinned", b"payload");
    let mut config = limited(16 * 1024);
    config.enable_perms = true;
    let fs = Filesystem::new(config, store).unwrap();
    let file = fs.lookup(1, "file").await.unwrap();
    let pinned = fs.lookup(1, "pinned").await.unwrap();
    let spare = 16 * 1024 - fs.metadata_usage().charged;
    fs.setxattr(pinned.ino, "user.fill", Some(&vec![b'x'; spare - 400]), 0)
        .unwrap();
    let before = file.attr();
    let version = file.state.lock().unwrap().version;
    assert!(matches!(
        fs.setattr(file.ino, Some(0o600), Some(1234), Some(5678), None),
        Err(Error::Fs(libc::ENOMEM))
    ));
    let after = file.attr();
    assert_eq!(
        (after.perm, after.uid, after.gid, after.mtime),
        (before.perm, before.uid, before.gid, before.mtime)
    );
    assert_eq!(file.state.lock().unwrap().version, version);
}

#[tokio::test]
async fn returned_directory_names_release_their_own_charge_when_dropped() {
    let store = MemoryStore::new();
    store.seed("entry-with-a-retained-name", b"payload");
    let fs = Filesystem::new(limited(64 * 1024), store).unwrap();
    let entries = fs.directory_chunk(1, "", 10).await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "entry-with-a-retained-name");
    let retained = fs.metadata_usage().charged;
    drop(entries);
    assert!(
        fs.metadata_usage().charged < retained,
        "caller-owned entries must retain a separate lifetime charge"
    );
}

#[tokio::test]
async fn negative_cache_admission_is_bounded_without_changing_missing_results() {
    let store = MemoryStore::new();
    let mut config = limited(8 * 1024);
    config.negative_lookup_cache_size = 100_000;
    let fs = Filesystem::new(config, store).unwrap();
    for index in 0..200 {
        assert!(matches!(
            fs.lookup(1, &format!("missing-{index}")).await,
            Err(Error::Fs(libc::ENOENT))
        ));
        assert!(fs.metadata_usage().charged <= 8 * 1024);
    }
    assert!(
        fs.node(1)
            .unwrap()
            .state
            .lock()
            .unwrap()
            .negative_lookups
            .len()
            < 200
    );
}

#[tokio::test]
async fn count_pressure_resumes_beyond_a_pinned_reclamation_batch() {
    let store = MemoryStore::new();
    for index in 0..257 {
        store.seed(&format!("file-{index}"), b"payload");
    }
    let mut config = MemoryStore::config(32);
    config.entry_limit = 257;
    let fs = Filesystem::new(config, store).unwrap();
    let mut pinned = Vec::new();
    for index in 0..256 {
        let node = fs.lookup(1, &format!("file-{index}")).await.unwrap();
        if index < 255 {
            pinned.push(node);
        }
    }
    let replacement =
        tokio::time::timeout(std::time::Duration::from_secs(5), fs.lookup(1, "file-256"))
            .await
            .unwrap()
            .unwrap();
    assert_eq!(replacement.attr().size, 7);
    assert!(fs.namespace.lock().unwrap().named("file-255").is_none());
    for node in pinned {
        assert_eq!(fs.node(node.ino).unwrap().ino, node.ino);
    }
    assert_eq!(
        fs.metadata_usage().limit,
        None,
        "count-only default must stay supported"
    );
}

#[tokio::test]
async fn count_pressure_revisits_a_previously_pinned_prefix_on_the_first_lookup() {
    for prefix_len in [1, 127, 128, 129, 255, 256, 257] {
        let store = MemoryStore::new();
        for index in 0..prefix_len {
            store.seed(&format!("pinned-{index}"), b"payload");
        }
        for name in ["victim", "replacement", "next"] {
            store.seed(name, b"payload");
        }
        let mut config = MemoryStore::config(32);
        config.entry_limit = prefix_len + 2; // Root, pinned prefix and one victim.
        let fs = Filesystem::new(config, store).unwrap();
        let mut pinned = Vec::new();
        for index in 0..prefix_len {
            let node = fs.lookup(1, &format!("pinned-{index}")).await.unwrap();
            fs.reference(&node);
            pinned.push(node.ino);
        }
        let victim = fs.lookup(1, "victim").await.unwrap().ino;
        let replacement = fs.lookup(1, "replacement").await.unwrap();
        fs.reference(&replacement);
        let replacement_ino = replacement.ino;
        drop(replacement);
        assert!(fs.node(victim).is_err());

        fs.forget(pinned[0], 1);
        let next = tokio::time::timeout(std::time::Duration::from_secs(5), fs.lookup(1, "next"))
            .await
            .expect("a full scan must terminate")
            .unwrap_or_else(|error| {
                panic!(
                    "first lookup must reclaim the released prefix ({prefix_len} pins): {error:?}"
                )
            });
        assert_eq!(next.attr().size, 7);
        assert!(fs.node(pinned[0]).is_err());
        for ino in pinned.into_iter().skip(1).chain([replacement_ino]) {
            assert_eq!(fs.node(ino).unwrap().ino, ino);
        }
        assert_eq!(fs.namespace.lock().unwrap().evictions, 2);
        assert_eq!(fs.metadata_usage().limit, None);
    }
}

#[tokio::test]
async fn count_pressure_with_all_inodes_pinned_terminates_for_interleaved_admissions() {
    let store = MemoryStore::new();
    for index in 0..385 {
        store.seed(&format!("pinned-{index}"), b"payload");
    }
    for name in ["first", "second"] {
        store.seed(name, b"payload");
    }
    let mut config = MemoryStore::config(32);
    config.entry_limit = 386;
    let fs = Filesystem::new(config, store).unwrap();
    let mut pinned = Vec::new();
    for index in 0..385 {
        let node = fs.lookup(1, &format!("pinned-{index}")).await.unwrap();
        fs.reference(&node);
        pinned.push(node.ino);
    }
    let (first, second) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::join!(fs.lookup(1, "first"), fs.lookup(1, "second"))
    })
    .await
    .expect("each admission must finish its own finite scan even when interleaved");
    for result in [first, second] {
        assert!(matches!(result, Err(Error::Fs(libc::ENOMEM))));
    }
    for ino in pinned {
        assert_eq!(fs.node(ino).unwrap().ino, ino);
    }
    let ns = fs.namespace.lock().unwrap();
    assert_eq!(ns.nodes.len(), 386);
    assert_eq!(ns.evictions, 0);
    assert_eq!(ns.capacity_failures, 2);
    assert!(ns.named("first").is_none());
    assert!(ns.named("second").is_none());
}

#[tokio::test]
async fn synchronous_preload_reclaims_a_tail_beyond_the_pinned_prefix_without_head_fallback() {
    let store = MemoryStore::new();
    store.seed("target", b"payload");
    let mut config = MemoryStore::config(32);
    config.list_type = Some(geesefs_rs::config::ListType::Extended);
    config.entry_limit = 131;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let victim = {
        let mut namespace = fs.namespace.lock().unwrap();
        for index in 0..129 {
            let name = format!("pinned-{index}");
            let node = namespace
                .install(&name, |ino| {
                    geesefs_rs::inode::Inode::new(ino, name.clone(), false, None, &fs.config)
                })
                .unwrap();
            fs.reference(&node);
        }
        namespace
            .install("victim", |ino| {
                geesefs_rs::inode::Inode::new(ino, "victim".into(), false, None, &fs.config)
            })
            .unwrap()
            .ino
    };
    let target = fs.lookup(1, "target").await.unwrap();
    assert_eq!(target.attr().size, 7);
    assert!(fs.node(victim).is_err());
    assert_eq!(
        store.list_calls.load(std::sync::atomic::Ordering::Relaxed),
        1
    );
    assert!(
        store.head_keys.lock().unwrap().is_empty(),
        "the complete synchronous preload must publish the listed metadata, not fall back to HEAD"
    );
    let namespace = fs.namespace.lock().unwrap();
    assert_eq!(namespace.evictions, 1);
    for index in 0..129 {
        assert!(namespace.named(&format!("pinned-{index}")).is_some());
    }
}

#[tokio::test]
async fn a_read_snapshot_keeps_its_charge_after_its_inode_is_reclaimed() {
    let store = MemoryStore::new();
    metadata_object(&store, "first", 8 * 1024);
    metadata_object(&store, "second", 32 * 1024);
    let fs = Filesystem::new(limited(104 * 1024), store).unwrap();
    let node = fs.lookup(1, "first").await.unwrap();
    let ino = node.ino;
    let snapshot = geesefs_rs::read::snapshot(&node).unwrap();
    drop(node);
    assert!(matches!(
        fs.lookup(1, "second").await,
        Err(Error::Fs(libc::ENOMEM))
    ));
    assert!(
        fs.node(ino).is_err(),
        "the namespace owner should already have been reclaimed"
    );
    assert_eq!(
        snapshot.remote().unwrap().metadata["label"],
        vec![b'x'; 8 * 1024]
    );
    drop(snapshot);
    assert_eq!(fs.lookup(1, "second").await.unwrap().attr().size, 7);
}

#[tokio::test]
async fn cancelled_flush_releases_temporary_metadata_and_remains_retryable() {
    let store = MemoryStore::new();
    store.seed("file", b"payload");
    let fs = Filesystem::new(limited(256 * 1024), store.clone()).unwrap();
    let node = fs.lookup(1, "file").await.unwrap();
    fs.write(node.ino, 2, b"XY").await.unwrap();
    let (entered, release) = store.pause_put();
    let flush = tokio::spawn({
        let fs = fs.clone();
        let ino = node.ino;
        async move { fs.fsync(ino).await }
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), entered)
        .await
        .unwrap()
        .unwrap();
    let inflight = fs.metadata_usage().charged;
    flush.abort();
    assert!(flush.await.unwrap_err().is_cancelled());
    drop(release);
    assert!(
        fs.metadata_usage().charged < inflight,
        "cancelled snapshots and request maps must release their permits"
    );
    fs.fsync(node.ino).await.unwrap();
    assert_eq!(store.objects.lock().unwrap()["file"].bytes, b"paXYoad");
    let state = node.state.lock().unwrap();
    assert_eq!(state.version, state.durable);
    assert!(fs.metadata_usage().charged <= 256 * 1024);
}

#[tokio::test]
async fn rejected_creation_never_publishes_a_partial_name_or_dirty_upload() {
    let store = MemoryStore::new();
    let fs = Filesystem::new(limited(16 * 1024), store.clone()).unwrap();
    assert!(matches!(
        fs.create_with_identity(
            1,
            "link",
            libc::S_IFLNK | 0o777,
            Some(&vec![b'x'; 20 * 1024]),
            0,
            (1234, 5678)
        )
        .await,
        Err(Error::Fs(libc::ENOMEM))
    ));
    assert!(fs.namespace.lock().unwrap().named("link").is_none());
    fs.flush_all().await.unwrap();
    assert_eq!(
        store.put_calls.load(std::sync::atomic::Ordering::Relaxed),
        0
    );
}

#[tokio::test]
async fn enabled_metadata_admission_preserves_rename_write_and_delete() {
    let store = MemoryStore::new();
    store.seed("original", b"payload");
    let fs = Filesystem::new(limited(256 * 1024), store.clone()).unwrap();
    let node = fs.lookup(1, "original").await.unwrap();
    let destination = "renamed-".to_owned() + &"x".repeat(192);
    fs.rename(1, "original", 1, &destination).await.unwrap();
    assert_eq!(fs.lookup(1, &destination).await.unwrap().ino, node.ino);
    fs.write(node.ino, 2, b"XY").await.unwrap();
    fs.fsync(node.ino).await.unwrap();
    assert_eq!(
        store.objects.lock().unwrap()[&destination].bytes,
        b"paXYoad"
    );
    fs.unlink(1, &destination, false).await.unwrap();
    assert!(!store.objects.lock().unwrap().contains_key(&destination));
    assert!(fs.namespace.lock().unwrap().named(&destination).is_none());
    assert!(fs.metadata_usage().charged <= 256 * 1024);
}
