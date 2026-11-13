#![forbid(unsafe_code)]
mod support;
use geesefs_rs::{error::Error, filesystem::Filesystem};
use std::sync::atomic::Ordering;
use support::MemoryStore;

#[tokio::test]
async fn clean_directory_rename_cancelled_during_discovery_keeps_source_usable() {
    for discovering_etag in [false, true] {
        let store = MemoryStore::new();
        store.seed("source/child", b"payload");
        let mut config = MemoryStore::config(32);
        config.metadata_limit = Some(1024 * 1024);
        config.no_dir_object = true;
        config.no_implicit_dir = true;
        let fs = Filesystem::new(config, store.clone()).unwrap();
        let source = fs
            .create(1, "source", libc::S_IFDIR | 0o755, None, 0)
            .await
            .unwrap();
        store
            .omit_list_etag
            .store(discovering_etag, Ordering::Relaxed);
        let (entered, release) = store.pause_list();
        let rename = tokio::spawn({
            let fs = fs.clone();
            async move { fs.rename(1, "source", 1, "destination").await }
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), entered)
            .await
            .unwrap()
            .unwrap();
        let head_release = if discovering_etag {
            let (entered, head_release) = store.pause_head();
            release.send(()).unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(5), entered)
                .await
                .unwrap()
                .unwrap();
            head_release
        } else {
            release
        };
        let retained = fs.metadata_usage().charged;
        rename.abort();
        assert!(rename.await.unwrap_err().is_cancelled());
        drop(head_release);
        assert_eq!(store.put_calls.load(Ordering::Relaxed), 0);
        assert_eq!(store.copy_calls.load(Ordering::Relaxed), 0);
        assert_eq!(store.delete_calls.load(Ordering::Relaxed), 0);
        assert_eq!(fs.lookup(1, "source").await.unwrap().ino, source.ino);
        assert!(matches!(
            fs.unlink(1, "source", true).await,
            Err(Error::Fs(libc::ENOTEMPTY))
        ));
        assert!(source.state.lock().unwrap().rename_target.is_none());
        assert!(fs.namespace.lock().unwrap().pending_moves.is_empty());
        assert!(
            fs.metadata_usage().charged < retained,
            "cancelled discovery retained temporary permits"
        );
        assert_eq!(
            store.objects.lock().unwrap()["source/child"].bytes,
            b"payload"
        );
        fs.rename(1, "source", 1, "destination").await.unwrap();
        assert_eq!(
            store.objects.lock().unwrap()["destination/child"].bytes,
            b"payload"
        );
    }
}

#[tokio::test]
async fn clean_directory_rename_discovery_error_does_not_publish_intent() {
    for discovering_etag in [false, true] {
        let store = MemoryStore::new();
        store.seed("source/child", b"payload");
        let mut config = MemoryStore::config(32);
        config.metadata_limit = Some(1024 * 1024);
        config.no_dir_object = true;
        config.no_implicit_dir = true;
        let fs = Filesystem::new(config, store.clone()).unwrap();
        let source = fs
            .create(1, "source", libc::S_IFDIR | 0o755, None, 0)
            .await
            .unwrap();
        store
            .omit_list_etag
            .store(discovering_etag, Ordering::Relaxed);
        if !discovering_etag {
            *store.failed_list_prefix.lock().unwrap() = Some("source/".into());
            assert!(fs.rename(1, "source", 1, "destination").await.is_err());
            *store.failed_list_prefix.lock().unwrap() = None;
        } else {
            let (entered, release) = store.pause_list();
            let rename = tokio::spawn({
                let fs = fs.clone();
                async move { fs.rename(1, "source", 1, "destination").await }
            });
            tokio::time::timeout(std::time::Duration::from_secs(5), entered)
                .await
                .unwrap()
                .unwrap();
            store.fail_head_once.store(true, Ordering::Relaxed);
            release.send(()).unwrap();
            assert!(rename.await.unwrap().is_err());
        }
        assert_eq!(store.put_calls.load(Ordering::Relaxed), 0);
        assert_eq!(store.copy_calls.load(Ordering::Relaxed), 0);
        assert_eq!(store.delete_calls.load(Ordering::Relaxed), 0);
        assert!(source.state.lock().unwrap().rename_target.is_none());
        assert!(fs.namespace.lock().unwrap().pending_moves.is_empty());
        assert_eq!(fs.lookup(1, "source").await.unwrap().ino, source.ino);
        assert!(matches!(
            fs.unlink(1, "source", true).await,
            Err(Error::Fs(libc::ENOTEMPTY))
        ));
        fs.rename(1, "source", 1, "destination").await.unwrap();
        assert_eq!(
            store.objects.lock().unwrap()["destination/child"].bytes,
            b"payload"
        );
    }
}

