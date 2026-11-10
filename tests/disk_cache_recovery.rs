#![forbid(unsafe_code)]
#[path = "support/disk.rs"]
mod support;
use budget::Budget;
use disk_cache::{Demotion, DiskCache, DiskKey, PreparedDiskCache, State};
use std::{
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::MetadataExt,
};
use support::{DEADLINE, MIB, block, key, open};
pub use support::{budget, disk_cache, error};

#[tokio::test]
async fn completed_partial_and_short_eof_survive_reopen_without_repreallocation() {
    let dir = support::directory();
    let resources = Budget::new(32 * MIB).unwrap();
    let output = Budget::new(MIB).unwrap();
    let eof = DiskKey::new(b"namespace", b"tail.rpm", b"etag", 131_079, 131_072, 7).unwrap();
    let cache = open(dir.path(), &resources).await;
    assert_eq!(
        cache.try_demote(eof.clone(), block(b"EOFtail")),
        Demotion::Admitted
    );
    cache.drain(DEADLINE).await.unwrap();
    let allocated = std::fs::metadata(dir.path().join("payload-v1.cache"))
        .unwrap()
        .blocks();
    assert!(allocated * 512 >= (16 * MIB) as u64);
    cache.close(DEADLINE).await.unwrap();
    drop(cache);
    let cache = open(dir.path(), &resources).await;
    assert_eq!(
        std::fs::metadata(dir.path().join("payload-v1.cache"))
            .unwrap()
            .blocks(),
        allocated
    );
    assert!(
        cache.stats().io_read_bytes < MIB as u64,
        "startup scans indexes/log, not the payload container"
    );
    assert_eq!(
        cache.read_block(&eof, &output).await.unwrap().as_slice(),
        b"EOFtail"
    );
    assert!(
        cache
            .read_block(
                &DiskKey::new(b"namespace", b"tail.rpm", b"etag", 131_080, 131_072, 8).unwrap(),
                &output
            )
            .await
            .is_none()
    );
    cache.close(DEADLINE).await.unwrap();
}

#[tokio::test]
async fn process_interruption_preserves_completed_unrelated_inventory() {
    for mode in ["graceful", "abrupt"] {
        let dir = support::directory();
        support::child(mode, dir.path()).await;
        let resources = Budget::new(32 * MIB).unwrap();
        let output = Budget::new(MIB).unwrap();
        let cache = open(dir.path(), &resources).await;
        assert_eq!(
            cache
                .read_block(&key(0, 9), &output)
                .await
                .unwrap()
                .as_slice(),
            b"completed",
            "{mode}"
        );
        let pending = cache.read_block(&key(128, 7), &output).await;
        if mode == "graceful" {
            assert_eq!(pending.unwrap().as_slice(), b"pending");
        } else {
            assert!(pending.is_none());
        }
        cache.close(DEADLINE).await.unwrap();
    }
}

#[tokio::test]
async fn second_process_cannot_open_locked_directory_even_before_engine_open() {
    let dir = support::directory();
    let prepared = PreparedDiskCache::prepare(dir.path(), (16 * MIB) as u64).unwrap();
    assert_eq!(prepared.status().state(), State::Prepared);
    support::child("locked", dir.path()).await;
    assert!(
        !dir.path().join("payload-v1.cache").exists(),
        "locking must precede preallocation"
    );
    drop(prepared);
    support::child("unlocked", dir.path()).await;
}

