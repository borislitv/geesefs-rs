#![forbid(unsafe_code)]
mod support;
use geesefs_rs::{error::Error, filesystem::Filesystem};
use std::future::Future;
use std::{sync::atomic::Ordering, time::Duration};
use support::MemoryStore;

fn fs(store: &std::sync::Arc<MemoryStore>) -> std::sync::Arc<Filesystem> {
    let mut c = MemoryStore::config(32);
    c.stat_cache_ttl = Duration::ZERO;
    Filesystem::new(c, store.clone()).unwrap()
}
async fn pending<F: Future>(future: std::pin::Pin<&mut F>) {
    let mut future = future;
    std::future::poll_fn(|cx| {
        assert!(
            future.as_mut().poll(cx).is_pending(),
            "operation must wait for the controlled gate"
        );
        std::task::Poll::Ready(())
    })
    .await;
}

#[tokio::test]
async fn stale_lookup_head_cannot_replace_a_new_successfully_flushed_generation() {
    let store = MemoryStore::new();
    store.seed("file", b"old");
    let fs = fs(&store);
    let node = fs.lookup(1, "file").await.unwrap();
    fs.open(node.ino, false).unwrap();
    fs.reference(&node);
    let (seen, release) = store.pause_head_response();
    let lookup = tokio::spawn({
        let fs = fs.clone();
        async move { fs.lookup(1, "file").await }
    });
    seen.await.unwrap();
    fs.write(node.ino, 0, b"newer").await.unwrap();
    fs.fsync(node.ino).await.unwrap();
    release.send(()).unwrap();
    let _ = lookup.await.unwrap();
    assert_eq!(node.attr().size, 5);
    assert_eq!(fs.read(node.ino, 0, 5).await.unwrap().as_slice(), b"newer");
    assert_eq!(node.state.lock().unwrap().opens, 1);
    assert_eq!(node.state.lock().unwrap().lookups, 1);
}

#[tokio::test]
async fn committed_multipart_generation_and_cancelled_verification_remain_retryable() {
    for cancelled in [false, true] {
        let store = MemoryStore::new();
        let size = 6 * 1024 * 1024;
        store.seed("file", &vec![b'a'; size]);
        let fs = store.filesystem(32);
        let node = fs.lookup(1, "file").await.unwrap();
        fs.write(node.ino, 2, b"XY").await.unwrap();
        if cancelled {
            let (seen, _release) = store.pause_head_response();
            let flush = tokio::spawn({
                let fs = fs.clone();
                let ino = node.ino;
                async move { fs.fsync(ino).await }
            });
            seen.await.unwrap();
            flush.abort();
            assert!(flush.await.unwrap_err().is_cancelled());
        } else {
            store
                .fail_head_after_complete_once
                .store(true, Ordering::Relaxed);
            assert!(fs.fsync(node.ino).await.is_err());
        }
        assert_eq!(fs.read(node.ino, 0, 6).await.unwrap().as_slice(), b"aaXYaa");
        assert_eq!(
            fs.read(node.ino, size as u64 - 1, 1)
                .await
                .unwrap()
                .as_slice(),
            b"a"
        );
        fs.fsync(node.ino).await.unwrap();
        let bytes = store.bytes("file");
        assert_eq!(&bytes[..6], b"aaXYaa");
        assert!(bytes[6..].iter().all(|b| *b == b'a'));
    }
}