#[tokio::test]
async fn clean_directory_rename_attempted_copy_retains_intent_for_retry() {
    for cancelled in [false, true] {
        let store = MemoryStore::new();
        store.seed("source/child", b"payload");
        let mut config = MemoryStore::config(32);
        config.metadata_limit = Some(1024 * 1024);
        config.no_dir_object = true;
        config.no_implicit_dir = true;
        let fs = Filesystem::new(config, store.clone()).unwrap();
        let source = fs
            .create(1, "source", libc::S_IFDIR | 0o755, None, 0)
            .await
            .unwrap();
        if cancelled {
            let (entered, release) = store.pause_copy();
            let rename = tokio::spawn({
                let fs = fs.clone();
                async move { fs.rename(1, "source", 1, "destination").await }
            });
            tokio::time::timeout(std::time::Duration::from_secs(5), entered)
                .await
                .unwrap()
                .unwrap();
            rename.abort();
            assert!(rename.await.unwrap_err().is_cancelled());
            drop(release);
        } else {
            store.fail_copy_after.store(0, Ordering::Relaxed);
            assert!(fs.rename(1, "source", 1, "destination").await.is_err());
        }
        assert_eq!(store.copy_calls.load(Ordering::Relaxed), 1);
        assert_eq!(store.put_calls.load(Ordering::Relaxed), 0);
        assert_eq!(
            source.state.lock().unwrap().rename_target.as_deref(),
            Some("destination")
        );
        assert_eq!(
            fs.namespace.lock().unwrap().pending_moves.get("source"),
            Some(&source.ino)
        );
        store.fail_copy_after.store(usize::MAX, Ordering::Relaxed);
        fs.rename(1, "source", 1, "destination").await.unwrap();
        assert_eq!(
            store.objects.lock().unwrap()["destination/child"].bytes,
            b"payload"
        );
        assert!(!store.objects.lock().unwrap().contains_key("source/child"));
        assert!(source.state.lock().unwrap().rename_target.is_none());
        assert!(fs.namespace.lock().unwrap().pending_moves.is_empty());
    }
}

#[tokio::test]
async fn empty_local_directory_rename_commits_without_remote_mutation() {
    let store = MemoryStore::new();
    let mut config = MemoryStore::config(32);
    config.metadata_limit = Some(1024 * 1024);
    config.no_dir_object = true;
    config.no_implicit_dir = true;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let source = fs
        .create(1, "source", libc::S_IFDIR | 0o755, None, 0)
        .await
        .unwrap();
    fs.rename(1, "source", 1, "destination").await.unwrap();
    assert_eq!(fs.lookup(1, "destination").await.unwrap().ino, source.ino);
    assert!(matches!(
        fs.lookup(1, "source").await,
        Err(Error::Fs(libc::ENOENT))
    ));
    assert_eq!(store.put_calls.load(Ordering::Relaxed), 0);
    assert_eq!(store.copy_calls.load(Ordering::Relaxed), 0);
    assert_eq!(store.delete_calls.load(Ordering::Relaxed), 0);
    assert!(source.state.lock().unwrap().rename_target.is_none());
    assert!(fs.namespace.lock().unwrap().pending_moves.is_empty());
    fs.unlink(1, "destination", true).await.unwrap();
}

