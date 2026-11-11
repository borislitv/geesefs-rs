#![forbid(unsafe_code)]
mod support;

use geesefs_rs::{
    budget::Budget,
    disk_cache::{DiskCache, DiskKey, PreparedDiskCache},
    filesystem::Filesystem,
    read::READ_BLOCK_BYTES,
};
use std::{os::unix::fs::PermissionsExt, sync::atomic::Ordering, time::Duration};
use support::MemoryStore;

const MIB: usize = 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(10);

async fn warm_media(
    name: &str,
    first_record: usize,
) -> (
    tempfile::TempDir,
    tempfile::TempDir,
    std::sync::Arc<MemoryStore>,
    Vec<u8>,
) {
    let directory = tempfile::tempdir().unwrap();
    let mount = tempfile::tempdir().unwrap();
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let store = MemoryStore::new();
    *store.cache_namespace.lock().unwrap() = Some([96; 32]);
    let bytes: Vec<_> = (0..4 * MIB)
        .map(|index| (index ^ (index >> 8) ^ (index >> 17)) as u8)
        .collect();
    store.seed(name, &bytes);
    let etag = store.objects.lock().unwrap()[name].etag.clone();
    let media_budget = Budget::new(32 * MIB).unwrap();
    let cache = DiskCache::open(
        PreparedDiskCache::prepare(directory.path(), 16 * MIB as u64).unwrap(),
        &media_budget,
    )
    .await
    .unwrap();
    for offset in (first_record..bytes.len()).step_by(READ_BLOCK_BYTES) {
        let key = DiskKey::new(
            &[96; 32],
            name.as_bytes(),
            etag.as_bytes(),
            bytes.len() as u64,
            offset as u64,
            READ_BLOCK_BYTES,
        )
        .unwrap();
        let block = Budget::new(READ_BLOCK_BYTES)
            .unwrap()
            .try_reserve(READ_BLOCK_BYTES)
            .unwrap()
            .copy(&bytes[offset..offset + READ_BLOCK_BYTES])
            .unwrap();
        assert!(matches!(
            cache.try_demote(key, block),
            geesefs_rs::disk_cache::Demotion::Admitted
        ));
        cache.drain(DEADLINE).await.unwrap();
    }
    cache.close(DEADLINE).await.unwrap();
    drop(cache);
    (directory, mount, store, bytes)
}

