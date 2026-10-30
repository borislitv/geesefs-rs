#![forbid(unsafe_code)]
mod support;

use geesefs_rs::{error::Error, filesystem::Filesystem};
use std::{sync::Arc, time::Duration};
use support::MemoryStore;
use tokio::sync::{Semaphore, oneshot};

const MIB: usize = 1024 * 1024;
const STEP: usize = 128 * 1024;

fn fixture() -> (Arc<MemoryStore>, Arc<Filesystem>, Vec<u8>) {
    let expected: Vec<u8> = (0..40 * MIB).map(|at| (at % 251) as u8).collect();
    let store = MemoryStore::new();
    store.seed("file", &expected);
    let mut config = MemoryStore::config(64);
    config.large_read_cutoff = 0;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    (store, fs, expected)
}

fn gate(store: &MemoryStore, offset: u64) -> Arc<Semaphore> {
    let gate = Arc::new(Semaphore::new(0));
    store
        .read_range_gates
        .lock()
        .unwrap()
        .insert((offset, 20 * MIB), gate.clone());
    gate
}

async fn finish(load: tokio::task::JoinHandle<Option<Error>>) -> Option<Error> {
    tokio::time::timeout(Duration::from_secs(3), load)
        .await
        .expect("the caller-owned prefetch must complete after its gates are released")
        .unwrap()
}

#[tokio::test]
async fn requested_reply_is_published_before_unrelated_prefetch_finishes() {
    let (store, fs, expected) = fixture();
    let node = fs.lookup(1, "file").await.unwrap();
    let ino = node.ino;
    let tail = gate(&store, 20 * MIB as u64);
    let (ready, mut received) = oneshot::channel();
    let load = tokio::spawn({
        let fs = fs.clone();
        async move {
            fs.read_reply(ino, 0, STEP, move |result| {
                let _ = ready.send(result);
            })
            .await
        }
    });
    let outcome = tokio::time::timeout(Duration::from_secs(3), &mut received).await;
    if outcome.is_err() {
        load.abort();
    }
    let block = outcome
        .expect("the requested range must be published before unrelated prefetch completes")
        .unwrap()
        .unwrap();
    assert_eq!(block.as_slice(), &expected[..STEP]);
    assert!(
        !load.is_finished(),
        "remaining prefetch must stay caller-owned"
    );
    let hit = fs.read(node.ino, MIB as u64 + 3, 4096).await.unwrap();
    assert_eq!(hit.as_slice(), &expected[MIB + 3..MIB + 3 + 4096]);
    tail.add_permits(1);
    assert!(
        tokio::time::timeout(Duration::from_secs(3), load)
            .await
            .unwrap()
            .unwrap()
            .is_none()
    );
    drop(block);
    drop(hit);
    assert_eq!(fs.effective_budgets().data_used_bytes, 40 * MIB);
    assert_eq!(store.read_ranges.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn a_requested_dependency_failure_is_published_as_an_error() {
    let (store, fs, _) = fixture();
    let node = fs.lookup(1, "file").await.unwrap();
    let gate = Arc::new(Semaphore::new(0));
    gate.close();
    *store.read_gate.lock().unwrap() = Some(gate);
    let (ready, received) = oneshot::channel();
    let load = tokio::spawn({
        let fs = fs.clone();
        async move {
            fs.read_reply(node.ino, 0, STEP, move |result| {
                let _ = ready.send(result);
            })
            .await
        }
    });
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(3), received)
            .await
            .unwrap()
            .unwrap(),
        Err(Error::Cancelled)
    ));
    assert!(finish(load).await.is_none());
    assert_eq!(fs.effective_budgets().data_used_bytes, 0);
}

#[tokio::test]
async fn failure_after_successful_reply_remains_an_observable_prefetch_error() {
    let (store, fs, expected) = fixture();
    let node = fs.lookup(1, "file").await.unwrap();
    let tail = gate(&store, 20 * MIB as u64);
    let (ready, received) = oneshot::channel();
    let load = tokio::spawn({
        let fs = fs.clone();
        async move {
            fs.read_reply(node.ino, 0, STEP, move |result| {
                let _ = ready.send(result);
            })
            .await
        }
    });
    let outcome = tokio::time::timeout(Duration::from_secs(3), received).await;
    if outcome.is_err() {
        load.abort();
    }
    let block = outcome
        .expect("valid requested bytes must not wait for optional prefetch")
        .unwrap()
        .unwrap();
    assert_eq!(block.as_slice(), &expected[..STEP]);
    tail.close();
    assert!(matches!(finish(load).await, Some(Error::Cancelled)));
    assert_eq!(block.as_slice(), &expected[..STEP]);
}

