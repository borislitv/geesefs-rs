#![forbid(unsafe_code)]
mod support;

use geesefs_rs::filesystem::Filesystem;
use std::sync::Arc;
use support::MemoryStore;

const MIB: usize = 1024 * 1024;
const STEP: usize = 128 * 1024;

fn fixture() -> (Arc<MemoryStore>, Arc<Filesystem>, Vec<u8>) {
    let store = MemoryStore::new();
    let bytes: Vec<u8> = (0..8 * MIB).map(|at| (at / STEP) as u8).collect();
    store.seed("file", &bytes);
    let mut config = MemoryStore::config(16);
    config.read_ahead = MIB / 1024;
    config.read_ahead_small = MIB / 1024;
    config.read_ahead_large = 4 * MIB / 1024;
    config.read_ahead_parallel = 4 * MIB / 1024;
    config.large_read_cutoff = (MIB / 1024) as u64;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    (store, fs, bytes)
}

#[tokio::test]
async fn a_reopened_stream_does_not_inherit_the_previous_streams_large_window() {
    let (store, fs, bytes) = fixture();
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    fs.open(ino, false).unwrap();
    for offset in (0..3 * MIB).step_by(STEP) {
        let block = fs.read(ino, offset as u64, STEP).await.unwrap();
        assert_eq!(block.as_slice(), &bytes[offset..offset + STEP]);
    }
    assert_eq!(
        *store.read_ranges.lock().unwrap(),
        [(0, MIB), (MIB as u64, 4 * MIB)]
    );
    fs.release(ino);
    // Force a real miss independently of whether idle flush capacity retains
    // the original window. Reopen policy must not rely on a particular cache cap.
    store.seed("pressure", &vec![b'p'; 16 * MIB]);
    let pressure = fs.lookup(1, "pressure").await.unwrap().ino;
    for offset in (0..16 * MIB).step_by(MIB) {
        let block = fs.read(pressure, offset as u64, MIB).await.unwrap();
        assert!(block.as_slice().iter().all(|&byte| byte == b'p'));
    }
    let before = store.read_ranges.lock().unwrap().len();
    fs.open(ino, false).unwrap();
    let first = fs.read(ino, 0, STEP).await.unwrap();
    assert_eq!(first.as_slice(), &bytes[..STEP]);
    assert_eq!(
        store.read_ranges.lock().unwrap()[before..],
        [(0, MIB)],
        "a new stream must start at the configured initial window, not the previous open's promoted window"
    );
    fs.release(ino);
}

#[tokio::test]
async fn opening_a_second_handle_preserves_the_active_streams_history() {
    let (store, fs, bytes) = fixture();
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    fs.open(ino, false).unwrap();
    for offset in (0..3 * MIB).step_by(STEP) {
        let block = fs.read(ino, offset as u64, STEP).await.unwrap();
        assert_eq!(block.as_slice(), &bytes[offset..offset + STEP]);
    }
    fs.open(ino, false).unwrap();
    let next = fs.read(ino, (5 * MIB) as u64, STEP).await.unwrap();
    assert_eq!(next.as_slice(), &bytes[5 * MIB..5 * MIB + STEP]);
    assert_eq!(
        *store.read_ranges.lock().unwrap(),
        [(0, MIB), (MIB as u64, 4 * MIB), ((5 * MIB) as u64, 3 * MIB)],
        "another open must not reset the history of an already active stream"
    );
    fs.release(ino);
    fs.release(ino);
}

#[tokio::test]
async fn reopening_resets_history_without_discarding_cached_payload() {
    let (store, fs, bytes) = fixture();
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    fs.open(ino, false).unwrap();
    drop(fs.read(ino, 0, STEP).await.unwrap());
    fs.release(ino);
    fs.open(ino, false).unwrap();
    let first = fs.read(ino, 0, STEP).await.unwrap();
    assert_eq!(first.as_slice(), &bytes[..STEP]);
    assert_eq!(
        *store.read_ranges.lock().unwrap(),
        [(0, MIB)],
        "resetting stream history must retain reusable payload"
    );
    fs.release(ino);
}

