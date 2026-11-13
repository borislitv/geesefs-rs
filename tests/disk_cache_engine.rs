#![forbid(unsafe_code)]

//! Qualification of the released storage API, before selecting a runtime dependency.
use foyer::{
    BlockEngineConfig, Code, DeviceBuilder, FileDeviceBuilder, HybridCache, HybridCacheBuilder,
    InvalidRatioPicker, RecoverMode, StorageFilter, StorageValue,
};
use foyer_storage::test_utils::Switch;
use geesefs_rs::budget::{Block, Budget};
use std::{hash::BuildHasher, num::NonZeroUsize, path::Path, sync::Arc, time::Duration};

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
    engine_with_options(directory, recent, Switch::default()).await
}

async fn engine_with_options<V: StorageValue>(
    directory: &Path,
    recent: Option<Recent>,
    flush_switch: Switch,
) -> HybridCache<u64, V> {
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
        .with_reclaimers(1)
        .with_flush_switch(flush_switch);
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

enum ChargedValue {
    Charged(Block),
    Decoded(Vec<u8>),
}

impl ChargedValue {
    fn bytes(&self) -> &[u8] {
        match self {
            Self::Charged(block) => block.as_slice(),
            Self::Decoded(bytes) => bytes,
        }
    }
}

impl Code for ChargedValue {
    fn encode(&self, writer: &mut impl std::io::Write) -> foyer::Result<()> {
        (self.bytes().len() as u64).encode(writer)?;
        writer
            .write_all(self.bytes())
            .map_err(foyer::Error::io_error)
    }

    fn decode(reader: &mut impl std::io::Read) -> foyer::Result<Self> {
        let size = u64::decode(reader)?;
        if size > (128 * KIB) as u64 {
            return Err(foyer::Error::new(
                foyer::ErrorKind::OutOfRange,
                "oversized test record",
            ));
        }
        let mut bytes = vec![0; size as usize];
        reader
            .read_exact(&mut bytes)
            .map_err(foyer::Error::io_error)?;
        Ok(Self::Decoded(bytes))
    }

    fn estimated_size(&self) -> usize {
        8 + self.bytes().len()
    }
}

#[tokio::test]
async fn a_stalled_flusher_retains_backing_charges_after_waiter_cancellation() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let directory = tempfile::tempdir().unwrap();
        let flush = Switch::default();
        flush.on();
        let cache = engine_with_options(directory.path(), None, flush.clone()).await;
        let budget = Budget::new(256 * KIB).unwrap();
        let backing = budget.try_reserve(256 * KIB).unwrap().zeroed();
        let view = backing.slice(0..128 * KIB);
        drop(backing);
        drop(
            cache
                .writer(1)
                .storage()
                .insert(ChargedValue::Charged(view))
                .unwrap(),
        );
        assert!(cache.storage().may_contains(&1));

        let mut waiting = Box::pin(cache.storage().wait());
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        assert!(std::future::Future::poll(waiting.as_mut(), &mut context).is_pending());
        drop(waiting);
        assert_eq!(
            budget.used(),
            256 * KIB,
            "a slice retains the entire backing, not its encoded length"
        );
        assert!(
            budget.try_reserve(1).is_err(),
            "cancelling the waiter does not release the writer owner"
        );

        flush.off();
        cache.storage().wait().await;
        assert_eq!(
            budget.used(),
            0,
            "completed I/O releases the original backing"
        );
        assert!(budget.try_reserve(256 * KIB).is_ok());
        let (_, decoded, _) = cache.storage().load(&1).await.unwrap().entry().unwrap();
        assert_eq!(decoded.bytes().len(), 128 * KIB);
        assert!(decoded.bytes().iter().all(|byte| *byte == 0));
        cache.close().await.unwrap();
    })
    .await
    .expect("stalled-owner qualification must finish after releasing the gate");
}

#[tokio::test]
async fn closing_and_dropping_a_cache_releases_its_partitions() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let directory = tempfile::tempdir().unwrap();
        let cache = engine(directory.path()).await;
        let partition = cache.storage().device().unwrap().partition(0);
        let owner = Arc::downgrade(&partition);
        drop(partition);
        drop(cache.writer(1).storage().insert(b"completed".to_vec()));
        cache.close().await.unwrap();
        drop(cache);
        assert!(
            owner.upgrade().is_none(),
            "a closed cache must release its idle workers and file owners"
        );
    })
    .await
    .expect("cache close must join its workers");
}
