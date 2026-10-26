use std::{sync::Arc, time::Duration};

use geesefs_rs::{budget::Budget, cache::ReadCache};
use tokio::sync::oneshot;
mod support;

#[tokio::test]
async fn pinned_evicted_allocation_remains_charged_until_last_owner_drops() {
    let budget = Budget::new(8).unwrap();
    let cache = ReadCache::new(8);
    let block = budget.reserve(8).await.unwrap().copy(b"abcdefgh").unwrap();
    cache.insert(1, 1, 0, block.clone());
    cache.clear();
    assert!(cache.get(1, 1, 0).is_none());
    assert_eq!(block.as_slice(), b"abcdefgh");
    assert_eq!(budget.used(), 8);
    // Moka reclamation is deferred. Destroy the cache to release its physical
    // references; the externally pinned owner must still keep its byte charge.
    drop(cache);
    assert_eq!(budget.used(), 8);
    drop(block);
    assert_eq!(budget.used(), 0);
}

#[tokio::test]
async fn full_budget_waiter_releases_on_owner_drop_and_cancellation() {
    let budget = Budget::new(4).unwrap();
    let owner = budget.reserve(4).await.unwrap().zeroed();
    let mut waiting = Box::pin(budget.reserve(1));
    // Poll deterministically while all permits are held; no timing/sleep assumptions.
    assert!(
        std::future::poll_fn(|cx| std::task::Poll::Ready(waiting.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    drop(waiting);
    drop(owner);
    assert_eq!(budget.used(), 0);
    let next = budget.reserve(4).await.unwrap().copy(b"next").unwrap();
    assert_eq!(next.as_slice(), b"next");
}

#[tokio::test]
async fn closing_budget_completes_waiters_with_error() {
    let budget = Arc::new(Budget::new(1).unwrap());
    let owner = budget.reserve(1).await.unwrap().zeroed();
    let (started, ready) = oneshot::channel();
    let b = budget.clone();
    let task = tokio::spawn(async move {
        started.send(()).unwrap();
        b.reserve(1).await
    });
    ready.await.unwrap();
    budget.close();
    assert!(
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    drop(owner);
    assert_eq!(budget.used(), 0);
}

#[tokio::test]
async fn oversized_and_mismatched_allocations_do_not_leak_admission() {
    let budget = Budget::new(4).unwrap();
    assert!(budget.reserve(5).await.is_err());
    assert!(budget.reserve(4).await.unwrap().copy(b"bad").is_err());
    assert_eq!(budget.used(), 0);
    assert!(Budget::new(0).is_err());
}

#[tokio::test]
async fn newer_writes_survive_a_concurrent_fsync_generation() {
    let store = support::MemoryStore::new();
    let fs = store.filesystem(32);
    let node = fs
        .create(1, "file", libc::S_IFREG | 0o644, None, 0)
        .await
        .unwrap();
    fs.write(node.ino, 0, b"first").await.unwrap();
    let (entered, release) = store.pause_put();
    let flushing = fs.clone();
    let ino = node.ino;
    let task = tokio::spawn(async move { flushing.fsync(ino).await });
    entered.await.unwrap();
    fs.write(node.ino, 0, b"later").await.unwrap();
    release.send(()).unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(store.bytes("file"), b"first");
    assert_eq!(fs.read(node.ino, 0, 5).await.unwrap().as_slice(), b"later");
    fs.fsync(node.ino).await.unwrap();
    assert_eq!(store.bytes("file"), b"later");
}

#[tokio::test]
async fn shrink_then_grow_during_new_object_flush_does_not_resurrect_tail() {
    let store = support::MemoryStore::new();
    let fs = store.filesystem(32);
    let node = fs
        .create(1, "file", libc::S_IFREG | 0o644, None, 0)
        .await
        .unwrap();
    fs.write(node.ino, 0, b"abcdef").await.unwrap();
    let (entered, release) = store.pause_put();
    let flushing = fs.clone();
    let ino = node.ino;
    let task = tokio::spawn(async move { flushing.fsync(ino).await });
    entered.await.unwrap();
    fs.truncate(node.ino, 2).unwrap();
    fs.truncate(node.ino, 6).unwrap();
    release.send(()).unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(
        fs.read(node.ino, 0, 6).await.unwrap().as_slice(),
        b"ab\0\0\0\0"
    );
    fs.fsync(node.ino).await.unwrap();
    assert_eq!(store.bytes("file"), b"ab\0\0\0\0");
}

#[tokio::test]
async fn full_dirty_budget_has_reserved_flush_progress() {
    let store = support::MemoryStore::new();
    let fs = store.filesystem(16);
    let node = fs
        .create(1, "file", libc::S_IFREG | 0o644, None, 0)
        .await
        .unwrap();
    let block = vec![b'X'; geesefs_rs::write::MAX_IO];
    for i in 0..5 {
        fs.write(node.ino, i * block.len() as u64, &block)
            .await
            .unwrap();
    }
    tokio::time::timeout(
        Duration::from_secs(2),
        fs.write(node.ino, 5 * block.len() as u64, b"Y"),
    )
    .await
    .unwrap()
    .unwrap();
    fs.fsync(node.ino).await.unwrap();
    let bytes = store.bytes("file");
    assert_eq!(bytes.len(), 5 * block.len() + 1);
    assert!(bytes[..5 * block.len()].iter().all(|b| *b == b'X'));
    assert_eq!(bytes[5 * block.len()], b'Y');
}

#[tokio::test]
async fn delete_waits_for_upload_and_old_listing_cannot_resurrect_it() {
    let store = support::MemoryStore::new();
    let fs = store.filesystem(32);
    let node = fs
        .create(1, "file", libc::S_IFREG | 0o644, None, 0)
        .await
        .unwrap();
    fs.write(node.ino, 0, b"data").await.unwrap();
    let (entered, release) = store.pause_put();
    let flushing = fs.clone();
    let ino = node.ino;
    let upload = tokio::spawn(async move { flushing.fsync(ino).await });
    entered.await.unwrap();
    let deleting = fs.clone();
    let deletion = tokio::spawn(async move { deleting.unlink(1, "file", false).await });
    release.send(()).unwrap();
    upload.await.unwrap().unwrap();
    deletion.await.unwrap().unwrap();
    assert!(!store.objects.lock().unwrap().contains_key("file"));
    store.seed("stale", b"old");
    let (listed, continue_list) = store.pause_list();
    let reader = fs.clone();
    let listing = tokio::spawn(async move { reader.directory_chunk(1, "", 128).await });
    listed.await.unwrap();
    fs.unlink(1, "stale", false).await.unwrap();
    continue_list.send(()).unwrap();
    assert!(listing.await.unwrap().is_err());
    assert!(fs.lookup(1, "stale").await.is_err());
}

#[tokio::test]
async fn directory_rename_waits_for_an_inflight_child_commit() {
    let store = support::MemoryStore::new();
    store.seed("old/file", b"before");
    let fs = store.filesystem(32);
    let directory = fs.lookup(1, "old").await.unwrap();
    let child = fs.lookup(directory.ino, "file").await.unwrap();
    fs.write(child.ino, 0, b"AFTER!").await.unwrap();
    let (entered, release) = store.pause_put();
    let flushing = fs.clone();
    let ino = child.ino;
    let flush = tokio::spawn(async move { flushing.fsync(ino).await });
    entered.await.unwrap();
    let mut rename = Box::pin(fs.rename(1, "old", 1, "new"));
    assert!(
        std::future::poll_fn(|cx| std::task::Poll::Ready(rename.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    release.send(()).unwrap();
    flush.await.unwrap().unwrap();
    rename.await.unwrap();
    assert_eq!(store.bytes("new/file"), b"AFTER!");
    assert!(!store.objects.lock().unwrap().contains_key("old/file"));
    assert_eq!(
        fs.read(child.ino, 0, 6).await.unwrap().as_slice(),
        b"AFTER!"
    );
}
#[tokio::test]
async fn cancelled_multipart_releases_owned_upload_and_shutdown_retries_dirty_data() {
    let store = support::MemoryStore::new();
    let fs = store.filesystem(32);
    let node = fs
        .create(1, "large", libc::S_IFREG | 0o644, None, 0)
        .await
        .unwrap();
    let block = vec![b'X'; geesefs_rs::write::MAX_IO];
    for i in 0..6 {
        fs.write(node.ino, i * block.len() as u64, &block)
            .await
            .unwrap();
    }
    let (entered, release) = store.pause_part();
    let flushing = fs.clone();
    let ino = node.ino;
    let task = tokio::spawn(async move { flushing.fsync(ino).await });
    tokio::time::timeout(Duration::from_secs(3), entered)
        .await
        .unwrap()
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    drop(release);
    tokio::time::timeout(Duration::from_secs(3), fs.shutdown())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        store.pending_uploads(),
        0,
        "cancelled upload must be aborted, not orphaned"
    );
    let bytes = store.bytes("large");
    assert_eq!(bytes.len(), 6 * block.len());
    assert!(bytes.iter().all(|b| *b == b'X'));
}
