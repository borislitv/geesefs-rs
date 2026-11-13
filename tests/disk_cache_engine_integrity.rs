#![forbid(unsafe_code)]

//! Public-API qualification of the pinned Foyer fork, not S3/FUSE fallback tests.
use foyer::{
    BlockEngineConfig, DeviceBuilder, FileDeviceBuilder, HybridCache, HybridCacheBuilder,
    RecoverMode,
};
use foyer_storage::{Load, test_utils::Switch};
use std::{
    collections::hash_map::DefaultHasher,
    hash::{BuildHasher, BuildHasherDefault, Hasher},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
    time::Duration,
};

const KIB: usize = 1024;
const CAPACITY: usize = 16 * 1024 * KIB;
const REGION: usize = 256 * KIB;
type Stable = BuildHasherDefault<DefaultHasher>;

#[derive(Clone, Default)]
struct CollisionHasher;

impl Hasher for CollisionHasher {
    fn finish(&self) -> u64 {
        0x1357_2468
    }

    fn write(&mut self, _: &[u8]) {}
}

async fn engine<S>(
    directory: &Path,
    hasher: S,
    tombstones: bool,
    flush: Switch,
) -> HybridCache<String, Vec<u8>, S>
where
    S: BuildHasher + Clone + Send + Sync + std::fmt::Debug + 'static,
{
    let device = FileDeviceBuilder::new(directory.join("payload.cache"))
        .with_capacity(CAPACITY)
        .build()
        .unwrap();
    let config = BlockEngineConfig::new(device)
        .with_block_size(REGION)
        .with_buffer_pool_size(512 * KIB)
        .with_submit_queue_size_threshold(1024 * KIB)
        .with_indexer_shards(4)
        .with_recover_concurrency(1)
        .with_flushers(1)
        .with_reclaimers(1)
        .with_tombstone_log(tombstones)
        .with_flush_switch(flush);
    HybridCacheBuilder::new()
        // Qualify Store deletion independently of HybridCache's optional
        // close-time L1 flush, which would be a new enqueue after deletion.
        .with_flush_on_close(false)
        .memory(1)
        .with_hash_builder(hasher)
        .storage()
        .with_engine_config(config)
        .with_recover_mode(RecoverMode::Strict)
        .build()
        .await
        .unwrap()
}

