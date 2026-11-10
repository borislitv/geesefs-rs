#![forbid(unsafe_code)]
#[path = "support/disk.rs"]
mod support;
use budget::Budget;
use disk_cache::{
    Demotion, DiskKey, Funding, MAX_IDENTITY_BYTES, MAX_PAYLOAD_BYTES, PreparedDiskCache, State,
};
use support::{DEADLINE, MIB, block, key, open};
pub use support::{budget, disk_cache, error};

#[tokio::test]
async fn exact_partial_payload_roundtrip_and_full_identity_misses() {
    let dir = support::directory();
    let resources = Budget::new(32 * MIB).unwrap();
    let output = Budget::new(MIB).unwrap();
    let cache = open(dir.path(), &resources).await;
    assert_eq!(cache.status().state(), State::Ready);
    assert_eq!(
        cache.try_demote(key(37, 19), block(b"independent payload")),
        Demotion::Admitted
    );
    cache.drain(DEADLINE).await.unwrap();
    assert_eq!(
        cache
            .read_block(&key(37, 19), &output)
            .await
            .unwrap()
            .as_slice(),
        b"independent payload"
    );
    let writes = cache.stats().io_write_bytes;
    assert_eq!(
        cache.try_demote(key(37, 19), block(b"independent payload")),
        Demotion::IndexedHint
    );
    assert_eq!(cache.stats().copied_bytes, 0);
    assert_eq!(cache.stats().active_jobs, 0);
    assert_eq!(
        cache.stats().io_write_bytes,
        writes,
        "read/promotion/demotion must not rewrite a known disk identity"
    );
    for wrong in [
        DiskKey::new(
            b"public-test-namespacE",
            b"packages/example.rpm",
            b"opaque-ETag",
            16 * MIB as u64,
            37,
            19,
        )
        .unwrap(),
        DiskKey::new(
            b"public-test-namespace",
            b"packages/example.RPM",
            b"opaque-ETag",
            16 * MIB as u64,
            37,
            19,
        )
        .unwrap(),
        DiskKey::new(
            b"public-test-namespace",
            b"packages/example.rpm",
            b"opaque-ETAg",
            16 * MIB as u64,
            37,
            19,
        )
        .unwrap(),
        DiskKey::new(
            b"public-test-namespace",
            b"packages/example.rpm",
            b"opaque-ETag",
            16 * MIB as u64 + 1,
            37,
            19,
        )
        .unwrap(),
        key(38, 19),
        key(37, 18),
    ] {
        assert!(cache.read_block(&wrong, &output).await.is_none());
    }
    cache.close(DEADLINE).await.unwrap();
    drop(cache);
    assert_eq!(resources.used(), 0);
    assert_eq!(output.used(), 0);
}

#[test]
fn bounded_identity_range_and_scaled_funding() {
    assert!(DiskKey::new(b"n", b"k", b"e", 1, 0, 2).is_err());
    assert!(DiskKey::new(b"n", b"k", b"e", u64::MAX, u64::MAX, 1).is_err());
    assert!(DiskKey::new(b"n", b"k", b"e", 100, 0, 0).is_err());
    assert!(DiskKey::new(&vec![1; MAX_IDENTITY_BYTES], b"k", b"e", 100, 0, 1).is_err());
    assert!(DiskKey::new(b"n", b"k", b"e", u64::MAX, 0, MAX_PAYLOAD_BYTES + 1).is_err());
    let small = Funding::derive((16 * MIB) as u64, 32 * MIB).unwrap();
    println!("small_device_funding={small:?}");
    assert!(
        small.required_bytes < 8 * MIB,
        "small cache must not inherit a GiB reserve"
    );
    assert_eq!(small.index_entries, 128);
    assert_eq!(
        small.fixed_bytes,
        2 * 512 * 1024 + small.recovery_bytes + MIB
    );
    assert!(Funding::derive((16 * MIB) as u64, MIB).is_err());
}

#[tokio::test]
async fn output_budget_and_wrong_coverage_fail_without_allocation_or_data() {
    let dir = support::directory();
    let resources = Budget::new(32 * MIB).unwrap();
    let cache = open(dir.path(), &resources).await;
    assert_eq!(cache.try_demote(key(0, 3), block(b"xx")), Demotion::Dropped);
    assert_eq!(
        cache.try_demote(key(0, 3), block(b"abc")),
        Demotion::Admitted
    );
    cache.drain(DEADLINE).await.unwrap();
    let output = Budget::new(2).unwrap();
    let before_read = cache.stats().io_read_bytes;
    assert!(cache.read_block(&key(0, 3), &output).await.is_none());
    assert_eq!(
        cache.stats().io_read_bytes,
        before_read,
        "output admission precedes disk I/O"
    );
    assert_eq!(output.used(), 0);
    assert_eq!(cache.stats().active_jobs, 0);
    assert_eq!(cache.stats().copied_bytes, 0);
    let admitted = Budget::new(3).unwrap();
    let reservation = admitted.try_reserve(3).unwrap();
    let result = cache
        .read_block_reserved(&key(0, 3), reservation, DEADLINE)
        .await
        .unwrap();
    assert_eq!(result.as_slice(), b"abc");
    assert_eq!(
        admitted.used(),
        3,
        "existing admission, not a second reservation, owns output"
    );
    drop(result);
    assert_eq!(admitted.used(), 0);
    cache.close(DEADLINE).await.unwrap();
}

