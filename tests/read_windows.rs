#![forbid(unsafe_code)]
mod support;
use geesefs_rs::filesystem::Filesystem;
use std::sync::atomic::Ordering;
use support::MemoryStore;
const MIB: usize = 1024 * 1024;

#[tokio::test]
async fn clean_sequential_windows_use_single_owned_buffers_and_whole_parallel_chunks() {
    const STEP: usize = 128 * 1024;
    let expected: Vec<u8> = (0..64 * MIB)
        .map(|offset| ((offset / (64 * 1024)) % 251) as u8)
        .collect();
    let store = MemoryStore::new();
    store.seed("file", &expected);
    let mut config = MemoryStore::config(64);
    config.large_read_cutoff = 0;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let node = fs.lookup(1, "file").await.unwrap();
    for offset in (0..expected.len()).step_by(STEP) {
        let block = fs.read(node.ino, offset as u64, STEP).await.unwrap();
        assert_eq!(block.as_slice(), &expected[offset..offset + STEP]);
        assert!(fs.effective_budgets().data_used_bytes <= fs.effective_budgets().data_shared_bytes);
        if offset == 0 {
            assert_eq!(
                *store.read_ranges.lock().unwrap(),
                [(0, 20 * MIB), (20 * MIB as u64, 20 * MIB)]
            );
            assert_eq!(fs.effective_budgets().data_used_bytes, 40 * MIB + STEP);
        }
    }
    assert_eq!(
        *store.read_ranges.lock().unwrap(),
        [
            (0, 20 * MIB),
            (20 * MIB as u64, 20 * MIB),
            (40 * MIB as u64, 20 * MIB),
            (60 * MIB as u64, 4 * MIB),
        ]
    );
    assert_eq!(fs.effective_budgets().payload_limit_bytes, 64 * MIB);
    assert_eq!(
        store.read_materialization_peak.load(Ordering::Relaxed),
        20 * MIB,
        "a clean 20 MiB GET must not also own a redundant 20 MiB overlay buffer"
    );
}