async fn put<S>(cache: &HybridCache<String, Vec<u8>, S>, key: &str, value: &[u8])
where
    S: BuildHasher + Clone + Send + Sync + std::fmt::Debug + 'static,
{
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
        let hasher = BuildHasherDefault::<CollisionHasher>::default();
        assert_eq!(hasher.hash_one("object-a"), hasher.hash_one("object-b"));
        let cache = engine(dir.path(), hasher.clone(), false, Switch::default()).await;
        put(
            &cache,
            "object-a",
            b"first object's independently known bytes",
        )
        .await;
        assert!(cache.storage().load("object-b").await.unwrap().is_miss());
        put(&cache, "object-b", b"second object's different bytes").await;
        // The hash-only disk index may evict A on collision. It must never return B as A.
        if let Some((key, value)) = cache.storage().load("object-a").await.unwrap().kv() {
            assert_eq!(key, "object-a");
            assert_eq!(value, b"first object's independently known bytes");
        }
        cache.close().await.unwrap();
        drop(cache);
        let cache = engine(dir.path(), hasher, false, Switch::default()).await;
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
        let cache = engine(
            dir.path(),
            BuildHasherDefault::<CollisionHasher>::default(),
            false,
            flush.clone(),
        )
        .await;
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

#[tokio::test]
async fn immutable_backend_object_etag_size_and_range_identities_survive_reopen() {
    tokio::time::timeout(Duration::from_secs(20), async {
        let dir = tempfile::tempdir().unwrap();
        // Explicit identities, not process-local inode/generation hashes. This is a
        // fixture encoding only; the FS adapter still needs its own unambiguous codec.
        let records: [(&str, &[u8]); 8] = [
            ("v1|backend-a|object-a|etag-a|100|0|4", b"AAAA"),
            ("v1|backend-b|object-a|etag-a|100|0|4", b"BBBB"),
            ("v1|backend-a|object-b|etag-a|100|0|4", b"CCCC"),
            ("v1|backend-a|object-a|etag-b|100|0|4", b"DDDD"),
            ("v1|backend-a|object-a|etag-a|101|0|4", b"EEEE"),
            ("v1|backend-a|object-a|etag-a|100|4|4", b"FFFF"),
            ("v1|backend-a|object-a|etag-a|100|0|3", b"GGG"),
            ("v2|backend-a|object-a|etag-a|100|0|4", b"HHHH"),
        ];
        let cache = engine(dir.path(), Stable::default(), true, Switch::default()).await;
        for (key, bytes) in records {
            put(&cache, key, bytes).await;
        }
        cache.close().await.unwrap();
        drop(cache);
        let cache = engine(dir.path(), Stable::default(), true, Switch::default()).await;
        for (key, bytes) in records {
            assert_eq!(
                cache.storage().load(key).await.unwrap().kv(),
                Some((key.to_owned(), bytes.to_vec()))
            );
        }
        assert!(
            cache
                .storage()
                .load("v1|backend-a|object-a|etag-new|100|0|4")
                .await
                .unwrap()
                .is_miss()
        );
        cache.close().await.unwrap();
    })
    .await
    .expect("immutable identity qualification deadline");
}

#[tokio::test]
async fn completed_delete_with_tombstones_survives_reopen() {
    tokio::time::timeout(Duration::from_secs(20), async {
        let dir = tempfile::tempdir().unwrap();
        let cache = engine(dir.path(), Stable::default(), true, Switch::default()).await;
        put(&cache, "deleted-generation", b"obsolete bytes").await;
        put(&cache, "unrelated-generation", b"still valid bytes").await;
        cache.storage().delete("deleted-generation");
        cache.storage().wait().await;
        assert!(
            cache
                .storage()
                .load("deleted-generation")
                .await
                .unwrap()
                .is_miss()
        );
        cache.close().await.unwrap();
        drop(cache);
        let cache = engine(dir.path(), Stable::default(), true, Switch::default()).await;
        assert!(
            cache
                .storage()
                .load("deleted-generation")
                .await
                .unwrap()
                .is_miss()
        );
        assert_eq!(
            cache
                .storage()
                .load("unrelated-generation")
                .await
                .unwrap()
                .kv()
                .unwrap()
                .1,
            b"still valid bytes"
        );
        cache.close().await.unwrap();
    })
    .await
    .expect("persistent deletion qualification deadline");
}

#[tokio::test]
async fn pending_keeper_remains_readable_after_delete_until_flush_finishes() {
    tokio::time::timeout(Duration::from_secs(20), async {
        let dir = tempfile::tempdir().unwrap();
        let flush = Switch::default();
        flush.on();
        let cache = engine(dir.path(), Stable::default(), true, flush.clone()).await;
        // Explicit enqueue establishes the pending keeper before deletion.
        let entry = cache.memory().insert(
            "pending-generation".to_owned(),
            b"pending old bytes".to_vec(),
        );
        cache.storage().enqueue(entry.piece(), true);
        assert_eq!(
            entry.hash(),
            Stable::default().hash_one("pending-generation")
        );
        assert!(
            cache.storage().may_contains("pending-generation"),
            "explicitly enqueued value must be pending before deletion"
        );
        cache.storage().delete("pending-generation");
        // Load::kv() intentionally excludes keeper pieces; None from kv() is
        // not proof of a miss. Match the public result and inspect its bytes.
        match cache.storage().load("pending-generation").await.unwrap() {
            Load::Piece { piece, .. } => {
                assert_eq!(piece.key(), "pending-generation");
                assert_eq!(piece.value(), b"pending old bytes");
            }
            other => panic!("expected the pending keeper piece, got {other:?}"),
        }
        flush.off();
        cache.storage().wait().await;
        assert!(
            cache
                .storage()
                .load("pending-generation")
                .await
                .unwrap()
                .is_miss()
        );
        cache.close().await.unwrap();
        drop(entry);
        drop(cache);
        let cache = engine(dir.path(), Stable::default(), true, Switch::default()).await;
        assert!(
            cache
                .storage()
                .load("pending-generation")
                .await
                .unwrap()
                .is_miss()
        );
        cache.close().await.unwrap();
    })
    .await
    .expect("pending delete qualification deadline");
}

#[derive(Clone, Copy, Debug)]
enum Damage {
    Payload,
    Index,
}

async fn corrupt_and_recover(damage: Damage) {
    let dir = tempfile::tempdir().unwrap();
    // Each large record occupies a different region: index damage can discard
    // its whole blob/region, but must not clear an unrelated completed region.
    let damaged = b"damaged payload marker: 0123456789".repeat(6000);
    let unaffected = b"unaffected payload marker: abcdef".repeat(6000);
    let cache = engine(dir.path(), Stable::default(), false, Switch::default()).await;
    put(&cache, "damaged-record", &damaged).await;
    put(
        &cache,
        "same-region-companion",
        b"neighbor bytes remain intact",
    )
    .await;
    put(&cache, "unaffected-record", &unaffected).await;
    cache.close().await.unwrap();
    drop(cache);
    let path = dir.path().join("payload.cache");
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    assert_eq!(file.metadata().unwrap().len(), CAPACITY as u64);
    let mut image = vec![0; CAPACITY];
    file.read_exact(&mut image).unwrap();
    let locate = |needle: &[u8]| {
        let matches: Vec<_> = image
            .windows(needle.len())
            .enumerate()
            .filter_map(|(offset, bytes)| (bytes == needle).then_some(offset))
            .collect();
        assert_eq!(matches.len(), 1, "identify one complete on-disk fixture");
        matches[0]
    };
    let payload = locate(&damaged);
    let companion = locate(b"neighbor bytes remain intact");
    let other = locate(&unaffected);
    assert_eq!(payload / REGION, companion / REGION);
    assert_ne!(payload / REGION, other / REGION);
    let offset = match damage {
        Damage::Payload => payload + damaged.len() / 2,
        Damage::Index => {
            // Pinned fork format: 4-KiB blob index, 28-byte versioned header,
            // then hash/sequence/offset/length. Identify the actual hash in the
            // region's first index, not an arbitrary byte or a forged length.
            let start = payload / REGION * REGION;
            let hash = Stable::default().hash_one("damaged-record").to_be_bytes();
            assert_eq!(&image[start + 28..start + 36], &hash);
            start + 28
        }
    };
    file.seek(SeekFrom::Start(offset as u64)).unwrap();
    file.write_all(&[image[offset] ^ 0x80]).unwrap();
    file.sync_all().unwrap();
    drop(file);
    let cache = engine(dir.path(), Stable::default(), false, Switch::default()).await;
    match cache.storage().load("damaged-record").await {
        Ok(result) => assert!(
            result.is_miss(),
            "{damage:?} corruption must not return bytes"
        ),
        Err(error) => eprintln!("{damage:?} corruption rejected: {error}"),
    }
    assert_eq!(
        cache
            .storage()
            .load("unaffected-record")
            .await
            .unwrap()
            .kv(),
        Some(("unaffected-record".to_owned(), unaffected))
    );
    if matches!(damage, Damage::Payload) {
        assert_eq!(
            cache
                .storage()
                .load("same-region-companion")
                .await
                .unwrap()
                .kv()
                .unwrap()
                .1,
            b"neighbor bytes remain intact"
        );
    } else {
        // A bad blob index stops recovery of the remaining region, including
        // later intact blobs. Containment is region-level, not record-level.
        assert!(
            cache
                .storage()
                .load("same-region-companion")
                .await
                .unwrap()
                .is_miss()
        );
    }
    cache.close().await.unwrap();
}

#[tokio::test]
async fn payload_corruption_is_rejected_without_losing_another_region() {
    tokio::time::timeout(
        Duration::from_secs(20),
        corrupt_and_recover(Damage::Payload),
    )
    .await
    .expect("payload corruption qualification deadline");
}

#[tokio::test]
async fn index_corruption_is_rejected_without_losing_another_region() {
    tokio::time::timeout(Duration::from_secs(20), corrupt_and_recover(Damage::Index))
        .await
        .expect("index corruption qualification deadline");
}
