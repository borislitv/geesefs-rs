#![forbid(unsafe_code)]
#![cfg(target_pointer_width = "64")]

#[path = "support/disk.rs"]
mod support;
use budget::Budget;
use disk_cache::{Demotion, DiskCache, Funding, PreparedDiskCache};
use error::Error;
use std::{
    fs::{self, File, OpenOptions},
    os::unix::fs::{FileExt, MetadataExt, OpenOptionsExt},
    path::Path,
};
use support::{DEADLINE, MIB, block, key};
pub use support::{budget, disk_cache, error};

const TWO_TIB: u64 = 2 << 40;
const THREE_TB: u64 = 3_000_000_000_000;
const TARGET: u64 = TWO_TIB + 1_000_000_000_000;
const THREE_TIB: u64 = 3 << 40;
const EIGHT_TIB: u64 = 8 << 40;
const TEN_TB: u64 = 10_000_000_000_000;
const LEGACY: &str = "payload-upstream-v1.cache";
const R32: &str = "payload-upstream-r32-v1.cache";
const R64: &str = "payload-upstream-r64-v1.cache";
const SENTINEL: [u8; 8] = *b"preserve";

#[test]
fn region_boundaries() {
    for (capacity, region_mib) in [
        (256 * MIB as u64 - 4096, 1),
        (256 * MIB as u64, 16),
        (TWO_TIB, 16),
        (THREE_TB - 4096, 16),
        (THREE_TB, 32),
        (THREE_TB + 4096, 32),
        (TARGET, 32),
        (THREE_TIB, 32),
        (EIGHT_TIB - 4096, 32),
        (EIGHT_TIB, 64),
        (EIGHT_TIB + 4096, 64),
        (2 * EIGHT_TIB, 64),
    ] {
        let funding = Funding::derive(capacity, usize::MAX).unwrap();
        assert_eq!(funding.capacity as u64, capacity);
        assert_eq!(funding.region_bytes, region_mib * MIB, "{capacity} bytes");
    }
    assert!(Funding::derive(2 * EIGHT_TIB + 4096, usize::MAX).is_err());
}

#[test]
fn larger_regions_reduce_funding_and_fund_without_allocating_media() {
    // With 16 MiB regions these capacities needed 9,437,003,520 and
    // 29,472,002,944 bytes respectively, including per-region allowances.
    for (capacity, regions, required) in [
        (TARGET, 95_338, 9_411_073_280),
        (TEN_TB, 149_011, 29_350_410_368),
    ] {
        let funding = Funding::derive(capacity, usize::MAX).unwrap();
        assert_eq!(funding.capacity / funding.region_bytes, regions);
        assert_eq!(funding.required_bytes, required);

        let dir = support::directory();
        let resources = Budget::new(required).unwrap();
        let prepared = PreparedDiskCache::prepare(dir.path(), capacity).unwrap();
        assert!(matches!(
            prepared.funding(required - 1),
            Err(Error::Budget { requested, .. }) if requested == required
        ));
        let funded = prepared.fund(&resources).unwrap();
        assert_eq!(resources.used(), required);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1); // only the lock
        funded.close();
        assert_eq!(resources.used(), 0);
    }
}

fn sparse_media(path: &Path, capacity: u64) -> fs::Metadata {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .unwrap();
    file.set_len(capacity).unwrap();
    file.write_all_at(&SENTINEL, capacity - SENTINEL.len() as u64)
        .unwrap();
    let meta = file.metadata().unwrap();
    assert!(meta.blocks() * 512 < capacity);
    meta
}

fn assert_preserved(path: &Path, before: &fs::Metadata) {
    let file = File::open(path).unwrap();
    let after = file.metadata().unwrap();
    assert_eq!(after.len(), before.len());
    assert_eq!(after.blocks(), before.blocks());
    assert_eq!((after.dev(), after.ino()), (before.dev(), before.ino()));
    let mut sentinel = [0; SENTINEL.len()];
    file.read_exact_at(&mut sentinel, after.len() - SENTINEL.len() as u64)
        .unwrap();
    assert_eq!(sentinel, SENTINEL);
}