#[tokio::test]
async fn dirty_rename_admission_rejection_keeps_old_name_unlinkable() {
    for directory in [false, true] {
        let store = MemoryStore::new();
        store.seed(
            if directory { "file/" } else { "file" },
            if directory { b"" } else { b"payload" },
        );
        let mut config = MemoryStore::config(32);
        config.metadata_limit = Some(98 * 1024);
        let fs = Filesystem::new(config, store.clone()).unwrap();
        let node = fs.lookup(1, "file").await.unwrap();
        fs.setxattr(node.ino, "user.label", Some(&vec![b'x'; 512]), 0)
            .unwrap();
        if !directory {
            fs.write(node.ino, 2, b"XY").await.unwrap();
        }
        let version = node.state.lock().unwrap().version;
        assert!(matches!(
            fs.rename(1, "file", 1, "destination").await,
            Err(Error::Fs(libc::ENOMEM))
        ));
        assert_eq!(store.put_calls.load(Ordering::Relaxed), 0);
        assert_eq!(store.copy_calls.load(Ordering::Relaxed), 0);
        assert_eq!(store.delete_calls.load(Ordering::Relaxed), 0);
        assert_eq!(fs.lookup(1, "file").await.unwrap().ino, node.ino);
        {
            let state = node.state.lock().unwrap();
            assert!(
                state.rename_target.is_none(),
                "pre-remote admission rejection left a new move intent; directory={directory}"
            );
            assert_eq!(state.version, version);
            assert!(state.durable < version);
            assert_eq!(state.metadata["label"], vec![b'x'; 512]);
        }
        assert!(fs.namespace.lock().unwrap().pending_moves.is_empty());
        assert!(
            fs.metadata_usage().charged < 24 * 1024,
            "failed rename retained temporary permits"
        );
        if !directory {
            assert_eq!(
                fs.read(node.ino, 0, 7).await.unwrap().as_slice(),
                b"paXYoad"
            );
        }
        fs.unlink(1, "file", directory).await.unwrap();
        assert!(!store.objects.lock().unwrap().contains_key(if directory {
            "file/"
        } else {
            "file"
        }));
    }
}

#[tokio::test]
async fn dirty_rename_admission_rejection_preserves_bytes_for_durable_retry() {
    let store = MemoryStore::new();
    store.seed("file", b"payload");
    let mut config = MemoryStore::config(32);
    config.metadata_limit = Some(98 * 1024);
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let node = fs.lookup(1, "file").await.unwrap();
    fs.setxattr(node.ino, "user.label", Some(&vec![b'x'; 512]), 0)
        .unwrap();
    fs.write(node.ino, 2, b"XY").await.unwrap();
    assert!(matches!(
        fs.rename(1, "file", 1, "destination").await,
        Err(Error::Fs(libc::ENOMEM))
    ));
    assert_eq!(store.put_calls.load(Ordering::Relaxed), 0);
    assert_eq!(store.copy_calls.load(Ordering::Relaxed), 0);
    fs.fsync(node.ino).await.unwrap();
    fs.rename(1, "file", 1, "destination").await.unwrap();
    assert_eq!(fs.lookup(1, "destination").await.unwrap().ino, node.ino);
    let objects = store.objects.lock().unwrap();
    assert_eq!(objects["destination"].bytes, b"paXYoad");
    assert_eq!(objects["destination"].metadata["label"], "x".repeat(512));
    assert!(!objects.contains_key("file"));
    assert!(fs.metadata_usage().charged < 24 * 1024);
}

#[tokio::test]
async fn attempted_dirty_flush_keeps_move_intent_on_cancellation_and_retries() {
    let store = MemoryStore::new();
    store.seed("file", b"payload");
    let mut config = MemoryStore::config(32);
    config.metadata_limit = Some(256 * 1024);
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let node = fs.lookup(1, "file").await.unwrap();
    fs.setxattr(node.ino, "user.label", Some(&vec![b'x'; 512]), 0)
        .unwrap();
    fs.write(node.ino, 2, b"XY").await.unwrap();
    let (entered, release) = store.pause_put();
    let rename = tokio::spawn({
        let fs = fs.clone();
        async move { fs.rename(1, "file", 1, "destination").await }
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), entered)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        node.state.lock().unwrap().rename_target.as_deref(),
        Some("destination")
    );
    assert_eq!(
        fs.namespace.lock().unwrap().pending_moves.get("file"),
        Some(&node.ino)
    );
    rename.abort();
    assert!(rename.await.unwrap_err().is_cancelled());
    drop(release);
    assert_eq!(
        node.state.lock().unwrap().rename_target.as_deref(),
        Some("destination")
    );
    assert_eq!(store.copy_calls.load(Ordering::Relaxed), 0);
    fs.rename(1, "file", 1, "destination").await.unwrap();
    assert_eq!(
        store.objects.lock().unwrap()["destination"].bytes,
        b"paXYoad"
    );
    assert_eq!(
        store.objects.lock().unwrap()["destination"].metadata["label"],
        "x".repeat(512)
    );
    assert!(node.state.lock().unwrap().rename_target.is_none());
    assert!(fs.namespace.lock().unwrap().pending_moves.is_empty());
}

