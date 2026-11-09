#![forbid(unsafe_code)]

//! Qualification of the released storage API, before selecting a runtime dependency.
use foyer::{
    BlockEngineConfig, DeviceBuilder, FileDeviceBuilder, HybridCache, HybridCacheBuilder,
    InvalidRatioPicker, RecoverMode, StorageFilter,
};
use std::{hash::BuildHasher, num::NonZeroUsize, path::Path, time::Duration};

#[path = "support/disk_cache_policy.rs"]
mod disk_cache_policy;
use disk_cache_policy::Recent;

const KIB: usize = 1024;
const MIB: usize = 1024 * KIB;

async fn engine(directory: &Path) -> HybridCache<u64, Vec<u8>> {
    engine_with_recency(directory, None).await
}

async fn engine_with_recency(
    directory: &Path,
    recent: Option<Recent>,
) -> HybridCache<u64, Vec<u8>> {
    let device = FileDeviceBuilder::new(directory.join("payload.cache"))
        .with_capacity(16 * MIB)
        .build()
        .unwrap();
    let mut config = BlockEngineConfig::new(device)
        .with_block_size(MIB)
        .with_buffer_pool_size(512 * KIB)
        .with_submit_queue_size_threshold(MIB)
        .with_indexer_shards(4)
        .with_recover_concurrency(1)
        .with_flushers(1)
        .with_reclaimers(1);
    if let Some(recent) = recent {
        config = config
            .with_eviction_pickers(vec![
                Box::new(InvalidRatioPicker::new(0.8)),
                Box::new(recent.picker()),
            ])
            .with_reinsertion_filter(StorageFilter::new().with_condition(recent));
    }
    HybridCacheBuilder::new()
        .memory(1)
        .storage()
        .with_engine_config(config)
        .with_recover_mode(RecoverMode::Strict)
        .build()
        .await
        .unwrap()
}

#[tokio::test]
async fn disk_only_records_are_recovered_without_memory_cache_ownership() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let directory = tempfile::tempdir().unwrap();
        let cache = engine(directory.path()).await;
        let expected = b"a package range with independently specified bytes".repeat(2048);
        let entry = cache
            .writer(42)
            .storage()
            .insert(expected.clone())
            .expect("disk-only writer admits a bounded record");
        drop(entry);
        cache.storage().wait().await;
        assert!(cache.memory().get(&42).is_none());
        let stored = cache.storage().load(&42).await.unwrap().kv().unwrap();
        assert_eq!(stored, (42, expected.clone()));
        cache.close().await.unwrap();
        drop(cache);

        let reopened = engine(directory.path()).await;
        assert!(reopened.memory().get(&42).is_none());
        let stored = reopened.storage().load(&42).await.unwrap().kv().unwrap();
        assert_eq!(stored, (42, expected));
        reopened.close().await.unwrap();
    })
    .await
    .expect("local engine roundtrip and reopen must finish");
}

#[tokio::test]
async fn region_recycling_preserves_a_bounded_physical_container() {
    tokio::time::timeout(Duration::from_secs(20), async {
        let directory = tempfile::tempdir().unwrap();
        let cache = engine(directory.path()).await;
        for number in 0..256_u64 {
            let payload = vec![(number % 251) as u8; 128 * KIB];
            drop(cache.writer(number).storage().insert(payload));
            cache.storage().wait().await;
        }
        let last = cache.storage().load(&255).await.unwrap().kv().unwrap();
        assert_eq!(last, (255, vec![4; 128 * KIB]));
        assert!(cache.storage().load(&0).await.unwrap().kv().is_none());
        assert_eq!(
            std::fs::metadata(directory.path().join("payload.cache"))
                .unwrap()
                .len(),
            (16 * MIB) as u64
        );
        cache.close().await.unwrap();
    })
    .await
    .expect("storage recycling must finish under sustained insertion");
}

async fn hot_scan(directory: &Path) -> HybridCache<u64, Vec<u8>> {
    let recent = Recent::new(NonZeroUsize::new(64).unwrap());
    let cache = engine_with_recency(directory, Some(recent.clone())).await;
    for number in 0..64_u64 {
        drop(
            cache
                .writer(number)
                .storage()
                .insert(vec![number as u8; 128 * KIB]),
        );
        cache.storage().wait().await;
    }
    for number in 64..256_u64 {
        let (key, payload) = cache
            .storage()
            .load(&0)
            .await
            .unwrap()
            .kv()
            .unwrap_or_else(|| panic!("hot record missing at scan step {number}"));
        assert_eq!(key, 0);
        assert_eq!(payload.len(), 128 * KIB);
        assert!(payload.iter().all(|byte| *byte == 0));
        recent.touch(cache.memory().hash_builder().hash_one(0_u64));
        drop(
            cache
                .writer(number)
                .storage()
                .insert(vec![(number % 251) as u8; 128 * KIB]),
        );
        cache.storage().wait().await;
    }
    cache
}

#[tokio::test]
async fn a_repeatedly_read_record_survives_a_cold_scan() {
    tokio::time::timeout(Duration::from_secs(20), async {
        let directory = tempfile::tempdir().unwrap();
        let cache = hot_scan(directory.path()).await;
        // Consuming the reference must not permanently pin a formerly hot record.
        for number in 256..512_u64 {
            drop(
                cache
                    .writer(number)
                    .storage()
                    .insert(vec![(number % 251) as u8; 128 * KIB]),
            );
            cache.storage().wait().await;
        }
        assert!(cache.storage().load(&0).await.unwrap().kv().is_none());
        cache.close().await.unwrap();
    })
    .await
    .expect("hot/cold scan must finish");
}

#[tokio::test]
async fn reinserted_records_survive_reopen() {
    tokio::time::timeout(Duration::from_secs(20), async {
        let directory = tempfile::tempdir().unwrap();
        let cache = hot_scan(directory.path()).await;
        cache.close().await.unwrap();
        let mut resident = Vec::new();
        for key in 0..256_u64 {
            if cache.storage().load(&key).await.unwrap().kv().is_some() {
                resident.push(key);
            }
        }
        assert!(resident.contains(&0));
        drop(cache);
        let cache = engine(directory.path()).await;
        let mut missing = Vec::new();
        for key in resident {
            let Some((stored_key, payload)) = cache.storage().load(&key).await.unwrap().kv() else {
                missing.push(key);
                continue;
            };
            assert_eq!(stored_key, key);
            assert_eq!(payload.len(), 128 * KIB);
            assert!(payload.iter().all(|byte| *byte == (key % 251) as u8));
        }
        cache.close().await.unwrap();
        assert!(
            missing.is_empty(),
            "resident keys missing after reinsertion and reopen: {missing:?}"
        );
    })
    .await
    .expect("recovery of reinserted records must finish");
}