#[test]
fn private_directory_and_capacity_shrink_are_nondestructive_errors() {
    let dir = support::directory();
    let lease = PreparedDiskCache::prepare(dir.path(), (16 * MIB) as u64).unwrap();
    assert!(PreparedDiskCache::prepare(dir.path(), (16 * MIB) as u64).is_err());
    drop(lease);
    std::fs::write(dir.path().join("foreign-media"), b"protected").unwrap();
    assert!(PreparedDiskCache::prepare(dir.path(), (16 * MIB) as u64).is_err());
    assert_eq!(
        std::fs::read(dir.path().join("foreign-media")).unwrap(),
        b"protected"
    );
}

#[test]
fn unsafe_permissions_and_symlink_directories_are_rejected_before_open() {
    use std::os::unix::fs::PermissionsExt;
    let dir = support::directory();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(PreparedDiskCache::prepare(dir.path(), (16 * MIB) as u64).is_err());
    assert!(!dir.path().join(".lock").exists());
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let parent = support::directory();
    let link = parent.path().join("cache");
    std::os::unix::fs::symlink(dir.path(), &link).unwrap();
    assert!(PreparedDiskCache::prepare(&link, (16 * MIB) as u64).is_err());
    assert!(!dir.path().join("payload-v1.cache").exists());
}

#[tokio::test]
async fn sliced_backing_is_released_and_stalled_spill_is_byte_and_job_bounded() {
    use std::sync::atomic::Ordering;
    let dir = support::directory();
    let resources = Budget::new(32 * MIB).unwrap();
    let gate = support::IoGate::new();
    let cache = support::gated(dir.path(), &resources, gate.clone()).await;
    gate.hold_writes.store(true, Ordering::SeqCst);
    let source = Budget::new(4 * MIB).unwrap();
    let big = source.try_reserve(4 * MIB).unwrap().zeroed();
    let slice = big.slice(0..MAX_PAYLOAD_BYTES);
    drop(big);
    assert_eq!(
        cache.try_demote(key(0, MAX_PAYLOAD_BYTES), slice),
        Demotion::Admitted
    );
    assert_eq!(
        source.used(),
        0,
        "only the exact admitted copy may cross the async boundary"
    );
    for n in 1..4 {
        assert_eq!(
            cache.try_demote(
                key(n * MAX_PAYLOAD_BYTES as u64, MAX_PAYLOAD_BYTES),
                block(&vec![n as u8; MAX_PAYLOAD_BYTES])
            ),
            Demotion::Admitted
        );
    }
    assert_eq!(
        cache.try_demote(
            key(4 * MAX_PAYLOAD_BYTES as u64, MAX_PAYLOAD_BYTES),
            block(&vec![9; MAX_PAYLOAD_BYTES])
        ),
        Demotion::Dropped
    );
    assert_eq!(cache.stats().active_jobs, 4);
    assert_eq!(cache.stats().copied_bytes, 4 * MAX_PAYLOAD_BYTES);
    assert!(cache.drain(std::time::Duration::ZERO).await.is_err());
    gate.hold_writes.store(false, Ordering::SeqCst);
    gate.release.add_permits(16);
    cache.drain(DEADLINE).await.unwrap();
    assert_eq!(cache.stats().copied_bytes, 0);
    assert_eq!(cache.stats().active_jobs, 0);
    assert!(cache.stats().submission_bytes <= cache.funding().submission_bytes);
    cache.close(DEADLINE).await.unwrap();
}

#[tokio::test]
async fn loaned_backing_is_discarded_not_queued_or_copied() {
    let dir = support::directory();
    let resources = Budget::new(32 * MIB).unwrap();
    let cache = open(dir.path(), &resources).await;
    let original_owner = Budget::new(100).unwrap();
    let loans = Budget::new(100).unwrap();
    let mut victim = block(b"abc").with_owner(original_owner.try_reserve(3).unwrap());
    assert!(victim.try_loan(&loans));
    assert_eq!(cache.try_demote(key(0, 3), victim), Demotion::Dropped);
    assert_eq!(
        (
            loans.used(),
            original_owner.used(),
            cache.stats().copied_bytes
        ),
        (0, 0, 0)
    );
    cache.close(DEADLINE).await.unwrap();
}

