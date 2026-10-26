#![forbid(unsafe_code)]
mod support;
use geesefs_rs::{error::Error, filesystem::Filesystem, parts::PartSchedule};
use std::sync::atomic::Ordering;
use support::MemoryStore;
const MIB: u64 = 1024 * 1024;

#[test]
fn tier_boundaries_counts_and_invalid_input_follow_reference_configuration() {
    let parts: PartSchedule = "5:1000,25:1000,125".parse().unwrap();
    assert_eq!(parts.max_parts(), 10000);
    assert_eq!(parts.max_size(), 1030000 * MIB);
    assert_eq!(parts.range(999).unwrap(), 4995 * MIB..5000 * MIB);
    assert_eq!(parts.range(1000).unwrap(), 5000 * MIB..5025 * MIB);
    assert_eq!(
        parts.at_offset(30000 * MIB).unwrap(),
        (2000, 30000 * MIB..30125 * MIB)
    );
    assert!(parts.range(10000).is_none());
    assert!(parts.at_offset(parts.max_size()).is_none());
    for invalid in [
        "",
        "4",
        "5121",
        "5,25",
        "5:10001",
        "5:9999,25:2",
        "5:1:2",
        "5:0,25",
    ] {
        assert!(invalid.parse::<PartSchedule>().is_err(), "{invalid}");
    }
    assert_eq!(
        "5:2,7:0".parse::<PartSchedule>().unwrap().max_parts(),
        10000
    );
}

#[tokio::test]
async fn configured_single_put_and_larger_parts_stream_inside_small_flush_budget() {
    for single in [true, false] {
        let store = MemoryStore::new();
        let mut config = MemoryStore::config(16);
        config.single_part = if single { 16 } else { 5 };
        config.part_sizes = "5:1,7:1,11".parse().unwrap();
        let fs = Filesystem::new(config, store.clone()).unwrap();
        let node = fs
            .create(1, "file", libc::S_IFREG | 0o644, None, 0)
            .await
            .unwrap();
        let size = if single { 12 * MIB } else { 20 * MIB };
        fs.truncate(node.ino, size).unwrap();
        fs.write(node.ino, size - 3, b"end").await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), fs.fsync(node.ino))
            .await
            .unwrap()
            .unwrap();
        let bytes = store.bytes("file");
        assert_eq!(bytes.len() as u64, size);
        assert!(bytes[..bytes.len() - 3].iter().all(|b| *b == 0));
        assert_eq!(&bytes[bytes.len() - 3..], b"end");
        assert!(store.max_upload_frame.load(Ordering::Relaxed) <= MIB as usize);
        if single {
            assert_eq!(store.put_calls.load(Ordering::Relaxed), 1);
            assert!(store.upload_sizes.lock().unwrap().is_empty());
        } else {
            store.upload_part_sizes.lock().unwrap().sort_unstable();
            assert_eq!(
                *store.upload_part_sizes.lock().unwrap(),
                [(1, 5 * MIB), (2, 7 * MIB), (3, 8 * MIB)]
            );
        }
    }
}

#[tokio::test]
async fn sixty_gib_object_uses_configured_tiers_without_materializing_the_object() {
    let store = MemoryStore::new();
    store.seed("virtual", b"");
    store.virtual_size.store(60 * 1024 * MIB, Ordering::Relaxed);
    let fs = store.filesystem(16);
    let node = fs.lookup(1, "virtual").await.unwrap();
    fs.write(node.ino, 0, b"X").await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), fs.fsync(node.ino))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(store.copied_parts.load(Ordering::Relaxed), 2251);
    let mut ranges = store.copied_ranges.lock().unwrap();
    ranges.sort_unstable_by_key(|range| *range.start());
    assert_eq!(ranges[999], 5000 * MIB..=5025 * MIB - 1);
    assert_eq!(ranges[1999], 30000 * MIB..=30125 * MIB - 1);
    assert_eq!(ranges.last().unwrap(), &(61375 * MIB..=61440 * MIB - 1));
    assert_eq!(*store.upload_part_sizes.lock().unwrap(), [(1, 5 * MIB)]);
    assert_eq!(store.bytes("virtual")[0], b'X');
}

#[tokio::test]
async fn configured_object_end_is_checked_before_any_write_or_truncate_side_effect() {
    let store = MemoryStore::new();
    let mut config = MemoryStore::config(16);
    config.part_sizes = "5:1".parse().unwrap();
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let node = fs
        .create(1, "file", libc::S_IFREG | 0o644, None, 0)
        .await
        .unwrap();
    assert!(matches!(
        fs.write(node.ino, 5 * MIB, b"x").await,
        Err(Error::Fs(libc::EFBIG))
    ));
    assert!(matches!(
        fs.write(node.ino, u64::MAX, b"x").await,
        Err(Error::Fs(libc::EFBIG))
    ));
    assert!(matches!(
        fs.truncate(node.ino, 5 * MIB + 1),
        Err(Error::Fs(libc::EFBIG))
    ));
    fs.fsync(node.ino).await.unwrap();
    assert_eq!(store.bytes("file"), b"");
}

#[tokio::test]
async fn multipart_parallelism_is_effective_bounded_and_completion_is_ordered() {
    let store = MemoryStore::new();
    let gate = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
    *store.part_gate.lock().unwrap() = Some(gate.clone());
    let mut config = MemoryStore::config(16);
    config.max_parallel_parts = 2;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let node = fs
        .create(1, "file", libc::S_IFREG | 0o644, None, 0)
        .await
        .unwrap();
    fs.truncate(node.ino, 15 * MIB + 1).unwrap();
    fs.write(node.ino, 15 * MIB, b"X").await.unwrap();
    let worker = fs.clone();
    let ino = node.ino;
    let task = tokio::spawn(async move { worker.fsync(ino).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while store.part_starts.load(Ordering::Relaxed) < 2 {
            store.part_started.notified().await;
        }
    })
    .await
    .expect("two allowed parts must enter concurrently");
    assert_eq!(store.part_starts.load(Ordering::Relaxed), 2);
    gate.add_permits(4);
    tokio::time::timeout(std::time::Duration::from_secs(10), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let bytes = store.bytes("file");
    assert_eq!(bytes.len() as u64, 15 * MIB + 1);
    assert_eq!(bytes.last(), Some(&b'X'));
    assert!(bytes[..bytes.len() - 1].iter().all(|b| *b == 0));
}