#[tokio::test]
async fn applied_dirty_flush_error_keeps_move_intent_until_retry() {
    let store = MemoryStore::new();
    store.seed("file", b"payload");
    let mut config = MemoryStore::config(32);
    config.metadata_limit = Some(256 * 1024);
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let node = fs.lookup(1, "file").await.unwrap();
    fs.setxattr(node.ino, "user.label", Some(&vec![b'x'; 512]), 0)
        .unwrap();
    fs.write(node.ino, 2, b"XY").await.unwrap();
    store
        .fail_head_after_put_once
        .store(true, Ordering::Relaxed);
    assert!(fs.rename(1, "file", 1, "destination").await.is_err());
    assert_eq!(store.put_calls.load(Ordering::Relaxed), 1);
    assert_eq!(store.copy_calls.load(Ordering::Relaxed), 0);
    assert_eq!(store.objects.lock().unwrap()["file"].bytes, b"paXYoad");
    assert_eq!(
        node.state.lock().unwrap().rename_target.as_deref(),
        Some("destination")
    );
    assert_eq!(
        fs.namespace.lock().unwrap().pending_moves.get("file"),
        Some(&node.ino)
    );
    fs.rename(1, "file", 1, "destination").await.unwrap();
    assert_eq!(
        store.objects.lock().unwrap()["destination"].bytes,
        b"paXYoad"
    );
    assert_eq!(
        store.objects.lock().unwrap()["destination"].metadata["label"],
        "x".repeat(512)
    );
    assert!(node.state.lock().unwrap().rename_target.is_none());
    assert!(fs.namespace.lock().unwrap().pending_moves.is_empty());
}

#[tokio::test]
async fn admission_rejection_does_not_remove_preexisting_partial_copy_intent() {
    let store = MemoryStore::new();
    store.seed("file", b"payload");
    let mut config = MemoryStore::config(32);
    config.metadata_limit = Some(98 * 1024);
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let node = fs.lookup(1, "file").await.unwrap();
    let (entered, release) = store.pause_delete(false);
    let rename = tokio::spawn({
        let fs = fs.clone();
        async move { fs.rename(1, "file", 1, "destination").await }
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), entered)
        .await
        .unwrap()
        .unwrap();
    rename.abort();
    assert!(rename.await.unwrap_err().is_cancelled());
    drop(release);
    fs.setxattr(node.ino, "user.label", Some(&vec![b'x'; 512]), 0)
        .unwrap();
    fs.write(node.ino, 2, b"XY").await.unwrap();
    let copies = store.copy_calls.load(Ordering::Relaxed);
    assert!(matches!(
        fs.rename(1, "file", 1, "destination").await,
        Err(Error::Fs(libc::ENOMEM))
    ));
    assert_eq!(store.put_calls.load(Ordering::Relaxed), 0);
    assert_eq!(store.copy_calls.load(Ordering::Relaxed), copies);
    assert_eq!(
        node.state.lock().unwrap().rename_target.as_deref(),
        Some("destination")
    );
    let ns = fs.namespace.lock().unwrap();
    assert_eq!(ns.pending_moves.get("file"), Some(&node.ino));
    assert_eq!(ns.pending_moves.get("destination"), Some(&node.ino));
    assert!(fs.metadata_usage().charged < 24 * 1024);
}

#[tokio::test]
async fn metadata_disabled_rename_preserves_its_existing_head_request_shape() {
    let store = MemoryStore::new();
    store.seed("file", b"payload");
    let fs = Filesystem::new(MemoryStore::config(32), store.clone()).unwrap();
    let _node = fs.lookup(1, "file").await.unwrap();
    store.head_keys.lock().unwrap().clear();
    fs.rename(1, "file", 1, "destination").await.unwrap();
    assert_eq!(
        *store.head_keys.lock().unwrap(),
        vec!["destination", "destination/", "destination"]
    );
    assert_eq!(
        store.objects.lock().unwrap()["destination"].bytes,
        b"payload"
    );
}

