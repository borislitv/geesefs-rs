#![forbid(unsafe_code)]
mod support;

use geesefs_rs::{error::Error, filesystem::Filesystem};
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use support::MemoryStore;
use tokio::sync::{Semaphore, oneshot};

const MIB: usize = 1024 * 1024;
const STEP: usize = 128 * 1024;

fn fixture(memory: usize, window: usize) -> (Arc<MemoryStore>, Arc<Filesystem>, Vec<u8>) {
    let bytes: Vec<u8> = (0..4 * window)
        .map(|at| ((at / STEP) % 251) as u8)
        .collect();
    let store = MemoryStore::new();
    store.seed("file", &bytes);
    let mut config = MemoryStore::config(memory);
    config.read_ahead = window / 1024;
    config.read_ahead_small = window / 1024;
    config.read_ahead_large = window / 1024;
    config.read_ahead_parallel = window / 1024;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    (store, fs, bytes)
}

async fn wait_for_refill(store: &MemoryStore, load: &mut tokio::task::JoinHandle<Option<Error>>) {
    let started = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if store.read_calls.load(Ordering::Relaxed) >= 2 {
                return true;
            }
            tokio::select! {
                _ = store.read_started.notified() => {},
                _ = &mut *load => return false,
            }
        }
    })
    .await;
    if started != Ok(true) {
        load.abort();
    }
    assert_eq!(
        started,
        Ok(true),
        "a cache-hit reply must start the next window before its first demand read"
    );
}

#[tokio::test]
async fn cache_hit_starts_next_window_before_boundary_without_delaying_reply() {
    let (store, fs, bytes) = fixture(32, MIB);
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    drop(fs.read(ino, 0, STEP).await.unwrap());
    let gate = Arc::new(Semaphore::new(0));
    store
        .read_range_gates
        .lock()
        .unwrap()
        .insert((MIB as u64, MIB), gate.clone());
    let (sent, received) = oneshot::channel();
    let mut load = tokio::spawn({
        let fs = fs.clone();
        async move {
            fs.read_reply_data(ino, (MIB / 2) as u64, STEP, move |result| {
                let _ = sent.send(result.map(<[u8]>::to_vec));
            })
            .await
        }
    });
    let reply = tokio::time::timeout(Duration::from_secs(3), received)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(reply, bytes[MIB / 2..MIB / 2 + STEP]);
    wait_for_refill(&store, &mut load).await;
    assert!(
        !load.is_finished(),
        "the blocked refill must remain caller-owned after reply"
    );
    assert_eq!(
        *store.read_ranges.lock().unwrap(),
        [(0, MIB), (MIB as u64, MIB)]
    );
    gate.add_permits(1);
    assert!(
        tokio::time::timeout(Duration::from_secs(3), load)
            .await
            .unwrap()
            .unwrap()
            .is_none()
    );
    let boundary = fs.read(ino, MIB as u64, STEP).await.unwrap();
    assert_eq!(boundary.as_slice(), &bytes[MIB..MIB + STEP]);
    assert_eq!(
        store.read_calls.load(Ordering::Relaxed),
        2,
        "the boundary read must reuse the completed refill"
    );
    assert_eq!(fs.effective_budgets().data_used_bytes, 2 * MIB);
}

async fn cached_load(
    fs: Arc<Filesystem>,
    ino: u64,
    offset: usize,
) -> (tokio::task::JoinHandle<Option<Error>>, Vec<u8>) {
    let (sent, received) = oneshot::channel();
    let load = tokio::spawn(async move {
        fs.read_reply_data(ino, offset as u64, STEP, move |result| {
            let _ = sent.send(result.map(<[u8]>::to_vec));
        })
        .await
    });
    let outcome = tokio::time::timeout(Duration::from_secs(3), received).await;
    if outcome.is_err() {
        load.abort();
    }
    (
        load,
        outcome
            .expect("cached requested bytes must not wait for optional refill")
            .unwrap()
            .unwrap(),
    )
}

fn next_gate(store: &MemoryStore) -> Arc<Semaphore> {
    let gate = Arc::new(Semaphore::new(0));
    store
        .read_range_gates
        .lock()
        .unwrap()
        .insert((MIB as u64, MIB), gate.clone());
    gate
}