#[tokio::test]
async fn cancelled_recovery_keeps_funding_and_directory_until_owned_startup_closes() {
    use std::sync::atomic::Ordering;
    let dir = support::directory();
    let resources = Budget::new(32 * MIB).unwrap();
    let cache = open(dir.path(), &resources).await;
    assert_eq!(
        cache.try_demote(key(0, 3), block(b"old")),
        Demotion::Admitted
    );
    cache.drain(DEADLINE).await.unwrap();
    cache.close(DEADLINE).await.unwrap();
    drop(cache);
    let prepared = PreparedDiskCache::prepare(dir.path(), (16 * MIB) as u64).unwrap();
    let status = prepared.status();
    let gate = support::IoGate::new();
    gate.hold_reads.store(true, Ordering::SeqCst);
    let opening = {
        let resources = resources.clone();
        let gate = gate.clone();
        tokio::spawn(async move {
            DiskCache::open_test(
                prepared,
                &resources,
                Some(Box::new(support::GateConfig(gate))),
            )
            .await
        })
    };
    tokio::time::timeout(DEADLINE, gate.entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    assert_eq!(status.state(), State::Recovering);
    opening.abort();
    assert!(matches!(opening.await, Err(error) if error.is_cancelled()));
    assert!(resources.used() > 0);
    assert!(PreparedDiskCache::prepare(dir.path(), (16 * MIB) as u64).is_err());
    gate.hold_reads.store(false, Ordering::SeqCst);
    gate.release.add_permits(1);
    tokio::time::timeout(DEADLINE, async {
        while status.state() != State::Closed || resources.used() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let cache = open(dir.path(), &resources).await;
    assert_eq!(
        cache
            .read_block(&key(0, 3), &Budget::new(3).unwrap())
            .await
            .unwrap()
            .as_slice(),
        b"old"
    );
    cache.close(DEADLINE).await.unwrap();
}

#[tokio::test]
async fn disk_cache_child() {
    let Ok(mode) = std::env::var("GEESEFS_L2_CHILD_MODE") else {
        return;
    };
    let path = std::path::PathBuf::from(std::env::var_os("GEESEFS_L2_CHILD_DIR").unwrap());
    if mode == "locked" {
        assert!(PreparedDiskCache::prepare(&path, (16 * MIB) as u64).is_err());
        return;
    }
    if mode == "unlocked" {
        assert!(PreparedDiskCache::prepare(&path, (16 * MIB) as u64).is_ok());
        return;
    }
    let resources = Budget::new(32 * MIB).unwrap();
    let gate = support::IoGate::new();
    let cache = support::gated(&path, &resources, gate.clone()).await;
    assert_eq!(
        cache.try_demote(key(0, 9), block(b"completed")),
        Demotion::Admitted
    );
    cache.drain(DEADLINE).await.unwrap();
    assert_eq!(
        cache
            .read_block(&key(0, 9), &Budget::new(9).unwrap())
            .await
            .unwrap()
            .as_slice(),
        b"completed"
    );
    gate.publication_only
        .store(true, std::sync::atomic::Ordering::SeqCst);
    gate.hold_writes
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        cache.try_demote(key(128, 7), block(b"pending")),
        Demotion::Admitted
    );
    tokio::time::timeout(DEADLINE, gate.entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    assert_eq!(
        gate.payload_written.available_permits(),
        1,
        "real payload write completed, but its publication index is still gated"
    );
    tokio::time::timeout(DEADLINE, gate.log_written.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    assert!(
        cache
            .read_block(&key(128, 7), &Budget::new(7).unwrap())
            .await
            .is_none()
    );
    if mode == "abrupt" {
        std::process::exit(0);
    }
    assert_eq!(mode, "graceful");
    gate.hold_writes
        .store(false, std::sync::atomic::Ordering::SeqCst);
    gate.release.add_permits(16);
    cache.close(DEADLINE).await.unwrap();
}

#[tokio::test]
async fn corrupt_record_is_a_miss_not_zero_fill_or_destructive_recovery() {
    let dir = support::directory();
    let resources = Budget::new(32 * MIB).unwrap();
    let payload = b"independent integrity test payload";
    let cache = open(dir.path(), &resources).await;
    assert_eq!(
        cache.try_demote(key(0, payload.len()), block(payload)),
        Demotion::Admitted
    );
    cache.drain(DEADLINE).await.unwrap();
    cache.close(DEADLINE).await.unwrap();
    drop(cache);
    let path = dir.path().join("payload-v1.cache");
    let bytes = std::fs::read(&path).unwrap();
    let position = bytes
        .windows(payload.len())
        .position(|window| window == payload)
        .unwrap();
    let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.seek(SeekFrom::Start(position as u64)).unwrap();
    file.write_all(b"X").unwrap();
    drop(file);
    let before = std::fs::read(&path).unwrap();
    let cache = open(dir.path(), &resources).await;
    assert!(
        cache
            .read_block(&key(0, payload.len()), &Budget::new(MIB).unwrap())
            .await
            .is_none()
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "a corrupt cache copy cannot authorize media reset"
    );
    // Read integration later performs the actual S3 fallback; this unit never
    // fabricates a cache hit or substitutes zeroes for the authoritative bytes.
    cache.close(DEADLINE).await.unwrap();
}

#[tokio::test]
async fn capacity_and_payload_budget_shrink_never_truncate_or_wipe_media() {
    let dir = support::directory();
    let resources = Budget::new(32 * MIB).unwrap();
    let cache = open(dir.path(), &resources).await;
    assert_eq!(
        cache.try_demote(key(0, 3), block(b"old")),
        Demotion::Admitted
    );
    cache.drain(DEADLINE).await.unwrap();
    cache.close(DEADLINE).await.unwrap();
    drop(cache);
    let path = dir.path().join("payload-v1.cache");
    let before = std::fs::read(&path).unwrap();
    assert!(PreparedDiskCache::prepare(dir.path(), (8 * MIB) as u64).is_err());
    let prepared = PreparedDiskCache::prepare(dir.path(), (16 * MIB) as u64).unwrap();
    assert!(
        DiskCache::open(prepared, &Budget::new(MIB).unwrap())
            .await
            .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let cache = open(dir.path(), &resources).await;
    assert_eq!(
        cache
            .read_block(&key(0, 3), &Budget::new(3).unwrap())
            .await
            .unwrap()
            .as_slice(),
        b"old"
    );
    cache.close(DEADLINE).await.unwrap();
}

#[tokio::test]
async fn unsafe_log_history_and_truncated_media_return_errors_without_reset() {
    let dir = support::directory();
    let resources = Budget::new(32 * MIB).unwrap();
    let cache = open(dir.path(), &resources).await;
    assert_eq!(
        cache.try_demote(key(0, 3), block(b"old")),
        Demotion::Admitted
    );
    cache.drain(DEADLINE).await.unwrap();
    cache.close(DEADLINE).await.unwrap();
    drop(cache);
    let path = dir.path().join("payload-v1.cache");
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    let mut original = [0; 8];
    file.read_exact(&mut original).unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(b"unsafe!!").unwrap();
    let before = std::fs::read(&path).unwrap();
    let prepared = PreparedDiskCache::prepare(dir.path(), (16 * MIB) as u64).unwrap();
    let status = prepared.status();
    assert!(DiskCache::open(prepared, &resources).await.is_err());
    assert_eq!(status.state(), State::Bypass);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(&original).unwrap();
    file.set_len((8 * MIB) as u64).unwrap();
    let truncated = std::fs::read(&path).unwrap();
    assert!(PreparedDiskCache::prepare(dir.path(), (16 * MIB) as u64).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), truncated);
}