#[tokio::test]
async fn percent_xattr_fsync_returns_admission_error_without_panicking() {
    let store = MemoryStore::new();
    store.seed("file", b"payload");
    let mut config = MemoryStore::config(32);
    config.metadata_limit = Some(118 * 1024);
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let node = fs.lookup(1, "file").await.unwrap();
    fs.setxattr(node.ino, "user.label", Some(&vec![b'%'; 22000]), 0)
        .unwrap();
    let task = tokio::spawn({
        let fs = fs.clone();
        let ino = node.ino;
        async move { fs.fsync(ino).await }
    });
    assert!(matches!(
        task.await.expect("fsync must not panic"),
        Err(Error::Fs(libc::ENOMEM))
    ));
    assert_eq!(store.put_calls.load(Ordering::Relaxed), 0);
    assert_eq!(
        node.state.lock().unwrap().metadata["label"],
        vec![b'%'; 22000]
    );
}

#[tokio::test]
async fn percent_xattr_with_sufficient_metadata_admission_is_encoded_and_durable() {
    let store = MemoryStore::new();
    store.seed("file", b"payload");
    let mut config = MemoryStore::config(32);
    config.metadata_limit = Some(2 * 1024 * 1024);
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let node = fs.lookup(1, "file").await.unwrap();
    fs.setxattr(node.ino, "user.label", Some(&vec![b'%'; 22000]), 0)
        .unwrap();
    fs.fsync(node.ino).await.unwrap();
    assert_eq!(
        store.objects.lock().unwrap()["file"].metadata["label"],
        "%25".repeat(22000)
    );
    assert_eq!(
        node.state.lock().unwrap().metadata["label"],
        vec![b'%'; 22000]
    );
}

#[tokio::test]
async fn moved_names_and_retained_xattrs_keep_their_charge() {
    let store = MemoryStore::new();
    let name = "n".repeat(192);
    store.seed(&name, b"payload");
    let mut config = MemoryStore::config(32);
    config.metadata_limit = Some(64 * 1024);
    let fs = Filesystem::new(config, store).unwrap();
    let names: Vec<_> = fs
        .directory_chunk(1, "", 1)
        .await
        .unwrap()
        .into_iter()
        .map(|entry| entry.name)
        .collect();
    let retained = fs.metadata_usage().charged;
    drop(names);
    assert!(
        fs.metadata_usage().charged + 192 <= retained,
        "moved names lost their permit"
    );
    let node = fs.lookup(1, &name).await.unwrap();
    fs.setxattr(node.ino, "user.label", Some(&vec![b'x'; 8192]), 0)
        .unwrap();
    let before = fs.metadata_usage().charged;
    let first = fs.xattrs(node.ino).unwrap();
    let second = fs.xattrs(node.ino).unwrap();
    assert_eq!(first["user.label"], vec![b'x'; 8192]);
    assert_eq!(second["user.label"], vec![b'x'; 8192]);
    assert!(fs.metadata_usage().charged >= before + 16384);
    drop((first, second));
    assert_eq!(fs.metadata_usage().charged, before);
}

#[tokio::test]
async fn rename_metadata_rejection_has_no_remote_side_effects() {
    for directory in [false, true] {
        for kib in 3..30 {
            let store = MemoryStore::new();
            let old = if directory { "source/" } else { "source" };
            store.seed(old, b"payload");
            let mut config = MemoryStore::config(32);
            config.metadata_limit = Some(kib * 1024);
            let fs = Filesystem::new(config, store.clone()).unwrap();
            let Ok(_node) = fs.lookup(1, "source").await else {
                continue;
            };
            let destination = "target-".to_owned() + &"x".repeat(192);
            if matches!(
                fs.rename(1, "source", 1, &destination).await,
                Err(Error::Fs(libc::ENOMEM))
            ) {
                assert_eq!(
                    store.copy_calls.load(Ordering::Relaxed),
                    0,
                    "COPY before admission: {kib} KiB directory={directory}"
                );
                assert_eq!(store.delete_calls.load(Ordering::Relaxed), 0);
                let objects = store.objects.lock().unwrap();
                assert_eq!(objects.len(), 1);
                assert!(objects.contains_key(old));
            }
        }
    }
}

