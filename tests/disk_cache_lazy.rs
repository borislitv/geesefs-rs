#![forbid(unsafe_code)]
mod support;

use geesefs_rs::{
    disk_cache::{PreparedDiskCache, State},
    filesystem::Filesystem,
    read::READ_BLOCK_BYTES,
};
use std::{os::unix::fs::PermissionsExt, time::Duration};
use support::MemoryStore;

const MIB: usize = 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(10);

struct Fixture {
    dir: tempfile::TempDir,
    _mount: tempfile::TempDir,
    store: std::sync::Arc<MemoryStore>,
    config: geesefs_rs::config::Config,
}

impl Fixture {
    fn new(entry_limit: u64) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mount = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let store = MemoryStore::new();
        *store.cache_namespace.lock().unwrap() = Some([94; 32]);
        store.seed("selected.rpm", &vec![0x53; 40 * MIB]);
        let mut config = MemoryStore::config(32);
        config.mountpoint = mount.path().into();
        config.disk_cache_dir = Some(dir.path().into());
        config.disk_cache_max_size = Some(16 * MIB as u64);
        config.disk_cache_regex = Some(r"\.rpm$".parse().unwrap());
        config.entry_limit = entry_limit;
        config.read_ahead = 128;
        config.read_ahead_small = 128;
        config.read_ahead_large = 128;
        config.read_ahead_parallel = 128;
        Self {
            dir,
            _mount: mount,
            store,
            config,
        }
    }
    fn prepare(&self) -> PreparedDiskCache {
        PreparedDiskCache::prepare(self.dir.path(), 16 * MIB as u64).unwrap()
    }
    async fn attach(&self) -> std::sync::Arc<Filesystem> {
        let fs = Filesystem::new(self.config.clone(), self.store.clone()).unwrap();
        fs.attach_disk_cache(self.prepare()).await.unwrap();
        fs
    }
}