#[tokio::test]
async fn cancelled_read_and_close_keep_admission_and_lock_until_real_io_finishes() {
    use std::sync::atomic::Ordering;
    let dir = support::directory();
    let resources = Budget::new(32 * MIB).unwrap();
    let output = Budget::new(MIB).unwrap();
    let gate = support::IoGate::new();
    let cache = support::gated(dir.path(), &resources, gate.clone()).await;
    assert_eq!(
        cache.try_demote(key(0, 3), block(b"abc")),
        Demotion::Admitted
    );
    cache.drain(DEADLINE).await.unwrap();
    gate.hold_reads.store(true, Ordering::SeqCst);
    let readers: Vec<_> = (0..4)
        .map(|_| {
            let cache = cache.clone();
            let output = output.clone();
            tokio::spawn(async move {
                cache
                    .read_block_with_deadline(&key(0, 3), &output, DEADLINE)
                    .await
            })
        })
        .collect();
    tokio::time::timeout(DEADLINE, gate.entered.acquire_many(4))
        .await
        .unwrap()
        .unwrap()
        .forget();
    assert!(
        cache.read_block(&key(0, 3), &output).await.is_none(),
        "saturated reads bypass rather than queue a fifth job"
    );
    assert_eq!(
        cache.try_demote(key(3, 3), block(b"def")),
        Demotion::Dropped
    );
    for reader in &readers {
        reader.abort();
    }
    for reader in readers {
        assert!(matches!(reader.await, Err(error) if error.is_cancelled()));
    }
    assert_eq!(
        output.used(),
        12,
        "cancelled waiter cannot free live output admission"
    );
    assert_eq!(cache.stats().active_jobs, 4);
    assert!(cache.close(std::time::Duration::ZERO).await.is_err());
    assert_eq!(cache.status().state(), State::Closing);
    assert!(PreparedDiskCache::prepare(dir.path(), (16 * MIB) as u64).is_err());
    assert!(resources.used() > 0);
    gate.hold_reads.store(false, Ordering::SeqCst);
    gate.release.add_permits(4);
    cache.close(DEADLINE).await.unwrap();
    assert_eq!(cache.status().state(), State::Closed);
    assert_eq!(output.used(), 0);
    assert!(PreparedDiskCache::prepare(dir.path(), (16 * MIB) as u64).is_ok());
    drop(cache);
    assert_eq!(resources.used(), 0);
}

#[tokio::test]
async fn optional_io_failure_bypasses_without_affecting_source_bytes() {
    use std::sync::atomic::Ordering;
    let dir = support::directory();
    let resources = Budget::new(32 * MIB).unwrap();
    let gate = support::IoGate::new();
    let cache = support::gated(dir.path(), &resources, gate.clone()).await;
    gate.fail_write.store(true, Ordering::SeqCst);
    assert_eq!(
        cache.try_demote(key(0, 3), block(b"abc")),
        Demotion::Admitted
    );
    assert!(cache.drain(DEADLINE).await.is_err());
    assert_eq!(cache.status().state(), State::Bypass);
    assert!(
        cache
            .read_block(&key(0, 3), &Budget::new(3).unwrap())
            .await
            .is_none()
    );
    assert_eq!(
        cache.try_demote(key(3, 3), block(b"def")),
        Demotion::Dropped
    );
    assert!(cache.close(DEADLINE).await.is_err());
}

