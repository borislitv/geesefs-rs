#![forbid(unsafe_code)]
mod support;

use geesefs_rs::{disk_cache::PreparedDiskCache, filesystem::Filesystem, read::READ_BLOCK_BYTES};
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use support::MemoryStore;

const DEADLINE: Duration = Duration::from_secs(10);

#[tokio::test]
async fn clean_reads_release_all_payload_after_the_last_consumer() {
    let store = MemoryStore::new();
    let expected = vec![0x5a; READ_BLOCK_BYTES];
    store.seed("file", &expected);
    let fs = store.filesystem(32);
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    let block = fs.read(ino, 7, 4096).await.unwrap();
    assert_eq!(block.as_slice(), &expected[7..4103]);
    assert!(fs.effective_budgets().data_used_bytes > 0);
    drop(block);
    assert_eq!(
        fs.effective_budgets().data_used_bytes,
        0,
        "completed clean reads must not leave a retained application payload cache"
    );
    assert_eq!(
        fs.read(ino, 7, 4096).await.unwrap().as_slice(),
        &expected[7..4103]
    );
    assert_eq!(
        store.read_calls.load(Ordering::Relaxed),
        2,
        "without L2 a new independent read must fetch its bytes again"
    );
    assert_eq!(
        *store.read_ranges.lock().unwrap(),
        [(7, 4096), (7, 4096)],
        "no speculative tail may be fetched when no persistent tier can retain it"
    );
    fs.shutdown().await.unwrap();
}

#[tokio::test]
async fn validated_fill_populates_fresh_l2_without_capacity_pressure() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let mount = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let store = MemoryStore::new();
    *store.cache_namespace.lock().unwrap() = Some([0x47; 32]);
    let expected = vec![0xc3; READ_BLOCK_BYTES];
    store.seed("file.rpm", &expected);
    let mut config = MemoryStore::config(32);
    config.mountpoint = mount.path().into();
    config.disk_cache_dir = Some(dir.path().into());
    config.disk_cache_max_size = Some(16 * 1024 * 1024);
    config.disk_cache_regex = Some(r"\.rpm$".parse().unwrap());
    config.read_ahead = 128;
    config.read_ahead_small = 128;
    config.read_ahead_large = 128;
    let prepared =
        PreparedDiskCache::prepare(dir.path(), config.disk_cache_max_size.unwrap()).unwrap();
    let fs = Filesystem::new(config, store.clone()).unwrap();
    fs.attach_disk_cache(prepared).await.unwrap();
    let ino = fs.lookup(1, "file.rpm").await.unwrap().ino;
    assert_eq!(
        fs.read(ino, 0, READ_BLOCK_BYTES).await.unwrap().as_slice(),
        expected
    );
    fs.drain_disk_cache(DEADLINE).await.unwrap();
    assert!(
        fs.disk_cache_stats()
            .is_some_and(|stats| stats.admitted_demotions > 0 && stats.io_write_bytes > 0),
        "a fresh persistent tier must start and receive the first validated source fill"
    );
    assert_eq!(
        fs.effective_budgets().data_used_bytes,
        fs.effective_budgets().disk_cache_funding_bytes
    );
    let gets = store.read_calls.load(Ordering::Relaxed);
    let disk_bytes = fs.disk_cache_read_bytes();
    assert_eq!(
        fs.read(ino, 13, 4096).await.unwrap().as_slice(),
        &expected[13..4109]
    );
    assert_eq!(store.read_calls.load(Ordering::Relaxed), gets);
    assert!(
        fs.disk_cache_read_bytes() > disk_bytes,
        "an independent reread must use validated L2, not a retained decoded RAM copy"
    );
    fs.shutdown().await.unwrap();
}

#[tokio::test]
async fn overlapping_active_readers_share_bytes_without_retaining_them_afterwards() {
    use std::{future::Future, task::Poll};
    let store = MemoryStore::new();
    let expected = vec![0x6b; READ_BLOCK_BYTES];
    store.seed("file", &expected);
    let fs = store.filesystem(32);
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *store.read_gate.lock().unwrap() = Some(gate.clone());
    let leader = tokio::spawn({
        let fs = fs.clone();
        async move { fs.read(ino, 0, READ_BLOCK_BYTES).await }
    });
    tokio::time::timeout(DEADLINE, async {
        while store.read_calls.load(Ordering::Relaxed) == 0 {
            store.read_started.notified().await;
        }
    })
    .await
    .unwrap();
    let mut follower = Box::pin(fs.read(ino, 17, 4096));
    std::future::poll_fn(|cx| {
        assert!(follower.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    gate.add_permits(2);
    let following = tokio::time::timeout(DEADLINE, follower)
        .await
        .unwrap()
        .unwrap();
    let first = tokio::time::timeout(DEADLINE, leader)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(first.as_slice(), expected);
    assert_eq!(following.as_slice(), &expected[17..4113]);
    assert_eq!(store.read_calls.load(Ordering::Relaxed), 1);
    drop(first);
    drop(following);
    assert_eq!(fs.effective_budgets().data_used_bytes, 0);
    fs.shutdown().await.unwrap();
}
