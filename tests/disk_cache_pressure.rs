#![forbid(unsafe_code)]
mod support;

use geesefs_rs::{
    config::Config, disk_cache::PreparedDiskCache, filesystem::Filesystem, read::READ_BLOCK_BYTES,
};
use std::{os::unix::fs::PermissionsExt, sync::atomic::Ordering, time::Duration};
use support::MemoryStore;

const MIB: usize = 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(10);

async fn attach(config: Config, store: std::sync::Arc<MemoryStore>) -> std::sync::Arc<Filesystem> {
    let disk = config.disk_cache_config().unwrap();
    let prepared = disk.map(|disk| {
        std::fs::set_permissions(&disk.directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        PreparedDiskCache::prepare(&disk.directory, disk.max_bytes).unwrap()
    });
    let fs = Filesystem::new(config, store).unwrap();
    if let Some(mut prepared) = prepared {
        let disk = fs.config.disk_cache_config().unwrap().unwrap();
        if !disk.directory.join("payload-v1.cache").exists() {
            // R1 exercises Ready demotion-before-loan, not initial startup.
            let resources = geesefs_rs::budget::Budget::new(32 * MIB).unwrap();
            let cache = geesefs_rs::disk_cache::DiskCache::open(prepared, &resources)
                .await
                .unwrap();
            cache.close(DEADLINE).await.unwrap();
            drop(cache);
            prepared = PreparedDiskCache::prepare(&disk.directory, disk.max_bytes).unwrap();
        }
        fs.attach_disk_cache(prepared).await.unwrap();
    }
    fs
}

async fn byte_pressure(enabled: bool, selected: bool, window: usize) {
    let dir = tempfile::tempdir().unwrap();
    let mount = tempfile::tempdir().unwrap();
    let store = MemoryStore::new();
    *store.cache_namespace.lock().unwrap() = Some([83; 32]);
    let key = if selected {
        "payload.rpm"
    } else {
        "payload.txt"
    };
    let mut bytes = vec![0; 48 * MIB];
    for (index, block) in bytes.chunks_mut(READ_BLOCK_BYTES).enumerate() {
        block.fill(index as u8);
    }
    store.seed(key, &bytes);
    let mut config = MemoryStore::config(32);
    config.mountpoint = mount.path().into();
    // Entry-count eviction must not mask the byte-admission path. A logical-
    // block window isolates that path from optional JOBS=4 saturation; a larger
    // MemoryStore backing also exercises bounded multi-fragment demotion.
    config.entry_limit = 100_000;
    config.read_ahead = window / 1024;
    config.read_ahead_small = window / 1024;
    config.read_ahead_large = window / 1024;
    config.read_ahead_parallel = window / 1024;
    if enabled {
        config.disk_cache_dir = Some(dir.path().into());
        config.disk_cache_max_size = Some(128 * MIB as u64);
        config.disk_cache_regex = Some(r"\.rpm$".parse().unwrap());
    }
    let fs = attach(config.clone(), store.clone()).await;
    let node = fs.lookup(1, key).await.unwrap();
    for offset in (0..bytes.len()).step_by(READ_BLOCK_BYTES) {
        let block =
            tokio::time::timeout(DEADLINE, fs.read(node.ino, offset as u64, READ_BLOCK_BYTES))
                .await
                .unwrap()
                .unwrap();
        assert_eq!(block.as_slice(), &bytes[offset..offset + READ_BLOCK_BYTES]);
        drop(block);
        // Deterministic completion barrier, not a sleep or a required-read wait
        // added to production. Deliberately permit every optional copy to finish.
        fs.drain_disk_cache(DEADLINE).await.unwrap();
        let budgets = fs.effective_budgets();
        assert!(
            budgets.data_used_bytes + budgets.ingress_used_bytes + budgets.flush_used_bytes
                <= 32 * MIB
        );
        if offset == 0 && enabled {
            assert_eq!(fs.disk_cache_stats().unwrap().admitted_demotions, 0);
        }
    }
    assert_eq!(
        store.read_calls.load(Ordering::Relaxed),
        bytes.len() / window
    );
    let budgets = fs.effective_budgets();
    eprintln!("enabled={enabled} selected={selected} window={window} budgets={budgets:?}");
    if enabled {
        let stats = fs.disk_cache_stats().unwrap();
        eprintln!("after byte pressure: {stats:?}");
        assert_eq!(stats.errors, 0);
        assert_eq!(stats.active_jobs, 0);
        assert_eq!(stats.copied_bytes, 0);
        if selected {
            assert!(
                stats.admitted_demotions > 0,
                "eligible byte-pressure victims must reach L2"
            );
            assert!(stats.index_entries > 0);
        } else {
            assert_eq!(stats.admitted_demotions, 0);
            assert_eq!(stats.index_entries, 0);
            assert!(
                budgets.flush_used_bytes > 0,
                "excluded data still uses idle loans"
            );
        }
    } else {
        assert!(fs.disk_cache_stats().is_none());
        assert_eq!(budgets.disk_cache_funding_bytes, 0);
        assert!(
            budgets.flush_used_bytes > 0,
            "disabled L2 must preserve idle loans"
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
    if enabled && !selected {
        let demotions = fs.disk_cache_stats().unwrap().admitted_demotions;
        let writer = fs
            .create(1, "writer.txt", libc::S_IFREG | 0o644, None, 0)
            .await
            .unwrap();
        fs.write(writer.ino, 0, b"committed").await.unwrap();
        tokio::time::timeout(DEADLINE, fs.fsync(writer.ino))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(store.bytes("writer.txt"), b"committed");
        assert_eq!(fs.effective_budgets().read_cache_loaned_bytes, 0);
        assert_eq!(
            fs.disk_cache_stats().unwrap().admitted_demotions,
            demotions,
            "writer loan revocation must not populate L2"
        );
    }
    fs.shutdown().await.unwrap();
    drop(fs);

    // A fresh L1 excludes hits on retained RAM or a serving read view. The first
    // block must have survived as an immutable persistent record, not just a
    // demotion submission counter.
    config.read_ahead = READ_BLOCK_BYTES / 1024;
    config.read_ahead_small = READ_BLOCK_BYTES / 1024;
    config.read_ahead_large = READ_BLOCK_BYTES / 1024;
    config.read_ahead_parallel = READ_BLOCK_BYTES / 1024;
    let fs = attach(config, store.clone()).await;
    let node = fs.lookup(1, key).await.unwrap();
    let gets = store.read_calls.load(Ordering::Relaxed);
    let block = tokio::time::timeout(DEADLINE, fs.read(node.ino, 0, READ_BLOCK_BYTES))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(block.as_slice(), &bytes[..READ_BLOCK_BYTES]);
    drop(block);
    assert_eq!(
        store.read_calls.load(Ordering::Relaxed) - gets,
        usize::from(!(enabled && selected)),
        "only a selected persistent victim can satisfy the restarted reader without GET"
    );
    if enabled && selected {
        assert!(fs.disk_cache_stats().unwrap().hits > 0);
        assert_eq!(fs.disk_cache_read_bytes(), READ_BLOCK_BYTES as u64);
        eprintln!("recovered probe: {:?}", fs.disk_cache_stats().unwrap());
        if window > READ_BLOCK_BYTES {
            // This offset lies beyond the four-fragment submission prefix. A
            // best-effort omission is allowed, but must fall back to exact bytes
            // without blocking required reads or retaining the evicted backing.
            let offset = 4 * READ_BLOCK_BYTES;
            let gets = store.read_calls.load(Ordering::Relaxed);
            let block =
                tokio::time::timeout(DEADLINE, fs.read(node.ino, offset as u64, READ_BLOCK_BYTES))
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(block.as_slice(), &bytes[offset..offset + READ_BLOCK_BYTES]);
            drop(block);
            eprintln!(
                "beyond submission prefix: source_gets={} stats={:?}",
                store.read_calls.load(Ordering::Relaxed) - gets,
                fs.disk_cache_stats().unwrap()
            );
        }
    }
    fs.shutdown().await.unwrap();
}

#[tokio::test]
async fn selected_byte_pressure_demotes_before_idle_loans_hide_capacity_victims() {
    byte_pressure(true, true, READ_BLOCK_BYTES).await;
}

#[tokio::test]
async fn excluded_and_disabled_byte_pressure_preserves_idle_loans() {
    byte_pressure(true, false, READ_BLOCK_BYTES).await;
    byte_pressure(false, true, READ_BLOCK_BYTES).await;
}

#[tokio::test]
async fn large_window_byte_pressure_preserves_correct_reads_with_bounded_optional_spills() {
    byte_pressure(true, true, 2 * MIB).await;
}

#[tokio::test]
async fn repeated_pressure_advances_beyond_already_indexed_fragment_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let mount = tempfile::tempdir().unwrap();
    let store = MemoryStore::new();
    *store.cache_namespace.lock().unwrap() = Some([84; 32]);
    let mut bytes = vec![0x17; 2 * MIB];
    bytes[MIB..].fill(0xa2);
    store.seed("package.rpm", &bytes);
    store.seed("pressure.txt", &vec![0x71; 48 * MIB]);
    let mut config = MemoryStore::config(32);
    config.mountpoint = mount.path().into();
    config.entry_limit = 100_000;
    config.disk_cache_dir = Some(dir.path().into());
    config.disk_cache_max_size = Some(128 * MIB as u64);
    config.disk_cache_regex = Some(r"\.rpm$".parse().unwrap());
    config.read_ahead = 2048;
    config.read_ahead_small = 2048;
    config.read_ahead_large = 2048;
    config.read_ahead_parallel = 2048;

    // Match the native qualification's fixed eight population mounts, not a
    // retry-until-hot. JOBS=4 bounds submissions, not durable records per round.
    // Existing prefixes must not permanently starve the remaining offsets.
    for round in 0..8 {
        let fs = attach(config.clone(), store.clone()).await;
        let package = fs.lookup(1, "package.rpm").await.unwrap();
        for offset in (0..bytes.len()).step_by(READ_BLOCK_BYTES) {
            let block = tokio::time::timeout(
                DEADLINE,
                fs.read(package.ino, offset as u64, READ_BLOCK_BYTES),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(block.as_slice(), &bytes[offset..offset + READ_BLOCK_BYTES]);
        }
        let pressure = fs.lookup(1, "pressure.txt").await.unwrap();
        for offset in (0..48 * MIB).step_by(2 * MIB) {
            let block = tokio::time::timeout(
                DEADLINE,
                fs.read(pressure.ino, offset as u64, READ_BLOCK_BYTES),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(block.as_slice(), &[0x71; READ_BLOCK_BYTES]);
            drop(block);
            fs.drain_disk_cache(DEADLINE).await.unwrap();
        }
        let stats = fs.disk_cache_stats().unwrap();
        eprintln!("population round={round}: {stats:?}");
        assert_eq!(stats.errors, 0);
        assert_eq!(stats.active_jobs, 0);
        assert_eq!(stats.copied_bytes, 0);
        fs.shutdown().await.unwrap();
        drop(fs);
    }
    let fs = attach(config, store.clone()).await;
    let package = fs.lookup(1, "package.rpm").await.unwrap();
    let gets = store.read_calls.load(Ordering::Relaxed);
    for offset in (0..bytes.len()).step_by(READ_BLOCK_BYTES) {
        let block = tokio::time::timeout(
            DEADLINE,
            fs.read(package.ino, offset as u64, READ_BLOCK_BYTES),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(block.as_slice(), &bytes[offset..offset + READ_BLOCK_BYTES]);
    }
    assert_eq!(store.read_calls.load(Ordering::Relaxed), gets);
    assert_eq!(fs.disk_cache_read_bytes(), bytes.len() as u64);
    fs.shutdown().await.unwrap();
}
