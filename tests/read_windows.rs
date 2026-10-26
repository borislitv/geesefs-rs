#![forbid(unsafe_code)]
mod support;
use geesefs_rs::filesystem::Filesystem;
use std::sync::atomic::Ordering;
use support::MemoryStore;
const MIB: usize = 1024 * 1024;

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
    assert_eq!(budgets.read_cache_policy_bytes, budgets.data_shared_bytes);
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
    for (key, byte) in [("pin", b'p'), ("cached", b'c'), ("large", b'l')] {
        store.seed(key, &vec![byte; 2 * MIB]);
    }
    let mut config = MemoryStore::config(16);
    config.read_ahead = 2048;
    config.read_ahead_small = 2048;
    config.read_ahead_large = 2048;
    config.read_ahead_parallel = 2048;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    assert_eq!(fs.effective_budgets().data_shared_bytes, 5 * MIB);
    let pin = fs.lookup(1, "pin").await.unwrap();
    let cached = fs.lookup(1, "cached").await.unwrap();
    let large = fs.lookup(1, "large").await.unwrap();
    // This externally owned reply leaves four MiB free after cache eviction.
    let owner = fs.read(pin.ino, 0, MIB).await.unwrap();
    assert!(owner.as_slice().iter().all(|b| *b == b'p'));
    // Queue two full five-MiB reservations in this order. Neither has a cache
    // handle yet; the second resets the policy before the first is admitted.
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