#[tokio::test]
async fn optional_read_deadline_returns_miss_without_releasing_live_io_funding() {
    use std::sync::atomic::Ordering;
    let dir = support::directory();
    let resources = Budget::new(32 * MIB).unwrap();
    let output = Budget::new(3).unwrap();
    let gate = support::IoGate::new();
    let cache = support::gated(dir.path(), &resources, gate.clone()).await;
    assert_eq!(
        cache.try_demote(key(0, 3), block(b"abc")),
        Demotion::Admitted
    );
    cache.drain(DEADLINE).await.unwrap();
    gate.hold_reads.store(true, Ordering::SeqCst);
    assert!(
        cache
            .read_block_with_deadline(&key(0, 3), &output, std::time::Duration::ZERO)
            .await
            .is_none()
    );
    tokio::time::timeout(DEADLINE, gate.entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    assert_eq!(output.used(), 3);
    assert_eq!(cache.stats().active_jobs, 1);
    gate.hold_reads.store(false, Ordering::SeqCst);
    gate.release.add_permits(1);
    cache.close(DEADLINE).await.unwrap();
    assert_eq!(output.used(), 0);
    assert_eq!(
        cache.stats().hits,
        0,
        "a timed-out caller did not receive a cache hit"
    );
    assert_eq!(cache.stats().misses, 1);
}

#[tokio::test]
async fn bounded_index_and_recent_hints_survive_capacity_pressure_with_hot_payload() {
    let dir = support::directory();
    let resources = Budget::new(32 * MIB).unwrap();
    let output = Budget::new(MIB).unwrap();
    let cache = open(dir.path(), &resources).await;
    let numbered = |n: u64| {
        DiskKey::new(
            b"test",
            format!("pkg/{n}.deb").as_bytes(),
            b"etag",
            MAX_PAYLOAD_BYTES as u64,
            0,
            MAX_PAYLOAD_BYTES,
        )
        .unwrap()
    };
    assert_eq!(
        cache.try_demote(numbered(0), block(&vec![0x71; MAX_PAYLOAD_BYTES])),
        Demotion::Admitted
    );
    cache.drain(DEADLINE).await.unwrap();
    assert_eq!(
        cache.stats().recent_entries,
        0,
        "demotion is not a reference"
    );
    assert!(
        !cache.touch_validated(&numbered(9999)),
        "a hint cannot create an absent record"
    );
    for n in 1..256 {
        assert!(
            cache.touch_validated(&numbered(0)),
            "hot record lost before insertion {n}"
        );
        assert_eq!(
            cache.try_demote(numbered(n), block(&vec![n as u8; MAX_PAYLOAD_BYTES])),
            Demotion::Admitted
        );
        cache.drain(DEADLINE).await.unwrap();
        let stats = cache.stats();
        assert!(stats.index_entries <= cache.funding().index_entries);
        assert!(stats.recent_entries <= cache.funding().recent_entries);
    }
    assert_eq!(
        cache
            .read_block(&numbered(0), &output)
            .await
            .unwrap()
            .as_slice(),
        vec![0x71; MAX_PAYLOAD_BYTES]
    );
    assert!(
        cache.read_block(&numbered(1), &output).await.is_none(),
        "cold victim must be reclaimed"
    );
    assert_eq!(
        std::fs::metadata(dir.path().join("payload-v1.cache"))
            .unwrap()
            .len(),
        (16 * MIB) as u64
    );
    cache.close(DEADLINE).await.unwrap();
}

#[tokio::test]
async fn partial_record_inventory_cannot_expand_metadata_cap() {
    let dir = support::directory();
    let resources = Budget::new(32 * MIB).unwrap();
    let cache = open(dir.path(), &resources).await;
    for n in 0..2304_u64 {
        assert_eq!(
            cache.try_demote(key(n * 3, 3), block(b"abc")),
            Demotion::Admitted
        );
        cache.drain(DEADLINE).await.unwrap();
        assert!(cache.stats().index_entries <= 128);
    }
    assert_eq!(cache.stats().index_entries, 128);
    cache.close(DEADLINE).await.unwrap();
    drop(cache);
    let cache = open(dir.path(), &resources).await;
    assert!(cache.stats().index_entries <= 128);
    assert_eq!(
        cache
            .read_block(&key(2303 * 3, 3), &Budget::new(3).unwrap())
            .await
            .unwrap()
            .as_slice(),
        b"abc"
    );
    cache.close(DEADLINE).await.unwrap();
}

#[tokio::test]
async fn all_referenced_capacity_pressure_still_drains_with_bounded_copy_work() {
    let dir = support::directory();
    let resources = Budget::new(32 * MIB).unwrap();
    let cache = open(dir.path(), &resources).await;
    let numbered = |n| {
        DiskKey::new(
            b"all-hot",
            format!("pkg/{n}").as_bytes(),
            b"v",
            MAX_PAYLOAD_BYTES as u64,
            0,
            MAX_PAYLOAD_BYTES,
        )
        .unwrap()
    };
    for n in 0..192 {
        // Mark every still-indexed old record, not just the eventual survivor.
        // Quotas allow loss, never unbounded copying or blocked required reads.
        for old in 0..n {
            cache.touch_validated(&numbered(old));
        }
        let outcome = cache.try_demote(numbered(n), block(&vec![n as u8; MAX_PAYLOAD_BYTES]));
        assert_ne!(outcome, Demotion::IndexedHint);
        cache.drain(DEADLINE).await.unwrap();
        assert_eq!(cache.stats().active_jobs, 0);
        assert!(cache.stats().index_entries <= cache.funding().index_entries);
        assert!(cache.stats().recent_entries <= cache.funding().recent_entries);
    }
    assert!(
        cache.stats().io_write_bytes < (64 * MIB) as u64,
        "region reinsertion must not amplify this 24-MiB input without bound"
    );
    cache.close(DEADLINE).await.unwrap();
}