#[tokio::test]
async fn concurrent_cache_hits_do_not_duplicate_an_inflight_refill() {
    let (store, fs, bytes) = fixture(32, MIB);
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    drop(fs.read(ino, 0, STEP).await.unwrap());
    let gate = next_gate(&store);
    let (mut load, reply) = cached_load(fs.clone(), ino, MIB / 2).await;
    assert_eq!(reply, bytes[MIB / 2..MIB / 2 + STEP]);
    wait_for_refill(&store, &mut load).await;
    let (hit, reply) = cached_load(fs.clone(), ino, MIB / 2 + STEP).await;
    assert_eq!(reply, bytes[MIB / 2 + STEP..MIB / 2 + 2 * STEP]);
    assert!(
        tokio::time::timeout(Duration::from_secs(3), hit)
            .await
            .unwrap()
            .unwrap()
            .is_none()
    );
    assert_eq!(store.read_calls.load(Ordering::Relaxed), 2);
    assert!(!load.is_finished());
    gate.add_permits(1);
    assert!(
        tokio::time::timeout(Duration::from_secs(3), load)
            .await
            .unwrap()
            .unwrap()
            .is_none()
    );
    let boundary = fs.read(ino, MIB as u64, STEP).await.unwrap();
    assert_eq!(boundary.as_slice(), &bytes[MIB..MIB + STEP]);
    assert_eq!(store.read_calls.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn cancelling_refill_wakes_a_required_reader_and_releases_its_reservation() {
    use std::{future::Future, task::Poll};
    let (store, fs, bytes) = fixture(32, MIB);
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    drop(fs.read(ino, 0, STEP).await.unwrap());
    let gate = next_gate(&store);
    let (mut load, reply) = cached_load(fs.clone(), ino, MIB / 2).await;
    wait_for_refill(&store, &mut load).await;
    let mut boundary = Box::pin(fs.read(ino, MIB as u64, STEP));
    std::future::poll_fn(|cx| {
        assert!(boundary.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    load.abort();
    assert!(
        tokio::time::timeout(Duration::from_secs(3), load)
            .await
            .unwrap()
            .unwrap_err()
            .is_cancelled()
    );
    gate.add_permits(1);
    let block = tokio::time::timeout(Duration::from_secs(3), boundary)
        .await
        .expect("a cancelled optional fill must release its interval and wake a required reader")
        .unwrap();
    assert_eq!(block.as_slice(), &bytes[MIB..MIB + STEP]);
    assert_eq!(reply, bytes[MIB / 2..MIB / 2 + STEP]);
    assert_eq!(store.read_calls.load(Ordering::Relaxed), 3);
    drop(block);
    assert_eq!(
        fs.effective_budgets().data_used_bytes,
        2 * MIB,
        "cancelled refill admission must not remain charged"
    );
}

#[tokio::test]
async fn optional_refill_never_evicts_the_unconsumed_current_backing_to_admit_itself() {
    let (store, fs, bytes) = fixture(16, 4 * MIB);
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    drop(fs.read(ino, 0, STEP).await.unwrap());
    assert_eq!(fs.effective_budgets().data_shared_bytes, 5 * MIB);
    let (load, reply) = cached_load(fs.clone(), ino, 2 * MIB).await;
    assert_eq!(reply, bytes[2 * MIB..2 * MIB + STEP]);
    assert!(
        tokio::time::timeout(Duration::from_secs(3), load)
            .await
            .unwrap()
            .unwrap()
            .is_none()
    );
    let next = fs.read(ino, (2 * MIB + STEP) as u64, STEP).await.unwrap();
    assert_eq!(next.as_slice(), &bytes[2 * MIB + STEP..2 * MIB + 2 * STEP]);
    assert_eq!(
        store.read_calls.load(Ordering::Relaxed),
        1,
        "speculation must skip insufficient admission instead of discarding future required bytes"
    );
    assert_eq!(fs.effective_budgets().data_used_bytes, 4 * MIB);
}

#[tokio::test]
async fn optional_refill_preserves_another_readers_unconsumed_backing() {
    let (store, fs, bytes) = fixture(16, 2 * MIB);
    store.seed("other", &bytes);
    let other = fs.lookup(1, "other").await.unwrap().ino;
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    drop(fs.read(other, 0, STEP).await.unwrap());
    drop(fs.read(ino, 0, STEP).await.unwrap());
    assert_eq!(fs.effective_budgets().data_shared_bytes, 5 * MIB);
    let (load, reply) = cached_load(fs.clone(), ino, MIB).await;
    assert_eq!(reply, bytes[MIB..MIB + STEP]);
    assert!(
        tokio::time::timeout(Duration::from_secs(3), load)
            .await
            .unwrap()
            .unwrap()
            .is_none()
    );
    let before = store.read_calls.load(Ordering::Relaxed);
    let next = fs.read(other, STEP as u64, STEP).await.unwrap();
    assert_eq!(next.as_slice(), &bytes[STEP..2 * STEP]);
    assert_eq!(
        store.read_calls.load(Ordering::Relaxed),
        before,
        "optional refill must not force another reader to fetch its unconsumed backing again"
    );
}

#[tokio::test]
async fn optional_refill_can_reclaim_a_fully_consumed_foreign_backing() {
    let (store, fs, bytes) = fixture(16, 2 * MIB);
    store.seed("other", &bytes);
    let other = fs.lookup(1, "other").await.unwrap().ino;
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    for offset in (0..2 * MIB).step_by(STEP) {
        let block = fs.read(other, offset as u64, STEP).await.unwrap();
        assert_eq!(block.as_slice(), &bytes[offset..offset + STEP]);
    }
    drop(fs.read(ino, 0, STEP).await.unwrap());
    let (load, reply) = cached_load(fs.clone(), ino, MIB).await;
    assert_eq!(reply, bytes[MIB..MIB + STEP]);
    assert!(
        tokio::time::timeout(Duration::from_secs(3), load)
            .await
            .unwrap()
            .unwrap()
            .is_none()
    );
    assert_eq!(store.read_calls.load(Ordering::Relaxed), 3);
    let boundary = fs.read(ino, (2 * MIB) as u64, STEP).await.unwrap();
    assert_eq!(boundary.as_slice(), &bytes[2 * MIB..2 * MIB + STEP]);
    assert_eq!(store.read_calls.load(Ordering::Relaxed), 3);
}

#[tokio::test]
async fn reading_a_foreign_backings_tail_does_not_mark_its_gap_consumed() {
    let (store, fs, bytes) = fixture(16, 2 * MIB);
    store.seed("other", &bytes);
    let other = fs.lookup(1, "other").await.unwrap().ino;
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    drop(fs.read(other, 0, STEP).await.unwrap());
    let tail = fs.read(other, (2 * MIB - STEP) as u64, STEP).await.unwrap();
    assert_eq!(tail.as_slice(), &bytes[2 * MIB - STEP..2 * MIB]);
    drop(tail);
    drop(fs.read(ino, 0, STEP).await.unwrap());
    let (load, reply) = cached_load(fs.clone(), ino, MIB).await;
    assert_eq!(reply, bytes[MIB..MIB + STEP]);
    assert!(
        tokio::time::timeout(Duration::from_secs(3), load)
            .await
            .unwrap()
            .unwrap()
            .is_none()
    );
    let next = fs.read(other, STEP as u64, STEP).await.unwrap();
    assert_eq!(next.as_slice(), &bytes[STEP..2 * STEP]);
    assert_eq!(store.read_calls.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn refill_stops_at_an_already_cached_future_range() {
    let (store, fs, bytes) = fixture(32, 2 * MIB);
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    drop(fs.read(ino, 0, STEP).await.unwrap());
    drop(fs.read(ino, (3 * MIB) as u64, STEP).await.unwrap());
    let (load, reply) = cached_load(fs.clone(), ino, MIB).await;
    assert_eq!(reply, bytes[MIB..MIB + STEP]);
    assert!(
        tokio::time::timeout(Duration::from_secs(3), load)
            .await
            .unwrap()
            .unwrap()
            .is_none()
    );
    assert_eq!(
        *store.read_ranges.lock().unwrap(),
        [
            (0, 2 * MIB),
            ((3 * MIB) as u64, 2 * MIB),
            ((2 * MIB) as u64, MIB)
        ]
    );
    for offset in [2 * MIB, 3 * MIB] {
        let block = fs.read(ino, offset as u64, STEP).await.unwrap();
        assert_eq!(block.as_slice(), &bytes[offset..offset + STEP]);
    }
    assert_eq!(
        store.read_calls.load(Ordering::Relaxed),
        3,
        "refill must load only the missing gap"
    );
}

#[tokio::test]
async fn a_write_during_refill_cannot_be_replaced_by_old_generation_bytes() {
    let (store, fs, bytes) = fixture(32, MIB);
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    drop(fs.read(ino, 0, STEP).await.unwrap());
    let gate = next_gate(&store);
    let (mut load, reply) = cached_load(fs.clone(), ino, MIB / 2).await;
    wait_for_refill(&store, &mut load).await;
    let replacement = vec![b'w'; STEP];
    fs.write(ino, MIB as u64, &replacement).await.unwrap();
    gate.add_permits(1);
    assert!(
        tokio::time::timeout(Duration::from_secs(3), load)
            .await
            .unwrap()
            .unwrap()
            .is_none()
    );
    let block = fs.read(ino, MIB as u64, STEP).await.unwrap();
    assert_eq!(
        block.as_slice(),
        replacement,
        "speculative old-generation bytes must not hide a concurrent write"
    );
    assert_eq!(reply, bytes[MIB / 2..MIB / 2 + STEP]);
    assert!(fs.effective_budgets().data_used_bytes <= fs.effective_budgets().data_shared_bytes);
}

#[tokio::test]
async fn callback_truncate_prevents_speculation_from_a_dirty_snapshot() {
    let (store, fs, bytes) = fixture(32, MIB);
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    drop(fs.read(ino, 0, STEP).await.unwrap());
    let mut reply = None;
    assert!(
        fs.read_reply_data(ino, (MIB / 2) as u64, STEP, |result| {
            reply = Some(result.unwrap().to_vec());
            fs.truncate(ino, (3 * MIB) as u64).unwrap();
        })
        .await
        .is_none()
    );
    assert_eq!(reply.unwrap(), bytes[MIB / 2..MIB / 2 + STEP]);
    assert_eq!(store.read_calls.load(Ordering::Relaxed), 1);
    assert!(
        fs.read(ino, (3 * MIB) as u64, STEP)
            .await
            .unwrap()
            .as_slice()
            .is_empty()
    );
}

#[tokio::test]
async fn failed_refill_preserves_the_owned_reply_and_can_be_retried() {
    let (store, fs, bytes) = fixture(32, MIB);
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    drop(fs.read(ino, 0, STEP).await.unwrap());
    next_gate(&store).close();
    let mut reply = None;
    let error = fs
        .read_reply(ino, (MIB / 2) as u64, STEP, |result| {
            reply = Some(result.unwrap())
        })
        .await;
    assert!(
        matches!(error, Some(Error::Cancelled)),
        "optional refill failure must remain observable after a successful hit"
    );
    assert_eq!(
        reply.as_ref().unwrap().as_slice(),
        &bytes[MIB / 2..MIB / 2 + STEP]
    );
    store.read_range_gates.lock().unwrap().clear();
    let next = tokio::time::timeout(Duration::from_secs(3), fs.read(ino, MIB as u64, STEP))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(next.as_slice(), &bytes[MIB..MIB + STEP]);
    assert_eq!(store.read_calls.load(Ordering::Relaxed), 3);
}

#[tokio::test]
async fn adjacent_cached_fragments_form_one_window_and_refill_respects_eof() {
    let store = MemoryStore::new();
    let bytes: Vec<u8> = (0..MIB + STEP).map(|at| (at / STEP) as u8).collect();
    store.seed("file", &bytes);
    let mut config = MemoryStore::config(32);
    config.read_ahead = MIB / 1024;
    config.read_ahead_small = MIB / 1024;
    config.read_ahead_large = MIB / 1024;
    config.read_ahead_parallel = STEP / 1024;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    drop(fs.read(ino, 0, STEP).await.unwrap());
    let (load, reply) = cached_load(fs.clone(), ino, MIB / 2).await;
    assert_eq!(reply, bytes[MIB / 2..MIB / 2 + STEP]);
    assert!(
        tokio::time::timeout(Duration::from_secs(3), load)
            .await
            .unwrap()
            .unwrap()
            .is_none()
    );
    let mut ranges = store.read_ranges.lock().unwrap().clone();
    ranges.sort_unstable();
    assert_eq!(
        ranges,
        (0..9)
            .map(|part| ((part * STEP) as u64, STEP))
            .collect::<Vec<_>>(),
        "adjacent cached fragments must not be refetched or hide the missing EOF tail"
    );
    let (load, last) = cached_load(fs.clone(), ino, MIB).await;
    assert_eq!(last, bytes[MIB..]);
    assert!(
        tokio::time::timeout(Duration::from_secs(3), load)
            .await
            .unwrap()
            .unwrap()
            .is_none()
    );
    assert_eq!(store.read_calls.load(Ordering::Relaxed), 9);
    assert_eq!(fs.effective_budgets().data_used_bytes, MIB + STEP);
}
