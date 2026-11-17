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

const EIGHT_TIB: u64 = 8 << 40;
const TEN_TB: u64 = 10_000_000_000_000;
const LEGACY: &str = "payload-upstream-v1.cache";
const R64: &str = "payload-upstream-r64-v1.cache";
const SENTINEL: [u8; 8] = *b"preserve";

#[test]
fn region_boundaries_and_ten_decimal_tb_funding() {
    for (capacity, region_mib) in [
        (256 * MIB as u64 - 4096, 1),
        (256 * MIB as u64, 16),
        (EIGHT_TIB - 4096, 16),
        (EIGHT_TIB, 64),
        (EIGHT_TIB + 4096, 64),
        (2 * EIGHT_TIB, 64),
    ] {
        let funding = Funding::derive(capacity, usize::MAX).unwrap();
        assert_eq!(funding.capacity as u64, capacity);
        assert_eq!(funding.region_bytes, region_mib * MIB, "{capacity} bytes");
    }
    assert!(Funding::derive(2 * EIGHT_TIB + 4096, usize::MAX).is_err());

    let funding = Funding::derive(TEN_TB, usize::MAX).unwrap();
    assert_eq!(funding.capacity / funding.region_bytes, 149_011);
    // Former 16 MiB geometry: 596,046 regions, 29,472,002,944 bytes.
    assert_eq!(funding.required_bytes, 29_350_410_368);
    assert_eq!(29_472_002_944 - funding.required_bytes, 121_592_576);

    let dir = support::directory();
    let resources = Budget::new(29_350_410_368).unwrap();
    let prepared = PreparedDiskCache::prepare(dir.path(), TEN_TB).unwrap();
    assert!(matches!(
        prepared.funding(resources.limit() - 1),
        Err(Error::Budget {
            requested: 29_350_410_368,
            ..
        })
    ));
    let funded = prepared.fund(&resources).unwrap();
    assert_eq!(resources.used(), resources.limit());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1); // only the lock
    funded.close();
    assert_eq!(resources.used(), 0);
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
        (EIGHT_TIB - 4096, LEGACY),
        (EIGHT_TIB, R64),
        (TEN_TB, R64),
    ] {
        for names in [&[LEGACY][..], &[R64], &[LEGACY, R64]] {
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
    let dir = support::directory();
    let capacity = 16 * MIB as u64;
    let prepared = PreparedDiskCache::prepare(dir.path(), capacity).unwrap();
    let path = dir.path().join(R64);
    let meta = sparse_media(&path, capacity);

    let result = DiskCache::open(prepared, &Budget::new(32 * MIB).unwrap()).await;
    assert!(matches!(result, Err(Error::Unsupported(_))));
    assert!(!dir.path().join(LEGACY).exists());
    assert_preserved(&path, &meta);
}

#[tokio::test]
async fn r64_store_recovers_old_bytes_and_accepts_new_bytes_after_reopen() {
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
                PreparedDiskCache::prepare_r64_fixture(dir.path()).unwrap(),
                &resources,
            ),
        )
        .await
        .unwrap()
        .unwrap()
    };

    let cache = open().await;
    assert_eq!(cache.stats().regions.size_bytes, 64 * MIB as u64);
    let media = dir.path().join(R64);
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
    assert_eq!(cache.stats().regions.size_bytes, 64 * MIB as u64);
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