#[tokio::test]
async fn a_follower_already_waiting_for_the_window_observes_its_completed_range() {
    use std::{future::Future, task::Poll};
    let (store, fs, expected) = fixture();
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    let head = gate(&store, 0);
    let tail = gate(&store, 20 * MIB as u64);
    let (ready, received) = oneshot::channel();
    let load = tokio::spawn({
        let fs = fs.clone();
        async move {
            fs.read_reply(ino, 0, STEP, move |result| {
                let _ = ready.send(result);
            })
            .await
        }
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while store.read_calls.load(std::sync::atomic::Ordering::Relaxed) < 2 {
            store.read_started.notified().await;
        }
    })
    .await
    .unwrap();
    let mut follower = Box::pin(fs.read(ino, STEP as u64 + 7, 4096));
    std::future::poll_fn(|cx| {
        assert!(follower.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    head.add_permits(1);
    let following = tokio::time::timeout(Duration::from_secs(3), follower)
        .await
        .expect("a waiting follower must observe published bytes before the unrelated tail")
        .unwrap();
    let block = tokio::time::timeout(Duration::from_secs(3), received)
        .await
        .expect("the requested result must be published while its follower can read")
        .unwrap()
        .unwrap();
    assert_eq!(block.as_slice(), &expected[..STEP]);
    assert_eq!(following.as_slice(), &expected[STEP + 7..STEP + 7 + 4096]);
    assert!(!load.is_finished());
    tail.add_permits(1);
    assert!(finish(load).await.is_none());
    assert_eq!(store.read_ranges.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn an_optional_tail_failure_cannot_cancel_a_still_loading_requested_range() {
    let (store, fs, expected) = fixture();
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    let head = gate(&store, 0);
    let tail = gate(&store, 20 * MIB as u64);
    tail.close();
    let (ready, received) = oneshot::channel();
    let load = tokio::spawn({
        let fs = fs.clone();
        async move {
            fs.read_reply(ino, 0, STEP, move |result| {
                let _ = ready.send(result);
            })
            .await
        }
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while store.read_calls.load(std::sync::atomic::Ordering::Relaxed) < 2 {
            store.read_started.notified().await;
        }
    })
    .await
    .unwrap();
    head.add_permits(1);
    let block = tokio::time::timeout(Duration::from_secs(3), received)
        .await
        .unwrap()
        .unwrap()
        .expect("optional read-ahead failure must not replace correct requested bytes");
    assert_eq!(block.as_slice(), &expected[..STEP]);
    assert!(matches!(finish(load).await, Some(Error::Cancelled)));
}

#[tokio::test]
async fn cancelling_after_reply_releases_tail_owners_and_allows_a_following_fill() {
    let (store, fs, expected) = fixture();
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    let tail = gate(&store, 20 * MIB as u64);
    let (ready, received) = oneshot::channel();
    let load = tokio::spawn({
        let fs = fs.clone();
        async move {
            fs.read_reply(ino, 0, STEP, move |result| {
                let _ = ready.send(result);
            })
            .await
        }
    });
    let block = tokio::time::timeout(Duration::from_secs(3), received)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    load.abort();
    assert!(
        tokio::time::timeout(Duration::from_secs(3), load)
            .await
            .unwrap()
            .unwrap_err()
            .is_cancelled()
    );
    tokio::time::timeout(Duration::from_secs(3), async {
        while fs.effective_budgets().data_used_bytes != 20 * MIB + STEP {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("cancelled tail reservations must be released while the reply remains valid");
    assert_eq!(block.as_slice(), &expected[..STEP]);
    tail.add_permits(1);
    let next = tokio::time::timeout(
        Duration::from_secs(3),
        fs.read(ino, 20 * MIB as u64 + 7, 4096),
    )
    .await
    .expect("the cancelled fill must release its overlapping interval")
    .unwrap();
    assert_eq!(
        next.as_slice(),
        &expected[20 * MIB + 7..20 * MIB + 7 + 4096]
    );
    drop(next);
    drop(block);
    assert_eq!(fs.effective_budgets().data_used_bytes, 40 * MIB);
    assert_eq!(store.read_ranges.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn validation_and_empty_reads_publish_exactly_one_result_without_backend_reads() {
    let store = MemoryStore::new();
    store.seed("file", b"fixture");
    let fs = store.filesystem(32);
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    for (ino, offset, length, errno) in [
        (ino, 0, 0, None),
        (ino, 100, STEP, None),
        (ino, 0, 2 * MIB, Some(libc::EINVAL)),
        (u64::MAX, 0, STEP, Some(libc::ENOENT)),
    ] {
        let mut observed = None;
        assert!(
            fs.read_reply(ino, offset, length, |result| {
                assert!(observed.replace(result).is_none());
            })
            .await
            .is_none()
        );
        match (observed.expect("read must publish a result"), errno) {
            (Ok(block), None) => assert!(block.as_slice().is_empty()),
            (Err(Error::Fs(actual)), Some(expected)) => assert_eq!(actual, expected),
            _ => panic!("unexpected validation or EOF outcome"),
        }
        let mut borrowed = None;
        assert!(
            fs.read_reply_data(ino, offset, length, |result| {
                assert!(borrowed.replace(result.map(<[u8]>::to_vec)).is_none());
            })
            .await
            .is_none()
        );
        match (
            borrowed.expect("borrowed read must publish exactly once"),
            errno,
        ) {
            (Ok(bytes), None) => assert!(bytes.is_empty()),
            (Err(Error::Fs(actual)), Some(expected)) => assert_eq!(actual, expected),
            _ => panic!("unexpected borrowed validation or EOF outcome"),
        }
    }
    assert_eq!(
        store.read_calls.load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    assert_eq!(fs.effective_budgets().data_used_bytes, 0);
}

#[tokio::test]
async fn requested_bytes_across_parallel_chunks_are_complete_before_publication() {
    let expected: Vec<u8> = (0..256 * 1024).map(|at| (at % 251) as u8).collect();
    let store = MemoryStore::new();
    store.seed("file", &expected);
    let mut config = MemoryStore::config(32);
    config.read_ahead = 256;
    config.read_ahead_small = 256;
    config.read_ahead_large = 256;
    config.read_ahead_parallel = 64;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    let tail = Arc::new(Semaphore::new(0));
    for offset in [64 * 1024, 128 * 1024, 192 * 1024] {
        store
            .read_range_gates
            .lock()
            .unwrap()
            .insert((offset, 64 * 1024), tail.clone());
    }
    let (ready, received) = oneshot::channel();
    let load = tokio::spawn({
        let fs = fs.clone();
        async move {
            fs.read_reply(ino, 63 * 1024, 4096, move |result| {
                let _ = ready.send(result);
            })
            .await
        }
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while store.read_calls.load(std::sync::atomic::Ordering::Relaxed) < 4 {
            store.read_started.notified().await;
        }
    })
    .await
    .unwrap();
    tail.add_permits(3);
    let block = tokio::time::timeout(Duration::from_secs(3), received)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(block.as_slice(), &expected[63 * 1024..63 * 1024 + 4096]);
    assert!(finish(load).await.is_none());
}

#[tokio::test]
async fn a_write_and_fsync_during_tail_loading_cannot_publish_stale_bytes_to_a_new_read() {
    let (store, fs, expected) = fixture();
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    let tail = gate(&store, 20 * MIB as u64);
    let (ready, received) = oneshot::channel();
    let load = tokio::spawn({
        let fs = fs.clone();
        async move {
            fs.read_reply(ino, 0, STEP, move |result| {
                let _ = ready.send(result);
            })
            .await
        }
    });
    let old = tokio::time::timeout(Duration::from_secs(3), received)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    fs.write(ino, 3, b"new-generation").await.unwrap();
    fs.write(ino, 20 * MIB as u64 + 3, b"updated-tail")
        .await
        .unwrap();
    let dirty = fs.read(ino, 3, 14).await.unwrap();
    assert_eq!(dirty.as_slice(), b"new-generation");
    drop(dirty);
    tokio::time::timeout(Duration::from_secs(3), fs.fsync(ino))
        .await
        .unwrap()
        .unwrap();
    assert!(!load.is_finished());
    tail.add_permits(1);
    assert!(matches!(finish(load).await, Some(Error::Fs(libc::ESTALE))));
    assert_eq!(old.as_slice(), &expected[..STEP]);
    store.read_range_gates.lock().unwrap().clear();
    let new_tail = tokio::time::timeout(
        Duration::from_secs(3),
        fs.read(ino, 20 * MIB as u64 + 3, 12),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(new_tail.as_slice(), b"updated-tail");
    drop(new_tail);
    let new = tokio::time::timeout(Duration::from_secs(3), fs.read(ino, 0, STEP))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&new.as_slice()[..3], &expected[..3]);
    assert_eq!(&new.as_slice()[3..17], b"new-generation");
    assert_eq!(&new.as_slice()[17..], &expected[17..STEP]);
    assert_eq!(&store.bytes("file")[3..17], b"new-generation");
    assert!(fs.effective_budgets().data_used_bytes <= fs.effective_budgets().data_shared_bytes);
}