#[tokio::test]
async fn clean_read_waiting_for_admission_rechecks_a_truncate_regrow_before_using_one_buffer() {
    use std::{future::Future, task::Poll, time::Duration};
    let store = MemoryStore::new();
    store.seed("file", &vec![b'x'; 8 * MIB]);
    store.seed("pin", &vec![b'p'; MIB]);
    let mut config = MemoryStore::config(16);
    config.large_read_cutoff = 0;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let node = fs.lookup(1, "file").await.unwrap();
    let pin = fs.lookup(1, "pin").await.unwrap();
    let pinned = fs.read(pin.ino, 0, MIB).await.unwrap();
    let mut read = Box::pin(fs.read(node.ino, 0, 128 * 1024));
    std::future::poll_fn(|cx| {
        assert!(read.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    fs.truncate(node.ino, (2 * MIB) as u64).unwrap();
    fs.truncate(node.ino, (8 * MIB) as u64).unwrap();
    drop(pinned);
    let block = tokio::time::timeout(Duration::from_secs(3), read)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(block.as_slice(), vec![b'x'; 128 * 1024]);
    let hole = fs
        .read(node.ino, (2 * MIB) as u64, 128 * 1024)
        .await
        .unwrap();
    assert_eq!(hole.as_slice(), vec![0; 128 * 1024]);
    assert!(
        store.read_materialization_peak.load(Ordering::Relaxed)
            <= fs.effective_budgets().data_shared_bytes,
        "GET output plus already allocated overlay scratch must fit the data lane"
    );
    assert!(fs.effective_budgets().data_used_bytes <= fs.effective_budgets().data_shared_bytes);
}

#[tokio::test]
async fn overlapping_large_read_fills_share_work_under_pressure_and_cancelled_waiters_do_not_evict_it()
 {
    use std::{future::Future, sync::Arc, task::Poll, time::Duration};
    const STEP: usize = 128 * 1024;
    let expected: Vec<u8> = (0..64 * MIB)
        .map(|offset| ((offset / (64 * 1024)) % 251) as u8)
        .collect();
    let store = MemoryStore::new();
    store.seed("file", &expected);
    let fs = store.filesystem(64);
    let node = fs.lookup(1, "file").await.unwrap();
    for offset in (0..20 * MIB).step_by(STEP) {
        let block = fs.read(node.ino, offset as u64, STEP).await.unwrap();
        assert_eq!(block.as_slice(), &expected[offset..offset + STEP]);
    }
    assert_eq!(store.read_calls.load(Ordering::Relaxed), 4);
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *store.read_gate.lock().unwrap() = Some(gate.clone());
    let first = tokio::spawn({
        let fs = fs.clone();
        let ino = node.ino;
        async move { fs.read(ino, (20 * MIB) as u64, STEP).await }
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while store.read_calls.load(Ordering::Relaxed) < 7 {
            store.read_started.notified().await;
        }
    })
    .await
    .expect("the promoted window's three EOF-bounded parallel chunks must start");
    let mut cancelled = Box::pin(fs.read(node.ino, (20 * MIB + STEP) as u64, STEP));
    let mut follower = Box::pin(fs.read(node.ino, (20 * MIB + 2 * STEP) as u64, STEP));
    std::future::poll_fn(|cx| {
        assert!(cancelled.as_mut().poll(cx).is_pending());
        assert!(follower.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    drop(cancelled);
    gate.add_permits(16);
    let first = tokio::time::timeout(Duration::from_secs(3), first)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let follower = tokio::time::timeout(Duration::from_secs(3), follower)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.as_slice(), &expected[20 * MIB..20 * MIB + STEP]);
    assert_eq!(
        follower.as_slice(),
        &expected[20 * MIB + 2 * STEP..20 * MIB + 3 * STEP]
    );
    assert_eq!(
        store.read_calls.load(Ordering::Relaxed),
        7,
        "overlapping kernel-sized reads must reuse one promoted window, not evict and reload it: {:?}",
        store.read_ranges.lock().unwrap()
    );
    let budgets = fs.effective_budgets();
    assert_eq!(budgets.payload_limit_bytes, 64 * MIB);
    assert!(budgets.data_used_bytes <= budgets.data_shared_bytes);
}

#[tokio::test]
async fn in_flight_window_publishes_into_the_current_policy_after_other_inode_pressure() {
    use std::{future::Future, sync::Arc, task::Poll, time::Duration};
    const STEP: usize = 128 * 1024;
    let store = MemoryStore::new();
    store.seed("file", &vec![b'x'; 8 * MIB]);
    store.seed("other", &vec![b'y'; MIB / 2]);
    store.seed("pin", &vec![b'p'; MIB / 2]);
    let mut config = MemoryStore::config(16);
    config.large_read_cutoff = 0;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let node = fs.lookup(1, "file").await.unwrap();
    let other = fs.lookup(1, "other").await.unwrap();
    let pin = fs.lookup(1, "pin").await.unwrap();
    let pinned = fs.read(pin.ino, 0, MIB / 2).await.unwrap();
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *store.read_gate.lock().unwrap() = Some(gate.clone());
    let first = tokio::spawn({
        let fs = fs.clone();
        let ino = node.ino;
        async move { fs.read(ino, 0, STEP).await }
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while store.read_calls.load(Ordering::Relaxed) < 2 {
            store.read_started.notified().await;
        }
    })
    .await
    .unwrap();
    let mut competing = Box::pin(fs.read(other.ino, 0, STEP));
    std::future::poll_fn(|cx| {
        assert!(competing.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    gate.add_permits(16);
    let first = tokio::time::timeout(Duration::from_secs(3), first)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(first.as_slice(), vec![b'x'; STEP]);
    drop(first);
    drop(pinned);
    let competing = tokio::time::timeout(Duration::from_secs(3), competing)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(competing.as_slice(), vec![b'y'; STEP]);
    let following = fs.read(node.ino, STEP as u64, STEP).await.unwrap();
    assert_eq!(following.as_slice(), vec![b'x'; STEP]);
    assert_eq!(
        store.read_calls.load(Ordering::Relaxed),
        3,
        "a concurrent policy replacement must not orphan the completed window"
    );
    assert!(fs.effective_budgets().data_used_bytes <= fs.effective_budgets().data_shared_bytes);
}

#[tokio::test]
async fn cancelled_fill_owner_wakes_overlapping_follower_to_retry_without_leaking_budget() {
    use std::{future::Future, sync::Arc, task::Poll, time::Duration};
    let store = MemoryStore::new();
    store.seed("file", &vec![b'r'; 16 * MIB]);
    let fs = store.filesystem(64);
    let node = fs.lookup(1, "file").await.unwrap();
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *store.read_gate.lock().unwrap() = Some(gate.clone());
    let owner = tokio::spawn({
        let fs = fs.clone();
        let ino = node.ino;
        async move { fs.read(ino, 0, 128 * 1024).await }
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while store.read_calls.load(Ordering::Relaxed) < 1 {
            store.read_started.notified().await;
        }
    })
    .await
    .unwrap();
    let mut follower = Box::pin(fs.read(node.ino, 128 * 1024, 128 * 1024));
    std::future::poll_fn(|cx| {
        assert!(follower.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    owner.abort();
    match owner.await {
        Err(error) => assert!(error.is_cancelled()),
        Ok(_) => panic!("fill owner was not cancelled"),
    }
    gate.add_permits(2);
    let block = tokio::time::timeout(Duration::from_secs(3), follower)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(block.as_slice(), vec![b'r'; 128 * 1024]);
    assert_eq!(store.read_calls.load(Ordering::Relaxed), 2);
    assert_eq!(fs.effective_budgets().data_used_bytes, 5 * MIB + 128 * 1024);
}

#[tokio::test]
async fn disjoint_read_fills_of_one_inode_still_fetch_concurrently() {
    use std::{sync::Arc, time::Duration};
    let store = MemoryStore::new();
    store.seed("file", &vec![b'd'; 32 * MIB]);
    let fs = store.filesystem(64);
    let node = fs.lookup(1, "file").await.unwrap();
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *store.read_gate.lock().unwrap() = Some(gate.clone());
    let mut reads = Vec::new();
    for offset in [0, 16 * MIB] {
        let fs = fs.clone();
        let ino = node.ino;
        reads.push(tokio::spawn(async move {
            fs.read(ino, offset as u64, 128 * 1024).await
        }));
    }
    tokio::time::timeout(Duration::from_secs(3), async {
        while store.read_calls.load(Ordering::Relaxed) < 2 {
            store.read_started.notified().await;
        }
    })
    .await
    .expect("disjoint windows must both start before either completes");
    gate.add_permits(2);
    for read in reads {
        let block = tokio::time::timeout(Duration::from_secs(3), read)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(block.as_slice(), vec![b'd'; 128 * 1024]);
    }
}

#[tokio::test]
async fn adjacent_read_windows_fetch_concurrently_in_both_arrival_orders() {
    use std::{sync::Arc, time::Duration};
    const STEP: usize = 128 * 1024;
    let expected: Vec<u8> = (0..2 * MIB)
        .map(|offset| if offset < MIB { b'a' } else { b'b' })
        .collect();
    for offsets in [[0, MIB], [MIB, 0]] {
        let store = MemoryStore::new();
        store.seed("file", &expected);
        let mut config = MemoryStore::config(64);
        config.read_ahead = 1024;
        config.read_ahead_small = 1024;
        config.read_ahead_large = 1024;
        let fs = Filesystem::new(config, store.clone()).unwrap();
        let node = fs.lookup(1, "file").await.unwrap();
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *store.read_gate.lock().unwrap() = Some(gate.clone());
        let mut reads = Vec::new();
        for (index, offset) in offsets.into_iter().enumerate() {
            let fs = fs.clone();
            let ino = node.ino;
            reads.push((
                offset,
                tokio::spawn(async move { fs.read(ino, offset as u64, STEP).await }),
            ));
            tokio::time::timeout(Duration::from_secs(3), async {
                while store.read_calls.load(Ordering::Relaxed) < index + 1 {
                    store.read_started.notified().await;
                }
            })
            .await
            .expect("adjacent windows must both reach the backend before either completes");
        }
        gate.add_permits(2);
        for (offset, read) in reads {
            let block = tokio::time::timeout(Duration::from_secs(3), read)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert_eq!(block.as_slice(), &expected[offset..offset + STEP]);
        }
        assert_eq!(store.read_calls.load(Ordering::Relaxed), 2);
        assert!(fs.effective_budgets().data_used_bytes <= fs.effective_budgets().data_shared_bytes);
    }
}

#[tokio::test]
async fn sequential_128_kib_reads_track_cache_hits_and_keep_adaptive_windows() {
    const STEP: usize = 128 * 1024;
    let size = 32 * MIB;
    let expected: Vec<u8> = (0..size)
        .map(|offset| ((offset / (64 * 1024)) % 251) as u8)
        .collect();
    let store = MemoryStore::new();
    store.seed("file", &expected);
    let fs = store.filesystem(64);
    let node = fs.lookup(1, "file").await.unwrap();
    for offset in (0..size).step_by(STEP) {
        let block = fs.read(node.ino, offset as u64, STEP).await.unwrap();
        assert_eq!(
            block.as_slice(),
            &expected[offset..offset + STEP],
            "sequential bytes at {offset}"
        );
        let budgets = fs.effective_budgets();
        assert!(budgets.data_used_bytes <= budgets.data_shared_bytes);
        assert_eq!(budgets.read_cache_policy_bytes, 60 * MIB);
        assert_eq!(budgets.payload_limit_bytes, 64 * MIB);
    }
    assert!(
        fs.read(node.ino, size as u64, STEP)
            .await
            .unwrap()
            .as_slice()
            .is_empty()
    );
    // Public Go defaults: 5 MiB until 20 MiB of contiguous reads, then the
    // configured large window (bounded here by EOF at 32 MiB). Cache-served
    // kernel-sized requests must not turn one sequential run into small seeks.
    assert_eq!(
        store.read_calls.load(Ordering::Relaxed),
        5,
        "one contiguous run should retain normal windows then promote"
    );
    assert_eq!(
        *store.read_ranges.lock().unwrap(),
        [
            (0, 5 * MIB),
            (5 * MIB as u64, 5 * MIB),
            (10 * MIB as u64, 5 * MIB),
            (15 * MIB as u64, 5 * MIB),
            (20 * MIB as u64, 12 * MIB)
        ]
    );
}

#[tokio::test]
async fn sequential_small_read_promotion_cancels_without_leaking_owners_and_retries_bytes() {
    use std::{sync::Arc, time::Duration};
    const STEP: usize = 128 * 1024;
    let store = MemoryStore::new();
    store.seed("file", &vec![b'x'; 32 * MIB]);
    let fs = store.filesystem(64);
    let node = fs.lookup(1, "file").await.unwrap();
    for offset in (0..20 * MIB).step_by(STEP) {
        let block = fs.read(node.ino, offset as u64, STEP).await.unwrap();
        assert!(block.as_slice().iter().all(|b| *b == b'x'));
    }
    assert_eq!(
        store.read_calls.load(Ordering::Relaxed),
        4,
        "contiguous small reads must retain the initial windows"
    );
    let before = fs.effective_budgets().data_used_bytes;
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *store.read_gate.lock().unwrap() = Some(gate.clone());
    let read = tokio::spawn({
        let fs = fs.clone();
        let ino = node.ino;
        async move { fs.read(ino, (20 * MIB) as u64, STEP).await }
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while store.read_calls.load(Ordering::Relaxed) < 5 {
            store.read_started.notified().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        store.read_ranges.lock().unwrap().last(),
        Some(&((20 * MIB) as u64, 12 * MIB))
    );
    read.abort();
    match read.await {
        Err(e) => assert!(e.is_cancelled()),
        Ok(_) => panic!("promoted read was not cancelled"),
    };
    tokio::time::timeout(Duration::from_secs(3), async {
        while fs.effective_budgets().data_used_bytes != before {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("cancelled adaptive-window scratch and reply owners must be released");
    gate.add_permits(1);
    let block = tokio::time::timeout(
        Duration::from_secs(3),
        fs.read(node.ino, (20 * MIB) as u64, STEP),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(block.as_slice().iter().all(|b| *b == b'x'));
    assert!(fs.effective_budgets().data_used_bytes <= fs.effective_budgets().data_shared_bytes);
}

#[tokio::test]
async fn real_small_seeks_still_reduce_windows_but_one_sample_configuration_disables_history() {
    for count in [4, 1] {
        let store = MemoryStore::new();
        store.seed("file", &vec![b's'; 32 * MIB]);
        let mut config = MemoryStore::config(64);
        config.small_read_count = count;
        let fs = Filesystem::new(config, store.clone()).unwrap();
        let node = fs.lookup(1, "file").await.unwrap();
        for offset in [0, 8 * MIB, 16 * MIB, 24 * MIB] {
            let block = fs.read(node.ino, offset as u64, 64 * 1024).await.unwrap();
            assert!(block.as_slice().iter().all(|b| *b == b's'));
            drop(block);
            assert!(
                fs.read(node.ino, (7 * MIB) as u64, 0)
                    .await
                    .unwrap()
                    .as_slice()
                    .is_empty()
            );
        }
        let tail = if count == 1 { 5 * MIB } else { 128 * 1024 };
        assert_eq!(
            *store.read_ranges.lock().unwrap(),
            [
                (0, 5 * MIB),
                ((8 * MIB) as u64, 5 * MIB),
                ((16 * MIB) as u64, tail),
                ((24 * MIB) as u64, tail)
            ]
        );
    }
}

#[tokio::test]
async fn requested_hundred_mib_window_is_fetched_in_configured_chunks_and_hits_far_offsets() {
    let store = MemoryStore::new();
    store.seed("file", &vec![b'x'; 104 * MIB]);
    let mut config = MemoryStore::config(256);
    config.large_read_cutoff = 0;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let node = fs.lookup(1, "file").await.unwrap();
    assert_eq!(fs.read(node.ino, 0, 1).await.unwrap().as_slice(), b"x");
    assert_eq!(store.read_calls.load(Ordering::Relaxed), 5);
    assert_eq!(
        fs.read(node.ino, 96 * MIB as u64 + 10, 3)
            .await
            .unwrap()
            .as_slice(),
        b"xxx"
    );
    assert_eq!(store.read_calls.load(Ordering::Relaxed), 5);
    let budgets = fs.effective_budgets();
    assert_eq!(budgets.requested_large_window_bytes, 100 * MIB);
    assert_eq!(budgets.read_parallel_chunk_bytes, 20 * MIB);
    assert!(budgets.max_effective_window_bytes >= 100 * MIB);
    assert!(budgets.data_used_bytes <= budgets.data_shared_bytes);
    assert_eq!(budgets.read_cache_policy_bytes, 240 * MIB);
    assert_eq!(budgets.payload_limit_bytes, 256 * MIB);
    serde_json::to_string(&budgets).unwrap();
}

#[tokio::test]
async fn retained_read_cache_can_use_more_than_half_the_shared_data_lane() {
    let store = MemoryStore::new();
    store.seed("file", &vec![b'z'; 12 * MIB]);
    let mut config = MemoryStore::config(32);
    config.read_ahead = 1024;
    config.read_ahead_small = 1024;
    config.read_ahead_parallel = 1024;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let node = fs.lookup(1, "file").await.unwrap();
    for i in 0..12 {
        assert_eq!(
            fs.read(node.ino, (i * MIB) as u64, 1)
                .await
                .unwrap()
                .as_slice(),
            b"z"
        );
    }
    assert_eq!(store.read_calls.load(Ordering::Relaxed), 12);
    assert!(fs.effective_budgets().data_used_bytes > fs.effective_budgets().data_shared_bytes / 2);
    for i in 0..12 {
        assert_eq!(
            fs.read(node.ino, (i * MIB + 10) as u64, 1)
                .await
                .unwrap()
                .as_slice(),
            b"z"
        );
    }
    assert_eq!(store.read_calls.load(Ordering::Relaxed), 12);
}

#[tokio::test]
async fn read_ahead_chunks_run_in_parallel_and_cancellation_releases_reserved_owners() {
    let store = MemoryStore::new();
    store.seed("file", &vec![b'x'; 4 * MIB]);
    let gate = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
    *store.read_gate.lock().unwrap() = Some(gate);
    let mut config = MemoryStore::config(32);
    config.read_ahead = 4096;
    config.read_ahead_parallel = 1024;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let node = fs.lookup(1, "file").await.unwrap();
    let worker = fs.clone();
    let ino = node.ino;
    let task = tokio::spawn(async move { worker.read(ino, 0, 1).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while store.read_calls.load(Ordering::Relaxed) < 4 {
            store.read_started.notified().await;
        }
    })
    .await
    .expect("all requested chunks should be in flight, not serialized behind the first");
    task.abort();
    match task.await {
        Err(error) => assert!(error.is_cancelled()),
        Ok(_) => panic!("read was not cancelled"),
    };
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while fs.effective_budgets().data_used_bytes != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("cancelled prefetch owners must be released");
}

#[tokio::test]
async fn cache_hit_never_pins_the_bytes_needed_by_an_earlier_fifo_miss() {
    use std::{future::Future, pin::Pin, time::Duration};
    async fn pending<F: Future>(mut f: Pin<&mut F>) {
        std::future::poll_fn(|cx| {
            assert!(f.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
    }
    let store = MemoryStore::new();
    for (key, byte, size) in [
        ("pin", b'p', 2 * MIB),
        ("cached", b'c', 2 * MIB),
        ("large", b'l', 4 * MIB),
    ] {
        store.seed(key, &vec![byte; size]);
    }
    let mut config = MemoryStore::config(16);
    config.read_ahead = 4096;
    config.read_ahead_small = 4096;
    config.read_ahead_large = 4096;
    config.read_ahead_parallel = 4096;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    assert_eq!(fs.effective_budgets().data_shared_bytes, 5 * MIB);
    let pin = fs.lookup(1, "pin").await.unwrap();
    let cached = fs.lookup(1, "cached").await.unwrap();
    let large = fs.lookup(1, "large").await.unwrap();
    // This externally owned reply leaves four MiB free after cache eviction.
    let owner = fs.read(pin.ino, 0, MIB).await.unwrap();
    assert!(owner.as_slice().iter().all(|b| *b == b'p'));
    // Queue a three-MiB cache fill, then a full five-MiB large reservation.
    // The latter resets the policy and cannot progress while the fill is pinned.
    let mut fill = Box::pin(fs.read(cached.ino, 0, MIB));
    pending(fill.as_mut()).await;
    let mut older = Box::pin(fs.read(large.ino, 0, MIB));
    pending(older.as_mut()).await;
    drop(owner);
    let reply = tokio::time::timeout(Duration::from_secs(3), fill)
        .await
        .unwrap()
        .unwrap();
    assert!(reply.as_slice().iter().all(|b| *b == b'c'));
    drop(reply);
    assert!(fs.effective_budgets().data_used_bytes <= 5 * MIB);
    // The cache entry arrived after the large waiter's eviction. Old code pins
    // it while queueing a one-MiB output reservation behind that five-MiB waiter.
    tokio::time::timeout(Duration::from_secs(3), async {
        let large_read = async {
            let block = older.await.unwrap();
            assert!(block.as_slice().iter().all(|b| *b == b'l'));
            drop(block);
        };
        let hit_read = async {
            let block = fs.read(cached.ino, 0, MIB).await.unwrap();
            assert!(block.as_slice().iter().all(|b| *b == b'c'));
            drop(block);
        };
        tokio::join!(biased;hit_read,large_read);
    })
    .await
    .expect("a cached read must not self-pin the permits needed by an earlier FIFO miss");
    assert!(fs.effective_budgets().data_used_bytes <= fs.effective_budgets().data_shared_bytes);
}