#[tokio::test]
async fn cancelled_delete_reconciliation_is_bounded_and_shutdown_does_not_flush_old_dirty_data() {
    let store = MemoryStore::new();
    store.seed("file", b"old");
    let fs = store.filesystem(32);
    let node = fs.lookup(1, "file").await.unwrap();
    fs.write(node.ino, 0, b"dirty").await.unwrap();
    let (seen, _release) = store.pause_delete(true);
    let delete = tokio::spawn({
        let fs = fs.clone();
        async move { fs.unlink(1, "file", false).await }
    });
    seen.await.unwrap();
    delete.abort();
    assert!(delete.await.unwrap_err().is_cancelled());
    assert_eq!(fs.read(node.ino, 0, 5).await.unwrap().as_slice(), b"dirty");
    fs.shutdown().await.unwrap();
    assert!(!store.objects.lock().unwrap().contains_key("file"));
    assert_eq!(store.put_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn standard_list_resolves_unknown_type_before_returning_a_safe_fuser_dirent() {
    let store = MemoryStore::new();
    store.standard_listing.store(true, Ordering::Relaxed);
    store.seed("link", b"");
    store.seed("fifo", b"");
    {
        let mut objects = store.objects.lock().unwrap();
        objects
            .get_mut("link")
            .unwrap()
            .metadata
            .insert("--symlink-target".into(), "target".into());
        objects
            .get_mut("fifo")
            .unwrap()
            .metadata
            .insert("mode".into(), format!("{}", libc::S_IFIFO | 0o600));
    }
    let mut config = MemoryStore::config(32);
    config.enable_specials = true;
    let fs = Filesystem::new(config.clone(), store.clone()).unwrap();
    let mut entries = fs.directory_chunk(1, "", 1).await.unwrap();
    assert_eq!(store.head_keys.lock().unwrap().len(), 1);
    entries.extend(fs.directory_chunk(1, "fifo", 1).await.unwrap());
    assert_eq!(store.head_keys.lock().unwrap().len(), 2);
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries
            .iter()
            .map(|e| (e.name.as_str(), e.kind))
            .collect::<Vec<_>>(),
        [
            ("fifo", fuser::FileType::NamedPipe),
            ("link", fuser::FileType::Symlink)
        ]
    );
    assert_eq!(
        fs.lookup(1, "link").await.unwrap().attr().kind,
        fuser::FileType::Symlink
    );
    assert_eq!(
        fs.lookup(1, "fifo").await.unwrap().attr().kind,
        fuser::FileType::NamedPipe
    );
    assert_eq!(store.head_keys.lock().unwrap().len(), 2);
    store.standard_listing.store(false, Ordering::Relaxed);
    store.head_keys.lock().unwrap().clear();
    let fresh = Filesystem::new(config, store.clone()).unwrap();
    let extended = fresh.directory_chunk(1, "", 8).await.unwrap();
    assert_eq!(
        extended
            .iter()
            .map(|e| (e.name.as_str(), e.kind))
            .collect::<Vec<_>>(),
        [
            ("fifo", fuser::FileType::NamedPipe),
            ("link", fuser::FileType::Symlink)
        ]
    );
    assert!(store.head_keys.lock().unwrap().is_empty());
    store.standard_listing.store(true, Ordering::Relaxed);
    store.seed("dir/child", b"child");
    assert!(matches!(
        fresh.unlink(1, "dir", true).await,
        Err(Error::Fs(libc::ENOTEMPTY))
    ));
    assert_eq!(store.bytes("dir/child"), b"child");
}

#[tokio::test]
async fn authoritative_directory_listing_drops_expired_clean_names_but_keeps_dirty_children() {
    let store = MemoryStore::new();
    store.seed("dir/", b"");
    store.seed("dir/gone", b"old");
    let fs = fs(&store);
    let dir = fs.lookup(1, "dir").await.unwrap();
    let old = fs.lookup(dir.ino, "gone").await.unwrap();
    store.objects.lock().unwrap().remove("dir/gone");
    assert!(fs.directory_chunk(dir.ino, "", 8).await.unwrap().is_empty());
    fs.unlink(1, "dir", true).await.unwrap();
    assert!(fs.lookup(1, "dir").await.is_err());
    assert_eq!(old.attr().size, 3);
    store.seed("implicit/file", b"old");
    let parent = fs.lookup(1, "implicit").await.unwrap();
    let child = fs.lookup(parent.ino, "file").await.unwrap();
    fs.write(child.ino, 0, b"dirty").await.unwrap();
    store.objects.lock().unwrap().remove("implicit/file");
    assert_eq!(
        fs.directory_chunk(1, "", 8)
            .await
            .unwrap()
            .iter()
            .map(|e| e.name.as_str())
            .collect::<Vec<_>>(),
        ["implicit"]
    );
    assert_eq!(
        fs.directory_chunk(parent.ino, "", 8)
            .await
            .unwrap()
            .iter()
            .map(|e| e.name.as_str())
            .collect::<Vec<_>>(),
        ["file"]
    );
}

#[tokio::test]
async fn create_probe_cannot_publish_under_a_renamed_or_deleted_parent() {
    for rename in [false, true] {
        let store = MemoryStore::new();
        store.seed("dir/", b"");
        let fs = store.filesystem(32);
        let dir = fs.lookup(1, "dir").await.unwrap();
        let (seen, release) = store.pause_list();
        let create = tokio::spawn({
            let fs = fs.clone();
            async move {
                fs.create(dir.ino, "late", libc::S_IFREG | 0o600, None, 0)
                    .await
            }
        });
        seen.await.unwrap();
        if rename {
            fs.rename(1, "dir", 1, "moved").await.unwrap();
        } else {
            fs.unlink(1, "dir", true).await.unwrap();
        }
        release.send(()).unwrap();
        assert!(
            create.await.unwrap().is_err(),
            "obsolete parent must not gain a child"
        );
        assert!(!fs.namespace.lock().unwrap().names.contains_key("dir/late"));
        fs.flush_all().await.unwrap();
        assert!(!store.objects.lock().unwrap().contains_key("dir/late"));
    }
}

#[tokio::test]
async fn rmdir_reserves_the_directory_before_its_remote_empty_probe() {
    let store = MemoryStore::new();
    store.seed("dir/", b"");
    let fs = store.filesystem(32);
    let dir = fs.lookup(1, "dir").await.unwrap();
    let (seen, release) = store.pause_list();
    let delete = tokio::spawn({
        let fs = fs.clone();
        async move { fs.unlink(1, "dir", true).await }
    });
    seen.await.unwrap();
    let create = fs
        .create(dir.ino, "child", libc::S_IFREG | 0o600, None, 0)
        .await;
    assert!(
        create.is_err(),
        "child creation must not cross the emptiness reservation"
    );
    release.send(()).unwrap();
    delete.await.unwrap().unwrap();
    assert!(!store.objects.lock().unwrap().contains_key("dir/"));
}

#[tokio::test]
async fn second_unlink_waiter_never_deletes_a_recreated_same_name_object() {
    let store = MemoryStore::new();
    store.seed("file", b"old");
    let fs = store.filesystem(32);
    let node = fs.lookup(1, "file").await.unwrap();
    fs.write(node.ino, 0, b"old").await.unwrap();
    let (seen, release) = store.pause_put();
    let flush = tokio::spawn({
        let fs = fs.clone();
        async move { fs.fsync(node.ino).await }
    });
    seen.await.unwrap();
    let mut first = Box::pin(fs.unlink(1, "file", false));
    pending(first.as_mut()).await;
    let mut second = Box::pin(fs.unlink(1, "file", false));
    pending(second.as_mut()).await;
    release.send(()).unwrap();
    flush.await.unwrap().unwrap();
    first.await.unwrap();
    let new = fs
        .create(1, "file", libc::S_IFREG | 0o600, None, 0)
        .await
        .unwrap();
    fs.write(new.ino, 0, b"new").await.unwrap();
    fs.fsync(new.ino).await.unwrap();
    assert!(second.await.is_err());
    assert_eq!(store.bytes("file"), b"new");
    assert_eq!(store.delete_calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn cancelled_delete_is_reconcilable_without_losing_dirty_ownership_or_deleting_a_replacement()
{
    for applied in [false, true] {
        let store = MemoryStore::new();
        store.seed("file", b"old");
        let fs = store.filesystem(32);
        let node = fs.lookup(1, "file").await.unwrap();
        fs.write(node.ino, 0, b"dirty").await.unwrap();
        let (seen, _release) = store.pause_delete(applied);
        let delete = tokio::spawn({
            let fs = fs.clone();
            async move { fs.unlink(1, "file", false).await }
        });
        seen.await.unwrap();
        delete.abort();
        assert!(delete.await.unwrap_err().is_cancelled());
        {
            let state = node.state.lock().unwrap();
            assert!(state.version > state.durable);
        }
        assert!(
            fs.create(1, "file", libc::S_IFREG | 0o600, None, 0)
                .await
                .is_err()
        );
        fs.unlink(1, "file", false).await.unwrap();
        let replacement = fs
            .create(1, "file", libc::S_IFREG | 0o600, None, 0)
            .await
            .unwrap();
        fs.write(replacement.ino, 0, b"replacement").await.unwrap();
        fs.fsync(replacement.ino).await.unwrap();
        assert_eq!(store.bytes("file"), b"replacement");
    }
}

#[tokio::test]
async fn known_commit_survives_failed_verification_and_remains_conditionally_readable_and_retryable()
 {
    let store = MemoryStore::new();
    store.seed("file", b"abcdef");
    let fs = store.filesystem(32);
    let node = fs.lookup(1, "file").await.unwrap();
    fs.write(node.ino, 2, b"XY").await.unwrap();
    store
        .fail_head_after_put_once
        .store(true, Ordering::Relaxed);
    assert!(fs.fsync(node.ino).await.is_err());
    assert_eq!(fs.read(node.ino, 0, 6).await.unwrap().as_slice(), b"abXYef");
    fs.fsync(node.ino).await.unwrap();
    assert_eq!(store.bytes("file"), b"abXYef");
}

#[tokio::test]
async fn root_attribute_changes_are_rejected_without_creating_an_empty_object_key() {
    let store = MemoryStore::new();
    let fs = store.filesystem(32);
    assert!(matches!(
        fs.setattr(1, Some(0o700), None, None, None),
        Err(Error::Fs(libc::EOPNOTSUPP))
    ));
    fs.flush_all().await.unwrap();
    assert_eq!(store.put_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn s3_last_modified_is_the_stable_fallback_and_configured_mtime_has_precedence() {
    for enable in [false, true] {
        let store = MemoryStore::new();
        store.seed("file", b"data");
        let stamp = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        *store.last_modified.lock().unwrap() = Some(stamp);
        let mut c = MemoryStore::config(32);
        c.enable_mtime = enable;
        c.stat_cache_ttl = Duration::ZERO;
        let fs = Filesystem::new(c, store.clone()).unwrap();
        let node = fs.lookup(1, "file").await.unwrap();
        assert_eq!(node.attr().mtime, stamp);
        assert_eq!(fs.lookup(1, "file").await.unwrap().attr().mtime, stamp);
        store
            .objects
            .lock()
            .unwrap()
            .get_mut("file")
            .unwrap()
            .metadata
            .insert("mtime".into(), "123".into());
        let refreshed = fs.refresh_inode_if_expired(node.ino).await.unwrap();
        assert_eq!(
            refreshed.attr().mtime,
            if enable {
                std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(123)
            } else {
                stamp
            }
        );
    }
}

#[tokio::test]
async fn getattr_refresh_keeps_identity_open_refs_and_dirty_state_and_handles_implicit_dirs() {
    let store = MemoryStore::new();
    store.seed("file", b"old");
    store.seed("implicit/child", b"x");
    let fs = fs(&store);
    let node = fs.lookup(1, "file").await.unwrap();
    fs.open(node.ino, true).unwrap();
    fs.reference(&node);
    store.seed("file", b"replacement");
    let refreshed = fs.refresh_inode_if_expired(node.ino).await.unwrap();
    assert!(std::sync::Arc::ptr_eq(&node, &refreshed));
    assert_eq!(refreshed.attr().size, 11);
    assert_eq!(
        fs.read(node.ino, 0, 11).await.unwrap().as_slice(),
        b"replacement"
    );
    {
        let s = node.state.lock().unwrap();
        assert_eq!((s.opens, s.lookups), (1, 1));
    }
    fs.write(node.ino, 0, b"dirty").await.unwrap();
    let heads = store.head_keys.lock().unwrap().len();
    fs.refresh_inode_if_expired(node.ino).await.unwrap();
    fs.refresh_inode_if_expired(1).await.unwrap();
    assert_eq!(store.head_keys.lock().unwrap().len(), heads);
    assert_eq!(
        fs.read(node.ino, 0, 11).await.unwrap().as_slice(),
        b"dirtycement"
    );
    let dir = fs.lookup(1, "implicit").await.unwrap();
    let heads = store.head_keys.lock().unwrap().len();
    let lists = store.list_calls.load(Ordering::Relaxed);
    fs.refresh_inode_if_expired(dir.ino).await.unwrap();
    assert_eq!(store.head_keys.lock().unwrap().len(), heads);
    assert_eq!(store.list_calls.load(Ordering::Relaxed), lists + 1);
    store.objects.lock().unwrap().remove("implicit/child");
    assert!(matches!(
        fs.refresh_inode_if_expired(dir.ino).await,
        Err(Error::Fs(libc::ENOENT))
    ));
}

#[tokio::test]
async fn create_uses_request_ownership_only_with_enable_perms_and_does_not_invent_setgid_inheritance()
 {
    for enabled in [false, true] {
        let store = MemoryStore::new();
        let mut c = MemoryStore::config(32);
        c.enable_perms = enabled;
        c.uid = Some(800);
        c.gid = Some(900);
        let fs = Filesystem::new(c.clone(), store.clone()).unwrap();
        let dir = fs
            .create(1, "dir", libc::S_IFDIR | 0o2775, None, 0)
            .await
            .unwrap();
        if enabled {
            fs.setattr(dir.ino, None, None, Some(777), None).unwrap();
        }
        let node = fs
            .create_with_identity(dir.ino, "child", libc::S_IFREG | 0o600, None, 0, (123, 456))
            .await
            .unwrap();
        assert_eq!(
            (node.attr().uid, node.attr().gid),
            if enabled { (123, 456) } else { (800, 900) }
        );
        fs.fsync(node.ino).await.unwrap();
        let fresh = Filesystem::new(c, store).unwrap();
        let dir = fresh.lookup(1, "dir").await.unwrap();
        let child = fresh.lookup(dir.ino, "child").await.unwrap();
        assert_eq!(
            (child.attr().uid, child.attr().gid),
            if enabled { (123, 456) } else { (800, 900) }
        );
    }
}
