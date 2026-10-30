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
