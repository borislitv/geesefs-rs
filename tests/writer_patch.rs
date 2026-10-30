#![forbid(unsafe_code)]
mod support;
use geesefs_rs::{filesystem::Filesystem, write::PART_SIZE};
use std::sync::atomic::Ordering;
use std::{sync::Arc, time::Duration};
use support::MemoryStore;
fn patch_fs(store: &Arc<MemoryStore>, drop_conflicts: bool) -> Arc<Filesystem> {
    let mut config = MemoryStore::config(32);
    config.enable_patch = true;
    config.drop_patch_conflicts = drop_conflicts;
    Filesystem::new(config, store.clone()).unwrap()
}
async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), future)
        .await
        .expect("deterministic writer operation deadline")
}
#[tokio::test]
async fn replacement_between_commit_and_head_never_retires_our_dirty_generation() {
    let store = MemoryStore::new();
    let fs = store.filesystem(32);
    let node = fs
        .create(1, "file", libc::S_IFREG | 0o644, None, 0)
        .await
        .unwrap();
    fs.write(node.ino, 0, b"ours").await.unwrap();
    store.overwrite_after_put.store(true, Ordering::Relaxed);
    assert!(
        fs.fsync(node.ino).await.is_err(),
        "a same-size external generation must not acknowledge our commit"
    );
    assert_eq!(fs.read(node.ino, 0, 4).await.unwrap().as_slice(), b"ours");
    fs.fsync(node.ino).await.unwrap();
    assert_eq!(store.bytes("file"), b"ours");
}

#[tokio::test]
async fn patch_overwrites_appends_and_persists_metadata_without_losing_newer_writes() {
    let store = MemoryStore::new();
    store.seed("file", b"abcdef");
    let fs = patch_fs(&store, false);
    let node = fs.lookup(1, "file").await.unwrap();
    fs.write(node.ino, 2, b"ZZ").await.unwrap();
    let (entered, release) = store.pause_patch_response();
    let worker = fs.clone();
    let ino = node.ino;
    let task = tokio::spawn(async move { worker.fsync(ino).await });
    bounded(entered).await.unwrap();
    fs.write(ino, 3, b"X").await.unwrap();
    release.send(()).unwrap();
    bounded(task).await.unwrap().unwrap();
    assert_eq!(store.bytes("file"), b"abZZef");
    assert_eq!(fs.read(ino, 0, 6).await.unwrap().as_slice(), b"abZXef");
    fs.fsync(ino).await.unwrap();
    fs.write(ino, 8, b"!").await.unwrap();
    fs.fsync(ino).await.unwrap();
    assert_eq!(store.bytes("file"), b"abZXef\0\0!");
    fs.setxattr(ino, "user.extra", Some(b"value"), 0).unwrap();
    fs.fsync(ino).await.unwrap();
    let fresh = store.filesystem(32);
    let reopened = fresh.lookup(1, "file").await.unwrap();
    assert_eq!(fresh.xattrs(reopened.ino).unwrap()["user.extra"], b"value");
    assert_eq!(store.patch_calls.load(Ordering::Relaxed), 3);
}

#[tokio::test]
async fn partial_patch_failure_advances_only_the_remote_baseline_and_is_retryable() {
    let store = MemoryStore::new();
    store.seed("file", &vec![b'a'; PART_SIZE + 8]);
    store
        .objects
        .lock()
        .unwrap()
        .get_mut("file")
        .unwrap()
        .etag
        .push_str("-2");
    let fs = patch_fs(&store, false);
    let node = fs.lookup(1, "file").await.unwrap();
    fs.write(node.ino, 0, b"ONE").await.unwrap();
    fs.write(node.ino, (PART_SIZE + 3) as u64, b"TWO")
        .await
        .unwrap();
    store.fail_patch_after.store(1, Ordering::Relaxed);
    assert!(fs.fsync(node.ino).await.is_err());
    assert_eq!(&store.bytes("file")[..3], b"ONE");
    assert_eq!(
        fs.read(node.ino, (PART_SIZE + 3) as u64, 3)
            .await
            .unwrap()
            .as_slice(),
        b"TWO"
    );
    store.fail_patch_after.store(usize::MAX, Ordering::Relaxed);
    fs.fsync(node.ino).await.unwrap();
    let bytes = store.bytes("file");
    assert_eq!(&bytes[..3], b"ONE");
    assert_eq!(&bytes[PART_SIZE + 3..PART_SIZE + 6], b"TWO");
    assert!(bytes[3..PART_SIZE + 3].iter().all(|b| *b == b'a'));
}

#[tokio::test]
async fn dirty_ranges_crossing_part_boundaries_are_split_without_changing_neighbor_bytes() {
    let store = MemoryStore::new();
    store.seed("file", &vec![b'a'; PART_SIZE + 4]);
    let fs = patch_fs(&store, false);
    let node = fs.lookup(1, "file").await.unwrap();
    fs.write(node.ino, (PART_SIZE - 2) as u64, b"WXYZ")
        .await
        .unwrap();
    fs.fsync(node.ino).await.unwrap();
    assert_eq!(
        *store.patch_ranges.lock().unwrap(),
        [((PART_SIZE - 2) as u64, 2), (PART_SIZE as u64, 2)]
    );
    let bytes = store.bytes("file");
    assert_eq!(&bytes[PART_SIZE - 3..PART_SIZE + 3], b"aWXYZa");
}

