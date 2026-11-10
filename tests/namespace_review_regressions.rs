#![forbid(unsafe_code)]
mod support;

use geesefs_rs::{error::Error, filesystem::Filesystem};
use std::{sync::Arc, sync::atomic::Ordering, time::Duration};
use support::MemoryStore;

fn filesystem(store: &Arc<MemoryStore>) -> Arc<Filesystem> {
    let mut config = MemoryStore::config(32);
    config.stat_cache_ttl = Duration::ZERO;
    Filesystem::new(config, store.clone()).unwrap()
}

async fn older_head_preserves_newer_metadata(getattr: bool) {
    let store = MemoryStore::new();
    store.seed("file", b"unchanged bytes");
    {
        let mut objects = store.objects.lock().unwrap();
        let object = objects.get_mut("file").unwrap();
        object.metadata.insert("mode".into(), "420".into());
        object.metadata.insert("label".into(), "old".into());
    }
    let fs = filesystem(&store);
    let node = fs.lookup(1, "file").await.unwrap();
    let etag = fs.xattrs(node.ino).unwrap()["s3.etag"].to_vec();
    let (seen, release) = store.pause_head_response();
    let older = tokio::spawn({
        let fs = fs.clone();
        let ino = node.ino;
        async move {
            if getattr {
                fs.refresh_inode_if_expired(ino).await
            } else {
                fs.lookup(1, "file").await
            }
        }
    });
    tokio::time::timeout(Duration::from_secs(5), seen)
        .await
        .unwrap()
        .unwrap();
    // A metadata-only S3 replacement need not change the content ETag.
    {
        let mut objects = store.objects.lock().unwrap();
        let object = objects.get_mut("file").unwrap();
        object.metadata.insert("mode".into(), "384".into());
        object.metadata.insert("label".into(), "new".into());
    }
    let newer = fs.refresh_inode_if_expired(node.ino).await.unwrap();
    assert_eq!(newer.attr().perm, 0o600);
    assert_eq!(&fs.xattrs(node.ino).unwrap()["user.label"], b"new");
    assert_eq!(fs.xattrs(node.ino).unwrap()["s3.etag"], etag);
    release.send(()).unwrap();
    let returned = older.await.unwrap().unwrap();
    assert_eq!(returned.ino, node.ino);
    assert_eq!(returned.attr().perm, 0o600);
    assert_eq!(&fs.xattrs(node.ino).unwrap()["user.label"], b"new");
    assert_eq!(store.bytes("file"), b"unchanged bytes");
    assert_eq!(store.put_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn older_lookup_head_does_not_undo_newer_metadata_at_the_same_etag() {
    older_head_preserves_newer_metadata(false).await;
}

#[tokio::test]
async fn older_getattr_head_does_not_undo_newer_metadata_at_the_same_etag() {
    older_head_preserves_newer_metadata(true).await;
}

#[tokio::test]
async fn implicit_directory_discovers_marker_metadata_unless_directory_objects_are_disabled() {
    for no_dir_object in [false, true] {
        let store = MemoryStore::new();
        store.seed("dir/file", b"payload");
        let mut config = MemoryStore::config(32);
        config.stat_cache_ttl = Duration::ZERO;
        config.no_dir_object = no_dir_object;
        let fs = Filesystem::new(config, store.clone()).unwrap();
        let dir = fs.lookup(1, "dir").await.unwrap();
        assert_eq!(dir.attr().perm, 0o755);
        store.seed("dir/", b"");
        {
            let mut objects = store.objects.lock().unwrap();
            let marker = objects.get_mut("dir/").unwrap();
            marker.metadata.insert("mode".into(), "448".into());
            marker.metadata.insert("uid".into(), "123".into());
            marker.metadata.insert("mtime".into(), "1700000000".into());
            marker.metadata.insert("label".into(), "marker".into());
        }
        store.head_keys.lock().unwrap().clear();
        let refreshed = fs.refresh_inode_if_expired(dir.ino).await.unwrap();
        assert_eq!(refreshed.ino, dir.ino);
        if no_dir_object {
            assert_eq!(refreshed.attr().perm, 0o755);
            assert!(!fs.xattrs(dir.ino).unwrap().contains_key("user.label"));
            assert!(store.head_keys.lock().unwrap().is_empty());
        } else {
            assert_eq!(refreshed.attr().perm, 0o700);
            assert_eq!(refreshed.attr().uid, 123);
            assert_eq!(
                refreshed.attr().mtime,
                std::time::UNIX_EPOCH + Duration::from_secs(1_700_000_000)
            );
            assert_eq!(&fs.xattrs(dir.ino).unwrap()["user.label"], b"marker");
            assert_eq!(*store.head_keys.lock().unwrap(), ["dir/"]);
        }
        assert_eq!(store.bytes("dir/file"), b"payload");
        assert_eq!(store.put_calls.load(Ordering::Relaxed), 0);
        assert_eq!(store.delete_calls.load(Ordering::Relaxed), 0);
    }
}

#[tokio::test]
async fn failed_or_cancelled_child_rename_keeps_an_expired_implicit_parent_retryable() {
    for cancelled in [false, true] {
        for getattr in [false, true] {
            let store = MemoryStore::new();
            store.seed("dir/file", b"payload");
            let fs = filesystem(&store);
            let dir = fs.lookup(1, "dir").await.unwrap();
            if cancelled {
                let (seen, _release) = store.pause_delete(true);
                let rename = tokio::spawn({
                    let fs = fs.clone();
                    let parent = dir.ino;
                    async move { fs.rename(parent, "file", 1, "target").await }
                });
                tokio::time::timeout(Duration::from_secs(5), seen)
                    .await
                    .unwrap()
                    .unwrap();
                rename.abort();
                assert!(rename.await.unwrap_err().is_cancelled());
            } else {
                store
                    .delete_applied_error_once
                    .store(true, Ordering::Relaxed);
                assert!(matches!(
                    fs.rename(dir.ino, "file", 1, "target").await,
                    Err(Error::Timeout(_))
                ));
            }
            assert!(!store.objects.lock().unwrap().contains_key("dir/file"));
            assert_eq!(store.bytes("target"), b"payload");
            let refreshed = if getattr {
                fs.refresh_inode_if_expired(dir.ino).await
            } else {
                fs.lookup(1, "dir").await
            }
            .expect("the source parent must survive until its child rename is reconciled");
            assert_eq!(refreshed.ino, dir.ino);
            assert_eq!(
                fs.directory_chunk(dir.ino, "", 8).await.unwrap()[0].name,
                "file"
            );
            fs.rename(dir.ino, "file", 1, "target").await.unwrap();
            fs.shutdown().await.unwrap();
            assert_eq!(store.bytes("target"), b"payload");
            assert!(!store.objects.lock().unwrap().contains_key("dir/file"));
        }
    }
}

#[tokio::test]
async fn rmdir_preserves_both_parent_paths_needed_by_a_pending_rename() {
    for copy_applied in [false, true] {
        let store = MemoryStore::new();
        store.seed("source/", b"");
        store.seed("source/file", b"payload");
        store.seed("destination/", b"");
        let fs = filesystem(&store);
        let source = fs.lookup(1, "source").await.unwrap();
        let destination = fs.lookup(1, "destination").await.unwrap();
        if copy_applied {
            store
                .delete_applied_error_once
                .store(true, Ordering::Relaxed);
        } else {
            store.fail_copy.store(true, Ordering::Relaxed);
        }
        assert!(
            fs.rename(source.ino, "file", destination.ino, "file")
                .await
                .is_err()
        );
        let empty_parent = if copy_applied {
            "source"
        } else {
            "destination"
        };
        let deletes = store.delete_calls.load(Ordering::Relaxed);
        assert!(matches!(
            fs.unlink(1, empty_parent, true).await,
            Err(Error::Fs(libc::EBUSY | libc::ENOTEMPTY))
        ));
        assert_eq!(store.delete_calls.load(Ordering::Relaxed), deletes);
        assert!(
            store
                .objects
                .lock()
                .unwrap()
                .contains_key(&format!("{empty_parent}/"))
        );
        store.fail_copy.store(false, Ordering::Relaxed);
        fs.rename(source.ino, "file", destination.ino, "file")
            .await
            .unwrap();
        fs.shutdown().await.unwrap();
        assert_eq!(store.bytes("destination/file"), b"payload");
        assert!(!store.objects.lock().unwrap().contains_key("source/file"));
    }
}

#[tokio::test]
async fn lost_delete_response_is_retryable_without_parent_refresh() {
    let store = MemoryStore::new();
    store.seed("dir/file", b"payload");
    let fs = filesystem(&store);
    let dir = fs.lookup(1, "dir").await.unwrap();
    store
        .delete_applied_error_once
        .store(true, Ordering::Relaxed);
    assert!(fs.rename(dir.ino, "file", 1, "target").await.is_err());
    fs.rename(dir.ino, "file", 1, "target").await.unwrap();
    fs.shutdown().await.unwrap();
    assert_eq!(store.bytes("target"), b"payload");
}