async fn read(fs: &Filesystem, ino: u64, index: usize) {
    let block = tokio::time::timeout(
        DEADLINE,
        fs.read(ino, (index * READ_BLOCK_BYTES) as u64, READ_BLOCK_BYTES),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(block.as_slice(), &[0x53; READ_BLOCK_BYTES]);
}

#[tokio::test]
async fn fresh_attachment_reserves_funding_without_constructing_an_unused_engine() {
    let dir = tempfile::tempdir().unwrap();
    let mount = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let store = MemoryStore::new();
    *store.cache_namespace.lock().unwrap() = Some([93; 32]);
    store.seed("selected.rpm", &vec![0x53; MIB]);
    let mut config = MemoryStore::config(32);
    config.mountpoint = mount.path().into();
    config.disk_cache_dir = Some(dir.path().into());
    config.disk_cache_max_size = Some(16 * MIB as u64);
    config.disk_cache_regex = Some(r"\.rpm$".parse().unwrap());
    config.read_ahead = 128;
    config.read_ahead_small = 128;
    config.read_ahead_large = 128;
    let prepared = PreparedDiskCache::prepare(dir.path(), 16 * MIB as u64).unwrap();
    let status = prepared.status();
    let fs = Filesystem::new(config, store.clone()).unwrap();
    fs.attach_disk_cache(prepared).await.unwrap();
    let budgets = fs.effective_budgets();
    eprintln!(
        "attachment state={:?} stats={:?} budgets={budgets:?}",
        fs.disk_cache_state(),
        fs.disk_cache_stats()
    );
    eprintln!(
        "layout bytes: Filesystem={} Prepared={} FundedPrepared={}",
        std::mem::size_of::<Filesystem>(),
        std::mem::size_of::<PreparedDiskCache>(),
        std::mem::size_of::<geesefs_rs::disk_cache::FundedPreparedDiskCache>()
    );
    assert_eq!(budgets.data_used_bytes, budgets.disk_cache_funding_bytes);
    assert!(budgets.disk_cache_funding_bytes > 0);
    assert!(
        !dir.path().join("payload-v1.cache").exists(),
        "unused fresh L2 must not preallocate media or build engine/index/pools"
    );
    let node = fs.lookup(1, "selected.rpm").await.unwrap();
    for _ in 0..2 {
        for offset in (0..MIB).step_by(READ_BLOCK_BYTES) {
            let block =
                tokio::time::timeout(DEADLINE, fs.read(node.ino, offset as u64, READ_BLOCK_BYTES))
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(block.as_slice(), &[0x53; READ_BLOCK_BYTES]);
        }
    }
    assert_eq!(status.state(), State::Prepared);
    assert!(fs.disk_cache_stats().is_none());
    let metrics = geesefs_rs::metrics::render(&fs).unwrap();
    assert!(metrics.contains("geesefs_disk_cache_state{state=\"prepared\"} 1"));
    assert!(!metrics.contains("geesefs_disk_cache_region_used_bytes "));
    assert!(metrics.contains("geesefs_disk_cache_stats_available 0"));
    assert!(
        !metrics
            .lines()
            .any(|line| line.starts_with("geesefs_disk_cache_io_read_bytes_total "))
    );
    assert_eq!(status.startup().startup.count, 0);
    assert_eq!(status.opened_file_allocated_bytes(), None);
    assert!(!dir.path().join("payload-v1.cache").exists());
    assert!(PreparedDiskCache::prepare(dir.path(), 16 * MIB as u64).is_err());
    fs.shutdown().await.unwrap();
    assert_eq!(status.state(), State::Closed);
    assert_eq!(fs.effective_budgets().data_used_bytes, MIB);
    assert!(PreparedDiskCache::prepare(dir.path(), 16 * MIB as u64).is_ok());
}

#[tokio::test]
async fn byte_pressure_starts_a_funded_empty_tier_without_loan_first_starvation() {
    let fixture = Fixture::new(100_000);
    let fs = fixture.attach().await;
    let funding = fs.effective_budgets().disk_cache_funding_bytes;
    let ino = fs.lookup(1, "selected.rpm").await.unwrap().ino;
    for index in 0..40 * MIB / READ_BLOCK_BYTES {
        read(&fs, ino, index).await;
        fs.drain_disk_cache(DEADLINE).await.unwrap();
        let budget = fs.effective_budgets();
        assert_eq!(budget.disk_cache_funding_bytes, funding);
        assert!(
            budget.data_used_bytes + budget.ingress_used_bytes + budget.flush_used_bytes
                <= 32 * MIB
        );
    }
    let stats = fs
        .disk_cache_stats()
        .expect("eligible byte pressure must start L2 before loans hide all victims");
    assert_eq!(stats.state, State::Ready);
    assert_eq!(stats.startup.startup.count, 1);
    assert_eq!(stats.startup.allocation.count, 1);
    assert!(stats.admitted_demotions > 0);
    assert_eq!(stats.errors, 0);
    fs.shutdown().await.unwrap();
}

#[tokio::test]
async fn dropping_an_unused_deferred_filesystem_releases_its_preparation_without_io() {
    let fixture = Fixture::new(2);
    let prepared = fixture.prepare();
    let status = prepared.status();
    let fs = Filesystem::new(fixture.config.clone(), fixture.store.clone()).unwrap();
    fs.attach_disk_cache(prepared).await.unwrap();
    assert_eq!(status.state(), State::Prepared);
    drop(fs);
    assert_eq!(status.state(), State::Closed);
    assert_eq!(status.startup().startup.count, 0);
    assert!(!fixture.dir.path().join("payload-v1.cache").exists());
    assert!(PreparedDiskCache::prepare(fixture.dir.path(), 16 * MIB as u64).is_ok());
}

#[tokio::test]
async fn existing_inventory_recovers_before_any_new_capacity_victim() {
    let fixture = Fixture::new(2);
    let fs = fixture.attach().await;
    let ino = fs.lookup(1, "selected.rpm").await.unwrap().ino;
    for index in 0..3 {
        read(&fs, ino, index).await;
    }
    fs.drain_disk_cache(DEADLINE).await.unwrap();
    read(&fs, ino, 3).await;
    fs.drain_disk_cache(DEADLINE).await.unwrap();
    assert!(fs.disk_cache_stats().unwrap().index_entries > 0);
    fs.shutdown().await.unwrap();
    drop(fs);
    let fs = fixture.attach().await;
    assert_eq!(fs.disk_cache_state(), Some(State::Ready));
    let ino = fs.lookup(1, "selected.rpm").await.unwrap().ino;
    let gets = fixture
        .store
        .read_calls
        .load(std::sync::atomic::Ordering::Relaxed);
    read(&fs, ino, 1).await;
    assert_eq!(
        fixture
            .store
            .read_calls
            .load(std::sync::atomic::Ordering::Relaxed),
        gets
    );
    assert!(fs.disk_cache_stats().unwrap().hits > 0);
    fs.shutdown().await.unwrap();
}

#[tokio::test]
async fn preattachment_selected_loans_survive_deferred_start_then_revoke_without_demotion() {
    let fixture = Fixture::new(100_000);
    let fs = Filesystem::new(fixture.config.clone(), fixture.store.clone()).unwrap();
    let ino = fs.lookup(1, "selected.rpm").await.unwrap().ino;
    for index in 0..4 * MIB / READ_BLOCK_BYTES {
        read(&fs, ino, index).await;
    }
    let holder = fs
        .create(1, "holder", libc::S_IFREG | 0o644, None, 0)
        .await
        .unwrap();
    let bytes = vec![0x31; MIB];
    for index in 0..17 {
        tokio::time::timeout(DEADLINE, fs.write(holder.ino, (index * MIB) as u64, &bytes))
            .await
            .unwrap()
            .unwrap();
    }
    let loans = fs.effective_budgets().read_cache_loaned_bytes;
    assert!(loans > 0);
    fs.truncate(holder.ino, 0).unwrap();
    fs.attach_disk_cache(fixture.prepare()).await.unwrap();
    assert_eq!(fs.disk_cache_state(), Some(State::Prepared));
    assert_eq!(fs.effective_budgets().read_cache_loaned_bytes, loans);
    for index in 0..14 {
        tokio::time::timeout(DEADLINE, fs.write(holder.ino, (index * MIB) as u64, &bytes))
            .await
            .unwrap()
            .unwrap();
    }
    fs.drain_disk_cache(DEADLINE).await.unwrap();
    assert_eq!(fs.disk_cache_state(), Some(State::Ready));
    assert!(fs.effective_budgets().read_cache_loaned_bytes >= loans);
    assert_eq!(fs.disk_cache_stats().unwrap().admitted_demotions, 0);
    tokio::time::timeout(DEADLINE, fs.fsync(holder.ino))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fixture.store.bytes("holder"), vec![0x31; 14 * MIB]);
    assert_eq!(fs.effective_budgets().read_cache_loaned_bytes, 0);
    let stats = fs.disk_cache_stats().unwrap();
    assert_eq!(stats.admitted_demotions, 0);
    assert_eq!(stats.index_entries, 0);
    assert_eq!(stats.drops, 0);
    fs.shutdown().await.unwrap();
}