#[tokio::test]
async fn interleaved_open_files_keep_independent_stream_histories_and_share_payload() {
    let (store, fs, bytes) = fixture();
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    let sequential = fs.open_file(ino, false).unwrap();
    let random = fs.open_file(ino, false).unwrap();
    for offset in (0..3 * MIB).step_by(STEP) {
        let block = sequential.read(offset as u64, STEP).await.unwrap();
        assert_eq!(block.as_slice(), &bytes[offset..offset + STEP]);
        let other = random.read((7 * MIB) as u64, STEP).await.unwrap();
        assert_eq!(other.as_slice(), &bytes[7 * MIB..7 * MIB + STEP]);
    }
    assert_eq!(
        *store.read_ranges.lock().unwrap(),
        [(0, MIB), ((7 * MIB) as u64, MIB), (MIB as u64, 4 * MIB)],
        "another handle's cached seeks must not break the sequential reader's history"
    );
    let before = store.read_ranges.lock().unwrap().len();
    let block = random.read((2 * MIB) as u64, STEP).await.unwrap();
    assert_eq!(block.as_slice(), &bytes[2 * MIB..2 * MIB + STEP]);
    assert_eq!(store.read_ranges.lock().unwrap().len(), before);
}

#[tokio::test]
async fn a_new_open_file_does_not_inherit_another_live_handles_promoted_window() {
    let (store, fs, bytes) = fixture();
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    let sequential = fs.open_file(ino, false).unwrap();
    for offset in (0..3 * MIB).step_by(STEP) {
        let block = sequential.read(offset as u64, STEP).await.unwrap();
        assert_eq!(block.as_slice(), &bytes[offset..offset + STEP]);
    }
    let before = store.read_ranges.lock().unwrap().len();
    let fresh = fs.open_file(ino, false).unwrap();
    let block = fresh.read((5 * MIB) as u64, STEP).await.unwrap();
    assert_eq!(block.as_slice(), &bytes[5 * MIB..5 * MIB + STEP]);
    assert_eq!(
        store.read_ranges.lock().unwrap()[before..],
        [((5 * MIB) as u64, MIB)],
        "a new handle must start at the initial configured window"
    );
}

#[tokio::test]
async fn the_last_open_owner_keeps_its_inode_live_and_then_permits_eviction() {
    let store = MemoryStore::new();
    store.seed("file", b"retained bytes");
    store.seed("other", b"other bytes");
    let mut config = MemoryStore::config(16);
    config.entry_limit = 2;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    let file = fs.open_file(ino, false).unwrap();
    let admitted = file.clone();
    drop(file);
    assert!(matches!(
        fs.lookup(1, "other").await,
        Err(geesefs_rs::error::Error::Fs(libc::ENOMEM))
    ));
    assert_eq!(
        admitted.read(0, 14).await.unwrap().as_slice(),
        b"retained bytes"
    );
    drop(admitted);
    assert_eq!(fs.lookup(1, "other").await.unwrap().attr().size, 11);
    assert!(fs.node(ino).is_err());
}

#[tokio::test]
async fn a_borrowed_fuse_reply_uses_its_own_open_history() {
    let (store, fs, bytes) = fixture();
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    let sequential = fs.open_file(ino, false).unwrap();
    for offset in (0..3 * MIB).step_by(STEP) {
        drop(sequential.read(offset as u64, STEP).await.unwrap());
    }
    let before = store.read_ranges.lock().unwrap().len();
    let mut replied = false;
    let error = sequential
        .read_reply_data((5 * MIB) as u64, STEP, |result| {
            assert_eq!(result.unwrap(), &bytes[5 * MIB..5 * MIB + STEP]);
            replied = true;
        })
        .await;
    assert!(error.is_none());
    assert!(replied);
    assert_eq!(
        store.read_ranges.lock().unwrap()[before..],
        [((5 * MIB) as u64, 3 * MIB)]
    );
}

#[tokio::test]
async fn cancelling_the_last_open_read_releases_its_inode_and_payload_admission() {
    let store = MemoryStore::new();
    store.seed("file", b"retained bytes");
    store.seed("other", b"other bytes");
    let mut config = MemoryStore::config(16);
    config.entry_limit = 2;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    let file = fs.open_file(ino, false).unwrap();
    *store.read_gate.lock().unwrap() = Some(Arc::new(tokio::sync::Semaphore::new(0)));
    let read = tokio::spawn(async move {
        file.read_reply_data(0, 14, |_| panic!("cancelled before the body was available"))
            .await
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        store.read_started.notified(),
    )
    .await
    .unwrap();
    assert!(matches!(
        fs.lookup(1, "other").await,
        Err(geesefs_rs::error::Error::Fs(libc::ENOMEM))
    ));
    read.abort();
    assert!(read.await.unwrap_err().is_cancelled());
    assert_eq!(fs.effective_budgets().data_used_bytes, 0);
    assert_eq!(fs.lookup(1, "other").await.unwrap().attr().size, 11);
    assert!(fs.node(ino).is_err());
}