#[tokio::test]
async fn dropping_a_conflict_preserves_writes_newer_than_its_snapshot() {
    let store = MemoryStore::new();
    store.seed("file", b"abcdef");
    let fs = patch_fs(&store, true);
    let node = fs.lookup(1, "file").await.unwrap();
    fs.write(node.ino, 2, b"ZZ").await.unwrap();
    store.seed("file", b"remote");
    let (entered, release) = store.pause_head();
    let worker = fs.clone();
    let ino = node.ino;
    let task = tokio::spawn(async move { worker.fsync(ino).await });
    bounded(entered).await.unwrap();
    fs.write(ino, 3, b"X").await.unwrap();
    release.send(()).unwrap();
    assert!(bounded(task).await.unwrap().is_err());
    assert_eq!(fs.read(ino, 0, 6).await.unwrap().as_slice(), b"remXte");
    fs.fsync(ino).await.unwrap();
    assert_eq!(store.bytes("file"), b"remXte");
}

#[tokio::test]
async fn retained_conflicts_refresh_the_baseline_and_merge_unmodified_remote_bytes_and_metadata() {
    let store = MemoryStore::new();
    store.seed("file", b"abcdef");
    store
        .objects
        .lock()
        .unwrap()
        .get_mut("file")
        .unwrap()
        .metadata
        .insert("shared".into(), "before".into());
    let fs = patch_fs(&store, false);
    let node = fs.lookup(1, "file").await.unwrap();
    fs.write(node.ino, 2, b"ZZ").await.unwrap();
    fs.setxattr(node.ino, "user.local", Some(b"ours"), 0)
        .unwrap();
    store.seed("file", b"REMOTE++");
    store
        .objects
        .lock()
        .unwrap()
        .get_mut("file")
        .unwrap()
        .metadata
        .insert("shared".into(), "after".into());
    assert!(matches!(
        fs.fsync(node.ino).await,
        Err(geesefs_rs::error::Error::Fs(libc::EBUSY))
    ));
    assert_eq!(
        fs.read(node.ino, 0, 8).await.unwrap().as_slice(),
        b"REZZTE++"
    );
    fs.fsync(node.ino).await.unwrap();
    assert_eq!(store.bytes("file"), b"REZZTE++");
    let fresh = store.filesystem(32);
    let node = fresh.lookup(1, "file").await.unwrap();
    let attrs = fresh.xattrs(node.ino).unwrap();
    assert_eq!(attrs["user.shared"], b"after");
    assert_eq!(attrs["user.local"], b"ours");
}

#[tokio::test]
async fn lost_patch_response_or_cancellation_never_blindly_repeats_an_append() {
    for cancel in [false, true] {
        let store = MemoryStore::new();
        store.seed("file", b"abc");
        let fs = patch_fs(&store, false);
        let node = fs.lookup(1, "file").await.unwrap();
        fs.write(node.ino, 3, b"def").await.unwrap();
        if cancel {
            let (entered, _release) = store.pause_patch_response();
            let worker = fs.clone();
            let ino = node.ino;
            let task = tokio::spawn(async move { worker.fsync(ino).await });
            bounded(entered).await.unwrap();
            task.abort();
            assert!(bounded(task).await.unwrap_err().is_cancelled());
        } else {
            store.patch_lost_response.store(true, Ordering::Relaxed);
            assert!(fs.fsync(node.ino).await.is_err());
        }
        assert_eq!(store.bytes("file"), b"abcdef");
        assert!(fs.fsync(node.ino).await.is_err());
        assert_eq!(store.bytes("file"), b"abcdef");
        assert_eq!(fs.read(node.ino, 3, 3).await.unwrap().as_slice(), b"def");
        fs.fsync(node.ino).await.unwrap();
        assert_eq!(store.bytes("file"), b"abcdef");
    }
}