#[test]
fn startup_is_singleflight_owned_and_does_not_block_required_reads_or_early_close() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let fixture = Fixture::new(2);
        let prepared = fixture.prepare();
        let status = prepared.status();
        let fs = Filesystem::new(fixture.config.clone(), fixture.store.clone()).unwrap();
        fs.attach_disk_cache(prepared).await.unwrap();
        let ino = fs.lookup(1, "selected.rpm").await.unwrap().ino;
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            entered_tx.send(()).unwrap();
            release_rx.recv_timeout(DEADLINE).unwrap();
        });
        entered_rx.await.unwrap();
        for index in 0..6 {
            read(&fs, ino, index).await;
        }
        tokio::time::timeout(DEADLINE, async {
            while status.state() == State::Prepared {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(status.state(), State::Recovering);
        assert!(!fixture.dir.path().join("payload-v1.cache").exists());
        assert_eq!(status.startup().allocation.count, 0);
        assert!(PreparedDiskCache::prepare(fixture.dir.path(), 16 * MIB as u64).is_err());
        let funding = fs.effective_budgets().disk_cache_funding_bytes;
        assert_eq!(
            fs.effective_budgets().data_used_bytes,
            funding + 2 * READ_BLOCK_BYTES
        );
        let shutdown = tokio::spawn({
            let fs = fs.clone();
            async move { fs.shutdown().await }
        });
        tokio::time::timeout(DEADLINE, async {
            while !fs.is_closing() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(
            !shutdown.is_finished(),
            "real startup I/O still owns shutdown work"
        );
        shutdown.abort();
        assert!(shutdown.await.unwrap_err().is_cancelled());
        assert!(PreparedDiskCache::prepare(fixture.dir.path(), 16 * MIB as u64).is_err());
        release_tx.send(()).unwrap();
        blocker.await.unwrap();
        tokio::time::timeout(DEADLINE, async {
            while status.state() != State::Closed {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        fs.drain_disk_cache(DEADLINE).await.unwrap();
        assert_eq!(status.startup().startup.count, 1);
        assert_eq!(status.startup().allocation.count, 1);
        assert_eq!(fs.effective_budgets().data_used_bytes, 2 * READ_BLOCK_BYTES);
        assert!(PreparedDiskCache::prepare(fixture.dir.path(), 16 * MIB as u64).is_ok());
    });
}

#[tokio::test]
async fn deferred_owner_refuses_an_introduced_file_and_existing_owner_never_recreates_unlinked_media()
 {
    let fixture = Fixture::new(2);
    let fs = fixture.attach().await;
    let path = fixture.dir.path().join("payload-v1.cache");
    std::fs::write(&path, b"foreign sentinel").unwrap();
    let ino = fs.lookup(1, "selected.rpm").await.unwrap().ino;
    for index in 0..3 {
        read(&fs, ino, index).await;
    }
    assert!(fs.drain_disk_cache(DEADLINE).await.is_err());
    assert_eq!(fs.disk_cache_state(), Some(State::Bypass));
    assert_eq!(std::fs::read(&path).unwrap(), b"foreign sentinel");
    read(&fs, ino, 3).await;
    fs.shutdown().await.unwrap();
    drop(fs);
    std::fs::remove_file(&path).unwrap();
    let resources = geesefs_rs::budget::Budget::new(32 * MIB).unwrap();
    let cache = geesefs_rs::disk_cache::DiskCache::open(fixture.prepare(), &resources)
        .await
        .unwrap();
    cache.close(DEADLINE).await.unwrap();
    drop(cache);
    let prepared = fixture.prepare();
    std::fs::remove_file(&path).unwrap();
    assert!(
        geesefs_rs::disk_cache::DiskCache::open(prepared, &resources)
            .await
            .is_err()
    );
    assert!(
        !path.exists(),
        "existing media must never be silently recreated"
    );
    assert_eq!(resources.used(), 0);
    assert!(PreparedDiskCache::prepare(fixture.dir.path(), 16 * MIB as u64).is_ok());
}
