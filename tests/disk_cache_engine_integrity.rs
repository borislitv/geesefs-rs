#![forbid(unsafe_code)]

//! Native hash-collision qualification. Adapter tests cover identity and corruption.
use foyer::{
    BlockEngineConfig, DeviceBuilder, FileDeviceBuilder, HybridCache, HybridCacheBuilder,
    RecoverMode,
};
use foyer_storage::{Load, test_utils::Switch};
use std::{
    hash::{BuildHasher, BuildHasherDefault, Hasher},
    path::Path,
    time::Duration,
};

#[derive(Clone, Default)]
struct CollisionHasher;

impl Hasher for CollisionHasher {
    fn finish(&self) -> u64 {
        0x1357_2468
    }
    fn write(&mut self, _: &[u8]) {}
}

type HasherBuilder = BuildHasherDefault<CollisionHasher>;
type Cache = HybridCache<String, Vec<u8>, HasherBuilder>;

async fn engine(directory: &Path, flush: Switch) -> Cache {
    let device = FileDeviceBuilder::new(directory.join("payload.cache"))
        .with_capacity(16 * 1024 * 1024)
        .build()
        .unwrap();
    let config = BlockEngineConfig::new(device)
        .with_block_size(256 * 1024)
        .with_buffer_pool_size(512 * 1024)
        .with_submit_queue_size_threshold(1024 * 1024)
        .with_recover_concurrency(1)
        .with_flushers(1)
        .with_reclaimers(1)
        .with_flush_switch(flush);
    HybridCacheBuilder::new()
        .with_flush_on_close(false)
        .memory(1)
        .with_hash_builder(HasherBuilder::default())
        .storage()
        .with_engine_config(config)
        .with_recover_mode(RecoverMode::Quiet)
        .build()
        .await
        .unwrap()
}

async fn put(cache: &Cache, key: &str, value: &[u8]) {
    drop(
        cache
            .writer(key.to_owned())
            .storage()
            .insert(value.to_vec())
            .unwrap(),
    );
    cache.storage().wait().await;
    assert!(
        cache.memory().get(key).is_none(),
        "must exercise disk, not L1"
    );
    assert_eq!(
        cache.storage().load(key).await.unwrap().kv(),
        Some((key.to_owned(), value.to_vec()))
    );
}

#[tokio::test]
async fn equal_hash_different_full_keys_never_alias_bytes() {
    tokio::time::timeout(Duration::from_secs(20), async {
        let dir = tempfile::tempdir().unwrap();
        let hasher = HasherBuilder::default();
        assert_eq!(hasher.hash_one("object-a"), hasher.hash_one("object-b"));
        let cache = engine(dir.path(), Switch::default()).await;
        put(
            &cache,
            "object-a",
            b"first object's independently known bytes",
        )
        .await;
        assert!(cache.storage().load("object-b").await.unwrap().is_miss());
        put(&cache, "object-b", b"second object's different bytes").await;
        // A hash collision may evict A, but must never return B as A.
        if let Some((key, value)) = cache.storage().load("object-a").await.unwrap().kv() {
            assert_eq!(key, "object-a");
            assert_eq!(value, b"first object's independently known bytes");
        }
        cache.close().await.unwrap();
        drop(cache);
        let cache = engine(dir.path(), Switch::default()).await;
        assert!(cache.storage().load("object-a").await.unwrap().is_miss());
        assert_eq!(
            cache
                .storage()
                .load("object-b")
                .await
                .unwrap()
                .kv()
                .unwrap()
                .1,
            b"second object's different bytes"
        );
        cache.close().await.unwrap();
    })
    .await
    .expect("collision qualification deadline");
}

#[tokio::test]
async fn equal_hash_pending_keeper_values_are_matched_by_full_key() {
    tokio::time::timeout(Duration::from_secs(20), async {
        let dir = tempfile::tempdir().unwrap();
        let flush = Switch::default();
        flush.on();
        let cache = engine(dir.path(), flush.clone()).await;
        let first = cache
            .memory()
            .insert("pending-a".to_owned(), b"AAAA".to_vec());
        let second = cache
            .memory()
            .insert("pending-b".to_owned(), b"BBBB".to_vec());
        assert_eq!(first.hash(), second.hash());
        cache.storage().enqueue(first.piece(), true);
        cache.storage().enqueue(second.piece(), true);
        for (key, expected) in [("pending-a", b"AAAA"), ("pending-b", b"BBBB")] {
            match cache.storage().load(key).await.unwrap() {
                Load::Piece { piece, .. } => {
                    assert_eq!(piece.key(), key);
                    assert_eq!(piece.value(), expected);
                }
                other => panic!("expected keeper piece for {key}, got {other:?}"),
            }
        }
        assert!(cache.storage().load("pending-c").await.unwrap().is_miss());
        flush.off();
        cache.storage().wait().await;
        cache.close().await.unwrap();
    })
    .await
    .expect("pending collision qualification deadline");
}