#[tokio::test]
async fn empty_truncated_and_unsupported_patch_paths_reupload_without_losing_bytes() {
    let store = MemoryStore::new();
    store.seed("file", b"abcdef");
    let fs = patch_fs(&store, false);
    let node = fs.lookup(1, "file").await.unwrap();
    fs.truncate(node.ino, 2).unwrap();
    fs.truncate(node.ino, 6).unwrap();
    fs.fsync(node.ino).await.unwrap();
    assert_eq!(store.bytes("file"), b"ab\0\0\0\0");
    assert_eq!(store.patch_calls.load(Ordering::Relaxed), 0);
    store.patch_unsupported.store(true, Ordering::Relaxed);
    fs.write(node.ino, 0, b"Z").await.unwrap();
    fs.fsync(node.ino).await.unwrap();
    assert_eq!(store.bytes("file"), b"Zb\0\0\0\0");
    assert_eq!(store.patch_calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn multipart_writer_supplies_the_full_snapshot_size_not_part_size() {
    let store = MemoryStore::new();
    let fs = store.filesystem(32);
    let node = fs
        .create(1, "large", libc::S_IFREG | 0o644, None, 0)
        .await
        .unwrap();
    fs.truncate(node.ino, (PART_SIZE + 3) as u64).unwrap();
    fs.write(node.ino, PART_SIZE as u64, b"end").await.unwrap();
    fs.fsync(node.ino).await.unwrap();
    assert_eq!(
        *store.upload_sizes.lock().unwrap(),
        [(PART_SIZE + 3) as u64]
    );
    let bytes = store.bytes("large");
    assert!(bytes[..PART_SIZE].iter().all(|b| *b == 0));
    assert_eq!(&bytes[PART_SIZE..], b"end");
}

#[tokio::test]
async fn failed_directory_rename_blocks_child_flush_until_retry_moves_its_key() {
    let store = MemoryStore::new();
    store.seed("tree/a", b"old");
    store.seed("tree/b", b"two");
    store.page_size.store(1, Ordering::Relaxed);
    let fs = store.filesystem(32);
    let directory = fs.lookup(1, "tree").await.unwrap();
    let child = fs.lookup(directory.ino, "a").await.unwrap();
    fs.write(child.ino, 0, b"NEW").await.unwrap();
    store.fail_copy_after.store(1, Ordering::Relaxed);
    assert!(fs.rename(1, "tree", 1, "moved").await.is_err());
    assert!(
        fs.create(directory.ino, "new-child", libc::S_IFREG | 0o644, None, 0)
            .await
            .is_err()
    );
    assert!(fs.unlink(directory.ino, "b", false).await.is_err());
    assert!(!store.objects.lock().unwrap().contains_key("tree/a"));
    assert!(
        fs.fsync(child.ino).await.is_err(),
        "a failed parent rename must not recreate its drained source child"
    );
    assert!(!store.objects.lock().unwrap().contains_key("tree/a"));
    store.fail_copy_after.store(usize::MAX, Ordering::Relaxed);
    fs.rename(1, "tree", 1, "moved").await.unwrap();
    fs.fsync(child.ino).await.unwrap();
    assert_eq!(store.bytes("moved/a"), b"NEW");
    assert!(!store.objects.lock().unwrap().contains_key("tree/a"));
}

#[tokio::test]
async fn prefer_patch_bootstraps_two_parts_and_finishes_a_new_sparse_tail() {
    for unsupported in [false, true] {
        let store = MemoryStore::new();
        store
            .patch_unsupported
            .store(unsupported, Ordering::Relaxed);
        let mut config = MemoryStore::config(32);
        config.enable_patch = true;
        config.prefer_patch_uploads = true;
        let fs = Filesystem::new(config, store.clone()).unwrap();
        let node = fs
            .create(1, "new", libc::S_IFREG | 0o644, None, 0)
            .await
            .unwrap();
        fs.write(node.ino, (2 * PART_SIZE) as u64, b"end")
            .await
            .unwrap();
        fs.fsync(node.ino).await.unwrap();
        let bytes = store.bytes("new");
        assert_eq!(bytes.len(), 2 * PART_SIZE + 3);
        assert!(bytes[..2 * PART_SIZE].iter().all(|b| *b == 0));
        assert_eq!(&bytes[2 * PART_SIZE..], b"end");
        assert_eq!(store.patch_calls.load(Ordering::Relaxed), 1);
        assert_eq!(
            store.upload_sizes.lock().unwrap().len(),
            if unsupported { 2 } else { 1 }
        );
    }
}

#[tokio::test]
async fn stale_directory_drain_pages_stop_without_repeating_remote_copies() {
    let store = MemoryStore::new();
    store.seed("tree/a", b"old");
    let fs = store.filesystem(32);
    fs.lookup(1, "tree").await.unwrap();
    store.retain_deleted.store(true, Ordering::Relaxed);
    store.fail_copy_after.store(2, Ordering::Relaxed);
    assert!(fs.rename(1, "tree", 1, "moved").await.is_err());
    assert_eq!(
        store.copy_calls.load(Ordering::Relaxed),
        1,
        "a repeated drain page must fail before recopying already-processed objects"
    );
}

#[tokio::test]
async fn applied_delete_with_a_lost_response_keeps_the_copied_base_readable_and_rename_retryable() {
    let store = MemoryStore::new();
    store.seed("old", b"abcdef");
    let fs = store.filesystem(32);
    let node = fs.lookup(1, "old").await.unwrap();
    store
        .delete_applied_error_once
        .store(true, Ordering::Relaxed);
    assert!(fs.rename(1, "old", 1, "new").await.is_err());
    assert!(!store.objects.lock().unwrap().contains_key("old"));
    assert_eq!(fs.read(node.ino, 0, 6).await.unwrap().as_slice(), b"abcdef");
    fs.write(node.ino, 2, b"ZZ").await.unwrap();
    fs.rename(1, "old", 1, "new").await.unwrap();
    assert_eq!(store.bytes("new"), b"abZZef");
    assert_eq!(store.copy_calls.load(Ordering::Relaxed), 1);
    assert_eq!(fs.lookup(1, "new").await.unwrap().ino, node.ino);
}