#[tokio::test]
async fn four_cache_hot_readers_with_readahead_do_not_fetch_missing_admission_from_source() {
    let (directory, mount, store, bytes) = warm_media("warm.rpm", 0).await;
    for (contention, reply_length) in [
        ("none", 4096),
        ("none", MIB),
        ("namespace", MIB),
        ("inode", MIB),
        ("timeout", MIB),
        ("cancel", MIB),
    ] {
        let mut config = MemoryStore::config(64);
        config.mountpoint = mount.path().into();
        config.disk_cache_dir = Some(directory.path().into());
        config.disk_cache_max_size = Some(16 * MIB as u64);
        config.disk_cache_regex = Some(r"\.rpm$".parse().unwrap());
        config.read_ahead = 1024;
        config.read_ahead_small = 1024;
        config.read_ahead_large = 1024;
        config.read_ahead_parallel = 128;
        let fs = Filesystem::new(config, store.clone()).unwrap();
        fs.attach_disk_cache(
            PreparedDiskCache::prepare(directory.path(), 16 * MIB as u64).unwrap(),
        )
        .await
        .unwrap();
        let ino = fs.lookup(1, "warm.rpm").await.unwrap().ino;
        let baseline = fs.effective_budgets().data_used_bytes;
        let source_gate = (contention == "timeout").then(|| {
            let gate = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
            *store.read_gate.lock().unwrap() = Some(gate.clone());
            gate
        });
        let held_namespace = (contention != "none").then(|| {
            let (ready, started) = std::sync::mpsc::channel();
            let (release, released) = std::sync::mpsc::channel();
            let fs = fs.clone();
            let thread = std::thread::spawn(move || {
                let _held = fs.namespace.lock().unwrap();
                ready.send(()).unwrap();
                released.recv_timeout(DEADLINE).unwrap();
            });
            started.recv_timeout(DEADLINE).unwrap();
            (release, thread)
        });
        let gets = store.read_calls.load(Ordering::Relaxed);
        let mut readers = tokio::task::JoinSet::new();
        for index in 0..4 {
            let fs = fs.clone();
            readers.spawn(async move {
                let block = tokio::time::timeout(
                    DEADLINE,
                    fs.read(ino, (index * MIB) as u64, reply_length),
                )
                .await
                .unwrap()
                .unwrap();
                (index, block)
            });
        }
        if let Some((release, thread)) = held_namespace {
            tokio::time::timeout(DEADLINE, async {
                while fs.disk_read_decisions()
                    [geesefs_rs::read::DiskDecision::NamespaceContended as usize]
                    < if contention == "inode" { 4 } else { 1 }
                {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            if contention == "timeout" {
                tokio::time::timeout(DEADLINE, store.read_started.notified())
                    .await
                    .unwrap();
                assert!(
                    fs.disk_cache_stats().unwrap().active_read_jobs > 0,
                    "deadline does not release actual blocked eligibility owners"
                );
            }
            if contention == "cancel" {
                readers.abort_all();
                while let Some(result) = readers.join_next().await {
                    assert!(matches!(result, Err(error) if error.is_cancelled()));
                }
                assert!(fs.disk_cache_stats().unwrap().active_read_jobs > 0);
                assert!(
                    fs.effective_budgets().data_used_bytes > baseline,
                    "cancelled observer leaves the actual blocking check funded"
                );
                release.send(()).unwrap();
                thread.join().unwrap();
                fs.drain_disk_cache(DEADLINE).await.unwrap();
                assert_eq!(fs.effective_budgets().data_used_bytes, baseline);
                assert_eq!(store.read_calls.load(Ordering::Relaxed) - gets, 0);
                fs.shutdown().await.unwrap();
                continue;
            }
            let held_inode = (contention == "inode").then(|| {
                let node = fs.node(ino).unwrap();
                let (ready, started) = std::sync::mpsc::channel();
                let (release, released) = std::sync::mpsc::channel();
                let thread = std::thread::spawn(move || {
                    let _held = node.state.lock().unwrap();
                    ready.send(()).unwrap();
                    released.recv_timeout(DEADLINE).unwrap();
                });
                started.recv_timeout(DEADLINE).unwrap();
                (release, thread)
            });
            release.send(()).unwrap();
            thread.join().unwrap();
            if let Some(gate) = &source_gate {
                gate.add_permits(64);
            }
            if let Some((release, thread)) = held_inode {
                tokio::time::timeout(DEADLINE, async {
                    while fs.disk_read_decisions()
                        [geesefs_rs::read::DiskDecision::InodeContended as usize]
                        == 0
                    {
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();
                release.send(()).unwrap();
                thread.join().unwrap();
            }
        }
        while let Some(result) = readers.join_next().await {
            let (index, block) = result.unwrap();
            assert_eq!(
                block.as_slice(),
                &bytes[index * MIB..index * MIB + reply_length]
            );
        }
        // Resolve the same public fill intervals before observing counters;
        // short replies must not make still-running readahead disappear.
        for index in 0..4 {
            let block = tokio::time::timeout(DEADLINE, fs.read(ino, (index * MIB) as u64, MIB))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(block.as_slice(), &bytes[index * MIB..(index + 1) * MIB]);
        }
        let stats = fs.disk_cache_stats().unwrap();
        eprintln!("contention={contention} reply_length={reply_length} warm reads: {stats:?}");
        assert_eq!(
            stats.read_operation_misses, 0,
            "population is byte-correct and recovered"
        );
        assert_eq!(stats.read_operation_errors, 0);
        if contention == "timeout" {
            assert!(store.read_calls.load(Ordering::Relaxed) > gets);
            assert!(
                stats.read_fallbacks[geesefs_rs::disk_cache::ReadFallback::Timeout as usize] > 0
            );
            assert!(
                fs.disk_read_decisions()[geesefs_rs::read::DiskDecision::Admission as usize] > 0
            );
            *store.read_gate.lock().unwrap() = None;
        } else {
            assert_eq!(
                store.read_calls.load(Ordering::Relaxed) - gets,
                0,
                "funded warm readers must not turn transient admission contention into source GETs"
            );
        }
        fs.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn parent_rename_starting_during_inode_eligibility_wait_still_bypasses_l2() {
    use geesefs_rs::read::DiskDecision;
    let (directory, mount, store, bytes) = warm_media("dir/warm.rpm", 0).await;
    let mut config = MemoryStore::config(64);
    config.mountpoint = mount.path().into();
    config.disk_cache_dir = Some(directory.path().into());
    config.disk_cache_max_size = Some(16 * MIB as u64);
    config.disk_cache_regex = Some(r"\.rpm$".parse().unwrap());
    config.read_ahead = 1024;
    config.read_ahead_small = 1024;
    config.read_ahead_large = 1024;
    config.read_ahead_parallel = 128;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    fs.attach_disk_cache(PreparedDiskCache::prepare(directory.path(), 16 * MIB as u64).unwrap())
        .await
        .unwrap();
    let dir = fs.lookup(1, "dir").await.unwrap();
    let file = fs.lookup(dir.ino, "warm.rpm").await.unwrap();
    let (ready, started) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel();
    let holder = std::thread::spawn({
        let fs = fs.clone();
        move || {
            let _held = fs.namespace.lock().unwrap();
            ready.send(()).unwrap();
            released.recv_timeout(DEADLINE).unwrap();
        }
    });
    started.recv_timeout(DEADLINE).unwrap();
    let reader = tokio::spawn({
        let fs = fs.clone();
        let ino = file.ino;
        async move { fs.read(ino, 0, MIB).await }
    });
    tokio::time::timeout(DEADLINE, async {
        while fs.disk_read_decisions()[DiskDecision::NamespaceContended as usize] == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let (ready, started) = std::sync::mpsc::channel();
    let (release_inode, released_inode) = std::sync::mpsc::channel();
    let inode_holder = std::thread::spawn(move || {
        let _held = file.state.lock().unwrap();
        ready.send(()).unwrap();
        released_inode.recv_timeout(DEADLINE).unwrap();
    });
    started.recv_timeout(DEADLINE).unwrap();
    release.send(()).unwrap();
    holder.join().unwrap();
    tokio::time::timeout(DEADLINE, async {
        while fs.disk_read_decisions()[DiskDecision::InodeContended as usize] == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let (copy_entered, release_copy) = store.pause_copy();
    let renamer = std::thread::spawn({
        let fs = fs.clone();
        move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    tokio::time::timeout(DEADLINE, fs.rename(1, "dir", 1, "moved"))
                        .await
                        .unwrap()
                })
        }
    });
    // Actual MoveLease publication precedes its preparation of the held child.
    tokio::time::timeout(DEADLINE, async {
        loop {
            let moving = fs.namespace.lock().unwrap().moving.contains("dir");
            if moving {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    release_inode.send(()).unwrap();
    inode_holder.join().unwrap();
    let data = tokio::time::timeout(DEADLINE, reader)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(data.as_slice(), &bytes[..MIB]);
    tokio::time::timeout(DEADLINE, copy_entered)
        .await
        .unwrap()
        .unwrap();
    let gets = store.read_calls.load(Ordering::Relaxed);
    let pending = fs.disk_read_decisions()[DiskDecision::Pending as usize];
    release_copy.send(()).unwrap();
    renamer.join().unwrap().unwrap();
    fs.shutdown().await.unwrap();
    assert!(
        gets > 0,
        "a pending ancestor rename must not reuse the pre-wait namespace decision"
    );
    assert!(pending > 0);
}

#[tokio::test]
async fn a_slow_missing_prefix_does_not_expire_later_independent_disk_attempts() {
    let (directory, mount, store, bytes) = warm_media("warm.rpm", READ_BLOCK_BYTES).await;
    let mut config = MemoryStore::config(64);
    config.mountpoint = mount.path().into();
    config.disk_cache_dir = Some(directory.path().into());
    config.disk_cache_max_size = Some(16 * MIB as u64);
    config.disk_cache_regex = Some(r"\.rpm$".parse().unwrap());
    config.read_ahead = 1024;
    config.read_ahead_small = 1024;
    config.read_ahead_large = 1024;
    config.read_ahead_parallel = 1024;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    fs.attach_disk_cache(PreparedDiskCache::prepare(directory.path(), 16 * MIB as u64).unwrap())
        .await
        .unwrap();
    let ino = fs.lookup(1, "warm.rpm").await.unwrap().ino;
    let gate = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
    *store.read_gate.lock().unwrap() = Some(gate.clone());
    let reader = tokio::spawn({
        let fs = fs.clone();
        async move { fs.read(ino, 0, MIB).await }
    });
    tokio::time::timeout(DEADLINE, store.read_started.notified())
        .await
        .unwrap();
    // A real missing prefix holds the producer in source I/O longer than one
    // optional lookup allowance. Later disk lookups have not started yet.
    assert!(
        tokio::time::timeout(Duration::from_millis(300), std::future::pending::<()>())
            .await
            .is_err()
    );
    gate.add_permits(64);
    let data = tokio::time::timeout(DEADLINE, reader)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(data.as_slice(), &bytes[..MIB]);
    let ranges = store.read_ranges.lock().unwrap().clone();
    let stats = fs.disk_cache_stats().unwrap();
    fs.shutdown().await.unwrap();
    assert_eq!(
        ranges,
        vec![(0, READ_BLOCK_BYTES)],
        "a completed source gap must not age an unrelated optional attempt"
    );
    assert_eq!(
        stats.read_fallbacks[geesefs_rs::disk_cache::ReadFallback::Timeout as usize],
        0
    );
}