#[tokio::test]
async fn directory_count_admission_resumes_after_pinned_batch() {
    for limit in [None, Some(2 * 1024 * 1024)] {
        let store = MemoryStore::new();
        for index in 0..257 {
            store.seed(&format!("file-{index:03}"), b"payload");
        }
        let mut config = MemoryStore::config(32);
        config.entry_limit = 257;
        config.metadata_limit = limit;
        config.stat_cache_ttl = std::time::Duration::ZERO;
        let fs = Filesystem::new(config, store).unwrap();
        let mut pins = Vec::new();
        for index in 0..256 {
            let node = fs.lookup(1, &format!("file-{index:03}")).await.unwrap();
            if index < 255 {
                pins.push(node);
            }
        }
        let entries = fs.directory_chunk(1, "file-255", 1).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "file-256");
    }
}

#[tokio::test]
async fn directory_rename_retains_charged_working_set_and_releases_it_on_cancellation() {
    let store = MemoryStore::new();
    store.seed("source/", b"");
    let names: Vec<_> = (0..32)
        .map(|index| format!("{index:02}-{}", "n".repeat(189)))
        .collect();
    for name in &names {
        store.seed(&format!("source/{name}"), b"payload");
    }
    let mut config = MemoryStore::config(32);
    config.metadata_limit = Some(1024 * 1024);
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let source = fs.lookup(1, "source").await.unwrap();
    for name in &names {
        fs.lookup(source.ino, name).await.unwrap();
    }
    let baseline = fs.metadata_usage().charged;
    let (entered, release) = store.pause_delete(false);
    let rename = tokio::spawn({
        let fs = fs.clone();
        async move { fs.rename(1, "source", 1, "destination").await }
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), entered)
        .await
        .unwrap()
        .unwrap();
    let retained = fs.metadata_usage().charged;
    assert!(
        retained >= baseline + names.iter().map(String::len).sum::<usize>(),
        "retained subtree name copies escaped admission"
    );
    rename.abort();
    assert!(rename.await.unwrap_err().is_cancelled());
    drop(release);
    assert!(
        fs.metadata_usage().charged < retained,
        "cancelled rename retained its working-set permits"
    );
    fs.rename(1, "source", 1, "destination").await.unwrap();
    let objects = store.objects.lock().unwrap();
    for name in names {
        assert_eq!(objects[&format!("destination/{name}")].bytes, b"payload");
        assert!(!objects.contains_key(&format!("source/{name}")));
    }
}

#[tokio::test]
async fn rename_preflight_admits_new_marker_and_same_etag_metadata_before_copy() {
    for directory in [false, true] {
        let store = MemoryStore::new();
        store.seed(
            if directory { "source/child" } else { "source" },
            b"payload",
        );
        let mut config = MemoryStore::config(32);
        config.metadata_limit = Some(64 * 1024);
        let fs = Filesystem::new(config, store.clone()).unwrap();
        let node = fs.lookup(1, "source").await.unwrap();
        // An implicit directory gains a marker; a file's metadata changes
        // without changing its content ETag while the inode cache is fresh.
        let key = if directory {
            store.seed("source/", b"");
            "source/"
        } else {
            "source"
        };
        store
            .objects
            .lock()
            .unwrap()
            .get_mut(key)
            .unwrap()
            .metadata
            .insert("label".into(), "x".repeat(22000));
        assert!(matches!(
            fs.rename(1, "source", 1, "destination").await,
            Err(Error::Fs(libc::ENOMEM))
        ));
        assert_eq!(store.copy_calls.load(Ordering::Relaxed), 0);
        assert_eq!(store.delete_calls.load(Ordering::Relaxed), 0);
        assert_eq!(store.put_calls.load(Ordering::Relaxed), 0);
        assert_eq!(node.state.lock().unwrap().key, "source");
        assert!(node.state.lock().unwrap().rename_target.is_none());
        assert!(fs.namespace.lock().unwrap().pending_moves.is_empty());
    }
}