#[test]
fn only_the_selected_inventory_is_recognized_at_the_same_capacity() {
    for (capacity, recognized) in [
        (16 * MIB as u64, LEGACY),
        (256 * MIB as u64, LEGACY),
        (TWO_TIB, LEGACY),
        (THREE_TB, R32),
        (TARGET, R32),
        (THREE_TIB, R32),
        (EIGHT_TIB - 4096, R32),
        (EIGHT_TIB, R64),
        (TEN_TB, R64),
    ] {
        for names in [
            &[LEGACY][..],
            &[R32],
            &[R64],
            &[LEGACY, R32],
            &[LEGACY, R64],
            &[R32, R64],
        ] {
            let dir = support::directory();
            let media: Vec<_> = names
                .iter()
                .map(|name| {
                    let path = dir.path().join(name);
                    let meta = sparse_media(&path, capacity);
                    (path, meta)
                })
                .collect();
            // Prepare checks only metadata; never open these sparse fixtures as engines.
            let result = PreparedDiskCache::prepare(dir.path(), capacity);
            if names == [recognized] {
                let funded = result
                    .unwrap()
                    .fund(&Budget::new(32 * 1024 * MIB).unwrap())
                    .unwrap();
                assert!(funded.has_inventory());
                funded.close();
            } else {
                assert!(
                    matches!(result, Err(Error::Unsupported(_))),
                    "accepted {names:?} at {capacity} bytes"
                );
            }
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), names.len() + 1);
            for (path, meta) in media {
                assert_preserved(&path, &meta);
            }
        }
    }
}

#[tokio::test]
async fn deferred_open_refuses_wrong_layout_introduced_after_prepare() {
    for (capacity, expected, foreign) in [(16 * MIB as u64, LEGACY, R64), (TARGET, R32, LEGACY)] {
        let dir = support::directory();
        let resources = Budget::new(32 * 1024 * MIB).unwrap();
        let prepared = PreparedDiskCache::prepare(dir.path(), capacity).unwrap();
        let status = prepared.status();
        let path = dir.path().join(foreign);
        let meta = sparse_media(&path, capacity);

        let result = DiskCache::open(prepared, &resources).await;
        assert!(matches!(result, Err(Error::Unsupported(_))));
        assert_eq!(resources.used(), 0);
        assert_eq!(status.startup().recovery.count, 0);
        assert!(!dir.path().join(expected).exists());
        assert_preserved(&path, &meta);
    }
}

#[tokio::test]
async fn r32_store_recovers_old_bytes_and_accepts_new_bytes_after_reopen() {
    assert_layout_reopen(TARGET, 32, R32).await;
}

#[tokio::test]
async fn r64_store_recovers_old_bytes_and_accepts_new_bytes_after_reopen() {
    assert_layout_reopen(TEN_TB, 64, R64).await;
}

async fn assert_layout_reopen(configured_capacity: u64, region_mib: u64, filename: &str) {
    let dir = support::directory();
    let resources = Budget::new(32 * MIB).unwrap();
    let output = Budget::new(MIB).unwrap();
    let old: Vec<_> = (0..128 * 1024).map(|n| (n % 251) as u8).collect();
    let new: Vec<_> = old.iter().map(|byte| !byte).collect();
    let old_key = key(0, old.len());
    let new_key = key(old.len() as u64, new.len());
    let open = || async {
        tokio::time::timeout(
            DEADLINE,
            DiskCache::open(
                PreparedDiskCache::prepare_layout_fixture(dir.path(), configured_capacity).unwrap(),
                &resources,
            ),
        )
        .await
        .unwrap()
        .unwrap()
    };

    let cache = open().await;
    assert_eq!(cache.stats().regions.size_bytes, region_mib * MIB as u64);
    let media = dir.path().join(filename);
    let before = fs::metadata(&media).unwrap();
    assert_eq!(before.len(), 256 * MIB as u64);
    assert!(!dir.path().join(LEGACY).exists());
    assert_eq!(
        cache.try_demote(old_key.clone(), block(&old)),
        Demotion::Admitted
    );
    cache.drain(DEADLINE).await.unwrap();
    cache.close(DEADLINE).await.unwrap();
    drop(cache);

    let cache = open().await;
    assert_eq!(cache.stats().regions.size_bytes, region_mib * MIB as u64);
    let after = fs::metadata(&media).unwrap();
    assert_eq!(
        (after.dev(), after.ino(), after.len()),
        (before.dev(), before.ino(), before.len())
    );
    assert_eq!(
        cache
            .read_block(&old_key, &output)
            .await
            .unwrap()
            .as_slice(),
        old
    );
    assert_eq!(
        cache.try_demote(new_key.clone(), block(&new)),
        Demotion::Admitted
    );
    cache.drain(DEADLINE).await.unwrap();
    assert_eq!(
        cache
            .read_block(&new_key, &output)
            .await
            .unwrap()
            .as_slice(),
        new
    );
    cache.close(DEADLINE).await.unwrap();
}
