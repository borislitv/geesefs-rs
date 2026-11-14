#![forbid(unsafe_code)]
mod support;

use geesefs_rs::read::READ_BLOCK_BYTES;
use std::{
    future::Future,
    sync::{Arc, atomic::Ordering},
    task::Poll,
    time::Duration,
};
use support::MemoryStore;

const MIB: usize = 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(10);

async fn pending(mut future: std::pin::Pin<&mut impl Future>) {
    std::future::poll_fn(|cx| {
        assert!(future.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
}

async fn calls(store: &MemoryStore, count: usize) {
    tokio::time::timeout(DEADLINE, async {
        while store.read_calls.load(Ordering::Relaxed) < count {
            store.read_started.notified().await;
        }
    })
    .await
    .expect("expected source reads must start before the gate is released");
}

#[tokio::test]
async fn sequential_l2_reads_preserve_adaptive_windows_without_sliding_the_speculative_tail() {
    let bytes: Vec<u8> = (0..32 * MIB)
        .map(|at| (at / READ_BLOCK_BYTES % 251) as u8)
        .collect();
    let store = MemoryStore::new();
    store.seed("file", &bytes);
    let (fs, _directory) = store.disk_filesystem(MemoryStore::config(64)).await;
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    for offset in (0..bytes.len()).step_by(READ_BLOCK_BYTES) {
        let reply = fs.read(ino, offset as u64, READ_BLOCK_BYTES).await.unwrap();
        assert_eq!(reply.as_slice(), &bytes[offset..offset + READ_BLOCK_BYTES]);
        drop(reply);
        fs.drain_disk_cache(DEADLINE).await.unwrap();
        let budget = fs.effective_budgets();
        assert_eq!(budget.data_used_bytes, budget.disk_cache_funding_bytes);
        assert_eq!(budget.flush_used_bytes, 0);
    }
    assert!(
        fs.read(ino, bytes.len() as u64, READ_BLOCK_BYTES)
            .await
            .unwrap()
            .as_slice()
            .is_empty()
    );
    // The configured default promotes after 20 MiB of contiguous caller reads,
    // including the requests served from Foyer rather than the source.
    assert_eq!(
        *store.read_ranges.lock().unwrap(),
        [
            (0, 5 * MIB),
            (5 * MIB as u64, 5 * MIB),
            (10 * MIB as u64, 5 * MIB),
            (15 * MIB as u64, 5 * MIB),
            (20 * MIB as u64, 12 * MIB),
        ]
    );
    fs.shutdown().await.unwrap();
}

#[tokio::test]
async fn small_seeks_reduce_persistent_windows_but_one_sample_disables_history() {
    for count in [4, 1] {
        let store = MemoryStore::new();
        store.seed("file", &vec![b's'; 32 * MIB]);
        let mut config = MemoryStore::config(64);
        config.small_read_count = count;
        let (fs, _directory) = store.disk_filesystem(config).await;
        let ino = fs.lookup(1, "file").await.unwrap().ino;
        for offset in [0, 8 * MIB, 16 * MIB, 24 * MIB] {
            let reply = fs.read(ino, offset as u64, 64 * 1024).await.unwrap();
            assert_eq!(reply.as_slice(), &[b's'; 64 * 1024]);
            drop(reply);
            fs.drain_disk_cache(DEADLINE).await.unwrap();
        }
        let tail = if count == 1 {
            5 * MIB
        } else {
            READ_BLOCK_BYTES
        };
        assert_eq!(
            *store.read_ranges.lock().unwrap(),
            [
                (0, 5 * MIB),
                (8 * MIB as u64, 5 * MIB),
                (16 * MIB as u64, tail),
                (24 * MIB as u64, tail),
            ]
        );
        fs.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn oversized_requested_prefetch_is_bounded_and_uses_whole_parallel_chunks() {
    let store = MemoryStore::new();
    store.seed("file", &vec![b'x'; 128 * MIB]);
    let mut config = MemoryStore::config(256);
    config.large_read_cutoff = 0;
    config.read_ahead_large = 200 * 1024;
    let (fs, _directory) = store.disk_filesystem(config).await;
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    assert_eq!(fs.read(ino, 0, 1).await.unwrap().as_slice(), b"x");
    fs.drain_disk_cache(DEADLINE).await.unwrap();
    let mut ranges = store.read_ranges.lock().unwrap().clone();
    ranges.sort_unstable();
    assert_eq!(
        ranges,
        [
            (0, 20 * MIB),
            (20 * MIB as u64, 20 * MIB),
            (40 * MIB as u64, 20 * MIB),
            (60 * MIB as u64, 20 * MIB),
            (80 * MIB as u64, 20 * MIB),
        ]
    );
    assert_eq!(
        fs.read(ino, 2 * MIB as u64 + 10, 3)
            .await
            .unwrap()
            .as_slice(),
        b"xxx"
    );
    assert_eq!(store.read_calls.load(Ordering::Relaxed), 5);
    let budget = fs.effective_budgets();
    assert_eq!(budget.requested_large_window_bytes, 200 * MIB);
    assert!(budget.max_effective_window_bytes < 200 * MIB);
    assert_eq!(budget.data_used_bytes, budget.disk_cache_funding_bytes);
    fs.shutdown().await.unwrap();
}

#[tokio::test]
async fn parallel_prefetch_cancellation_releases_owners_and_required_retry_does_not_redownload_the_tail()
 {
    let store = MemoryStore::new();
    store.seed("file", &vec![b'x'; 4 * MIB]);
    let mut config = MemoryStore::config(32);
    config.read_ahead = 4096;
    config.read_ahead_parallel = 1024;
    let (fs, _directory) = store.disk_filesystem(config).await;
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *store.read_gate.lock().unwrap() = Some(gate.clone());
    let task = tokio::spawn({
        let fs = fs.clone();
        async move { fs.read(ino, 0, 1).await }
    });
    calls(&store, 4).await;
    task.abort();
    assert!(matches!(task.await, Err(error) if error.is_cancelled()));
    let funding = fs.effective_budgets().disk_cache_funding_bytes;
    tokio::time::timeout(DEADLINE, async {
        while fs.effective_budgets().data_used_bytes != funding {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    gate.add_permits(1);
    assert_eq!(fs.read(ino, 0, 1).await.unwrap().as_slice(), b"x");
    assert_eq!(store.read_ranges.lock().unwrap().last(), Some(&(0, 1)));
    fs.shutdown().await.unwrap();
}

#[tokio::test]
async fn overlapping_active_readers_share_one_persistent_window_and_cancelled_followers_release_it()
{
    let store = MemoryStore::new();
    let bytes: Vec<u8> = (0..MIB).map(|at| (at % 251) as u8).collect();
    store.seed("file", &bytes);
    let mut config = MemoryStore::config(32);
    config.read_ahead = 1024;
    config.read_ahead_parallel = 1024;
    let (fs, _directory) = store.disk_filesystem(config).await;
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *store.read_gate.lock().unwrap() = Some(gate.clone());
    let leader = tokio::spawn({
        let fs = fs.clone();
        async move { fs.read(ino, 0, READ_BLOCK_BYTES).await }
    });
    calls(&store, 1).await;
    let mut cancelled = Box::pin(fs.read(ino, READ_BLOCK_BYTES as u64, READ_BLOCK_BYTES));
    let mut follower = Box::pin(fs.read(ino, (2 * READ_BLOCK_BYTES) as u64, READ_BLOCK_BYTES));
    pending(cancelled.as_mut()).await;
    pending(follower.as_mut()).await;
    drop(cancelled);
    gate.add_permits(1);
    let reply = tokio::time::timeout(DEADLINE, follower)
        .await
        .unwrap()
        .unwrap();
    let first = leader.await.unwrap().unwrap();
    assert_eq!(
        reply.as_slice(),
        &bytes[2 * READ_BLOCK_BYTES..3 * READ_BLOCK_BYTES]
    );
    assert_eq!(first.as_slice(), &bytes[..READ_BLOCK_BYTES]);
    assert_eq!(*store.read_ranges.lock().unwrap(), [(0, MIB)]);
    drop(reply);
    drop(first);
    fs.drain_disk_cache(DEADLINE).await.unwrap();
    assert_eq!(
        fs.effective_budgets().data_used_bytes,
        fs.effective_budgets().disk_cache_funding_bytes
    );
    fs.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancelled_producer_wakes_a_genuinely_overlapping_reader_without_leaking_payload() {
    let store = MemoryStore::new();
    store.seed("file", &vec![b'r'; MIB]);
    let fs = store.filesystem(32);
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *store.read_gate.lock().unwrap() = Some(gate.clone());
    let owner = tokio::spawn({
        let fs = fs.clone();
        async move { fs.read(ino, 0, MIB).await }
    });
    calls(&store, 1).await;
    let mut follower = Box::pin(fs.read(ino, READ_BLOCK_BYTES as u64, READ_BLOCK_BYTES));
    pending(follower.as_mut()).await;
    owner.abort();
    assert!(matches!(owner.await, Err(error) if error.is_cancelled()));
    gate.add_permits(1);
    let reply = tokio::time::timeout(DEADLINE, follower)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reply.as_slice(), &[b'r'; READ_BLOCK_BYTES]);
    assert_eq!(
        *store.read_ranges.lock().unwrap(),
        [(0, MIB), (READ_BLOCK_BYTES as u64, READ_BLOCK_BYTES)]
    );
    drop(reply);
    assert_eq!(fs.effective_budgets().data_used_bytes, 0);
}

#[tokio::test]
async fn disjoint_ranges_of_one_inode_start_concurrently() {
    let store = MemoryStore::new();
    store.seed("file", &vec![b'd'; 4 * MIB]);
    let fs = store.filesystem(32);
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *store.read_gate.lock().unwrap() = Some(gate.clone());
    let reads: Vec<_> = [0, 2 * MIB]
        .into_iter()
        .map(|offset| {
            let fs = fs.clone();
            tokio::spawn(async move { fs.read(ino, offset as u64, READ_BLOCK_BYTES).await })
        })
        .collect();
    calls(&store, 2).await;
    gate.add_permits(2);
    for read in reads {
        assert_eq!(
            read.await.unwrap().unwrap().as_slice(),
            &[b'd'; READ_BLOCK_BYTES]
        );
    }
    assert_eq!(fs.effective_budgets().data_used_bytes, 0);
}

#[tokio::test]
async fn a_fill_follower_cannot_pin_payload_while_queueing_output_behind_an_older_large_read() {
    let store = MemoryStore::new();
    for (name, byte) in [
        ("file", b'f'),
        ("pin-a", b'p'),
        ("pin-b", b'p'),
        ("dirty", b'd'),
    ] {
        store.seed(name, &vec![byte; MIB]);
    }
    let fs = store.filesystem(16);
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *store.read_gate.lock().unwrap() = Some(gate.clone());
    let leader = tokio::spawn({
        let fs = fs.clone();
        async move { fs.read(ino, 0, MIB).await }
    });
    calls(&store, 1).await;
    let mut follower = Box::pin(fs.read(ino, 0, MIB));
    pending(follower.as_mut()).await;
    gate.add_permits(1);
    drop(leader.await.unwrap().unwrap());
    *store.read_gate.lock().unwrap() = None;
    // External owners are legitimate pins. Only the follower's unpublished
    // source owner can be released to let the older three-buffer plan proceed.
    let mut pins = Vec::new();
    for name in ["pin-a", "pin-b"] {
        let ino = fs.lookup(1, name).await.unwrap().ino;
        pins.push(fs.read(ino, 0, MIB).await.unwrap());
    }
    let dirty = fs.lookup(1, "dirty").await.unwrap().ino;
    fs.write(dirty, 0, b"N").await.unwrap();
    let mut older = Box::pin(fs.read(dirty, 0, 3 * MIB / 4));
    pending(older.as_mut()).await;
    tokio::time::timeout(Duration::from_secs(3), async {
        let reuse = async {
            let reply = follower.await.unwrap();
            assert_eq!(reply.as_slice(), &vec![b'f'; MIB]);
            drop(reply);
        };
        let large = async {
            let reply = older.await.unwrap();
            assert_eq!(reply.as_slice()[0], b'N');
            assert_eq!(&reply.as_slice()[1..], &vec![b'd'; 3 * MIB / 4 - 1]);
        };
        tokio::join!(biased; reuse, large);
    })
    .await
    .expect("a follower must not retain the bytes needed by the older FIFO admission");
    drop(pins);
    fs.shutdown().await.unwrap();
    assert_eq!(fs.effective_budgets().data_used_bytes, 0);
}
