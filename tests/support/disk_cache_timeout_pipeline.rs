// Compiled as a library unit-test module to keep engine injection cfg(test)-only.
#![cfg(test)]
#![forbid(unsafe_code)]

use crate::{
    backend::{Store, cache_scope::CacheScope, extended_list::ListPage, s3::ObjectHead},
    budget::{Block, Budget},
    config::Config,
    disk_cache::{Demotion, DiskCache, DiskKey, PreparedDiskCache, ReadFallback, State},
    error::Result,
    filesystem::Filesystem,
    read::READ_BLOCK_BYTES as BLOCK,
};
use aws_sdk_s3::{primitives::ByteStream, types::CompletedPart};
use clap::Parser;
use foyer::{IoEngine, IoEngineConfig};
use std::{
    collections::HashMap,
    os::unix::fs::PermissionsExt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

const MIB: usize = 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(10);
const NAMESPACE: [u8; 32] = [112; 32];

struct Source {
    bytes: Vec<u8>,
    ranges: Mutex<Vec<(u64, usize)>>,
    started: tokio::sync::Notify,
    release: tokio::sync::Semaphore,
}

#[async_trait::async_trait]
impl Store for Source {
    fn persistent_cache_scope(&self) -> Result<CacheScope> {
        Ok(CacheScope::Namespace(NAMESPACE))
    }
    async fn head(&self, _key: &str) -> Result<ObjectHead> {
        Ok(ObjectHead {
            size: self.bytes.len() as u64,
            etag: "generation-one".into(),
            metadata: HashMap::new(),
            ..Default::default()
        })
    }
    async fn read_range(
        &self,
        _key: &str,
        offset: u64,
        length: usize,
        _head: &ObjectHead,
        budget: &Budget,
    ) -> Result<Block> {
        self.ranges.lock().unwrap().push((offset, length));
        self.started.notify_one();
        self.release.acquire().await.unwrap().forget();
        budget
            .try_reserve(length)
            .unwrap()
            .copy(&self.bytes[offset as usize..offset as usize + length])
    }
    async fn put(&self, _: &str, _: ByteStream, _: HashMap<String, String>) -> Result<String> {
        unreachable!("read-only fixture")
    }
    async fn begin_upload(&self, _: &str, _: HashMap<String, String>, _: u64) -> Result<String> {
        unreachable!("read-only fixture")
    }
    async fn replace_metadata(
        &self,
        _: &str,
        _: &str,
        _: HashMap<String, String>,
    ) -> Result<ObjectHead> {
        unreachable!("read-only fixture")
    }
    async fn upload_part(&self, _: &str, _: &str, _: i32, _: ByteStream) -> Result<CompletedPart> {
        unreachable!("read-only fixture")
    }
    async fn copy_part(
        &self,
        _: &str,
        _: &str,
        _: i32,
        _: &str,
        _: std::ops::RangeInclusive<u64>,
        _: &str,
    ) -> Result<CompletedPart> {
        unreachable!("read-only fixture")
    }
    async fn complete_upload(&self, _: &str, _: &str, _: Vec<CompletedPart>) -> Result<String> {
        unreachable!("read-only fixture")
    }
    async fn abort_upload(&self, _: &str, _: &str) -> Result<()> {
        unreachable!("read-only fixture")
    }
    async fn copy(&self, _: &str, _: &str, _: &str) -> Result<String> {
        unreachable!("read-only fixture")
    }
    async fn delete(&self, _: &str) -> Result<()> {
        unreachable!("read-only fixture")
    }
    async fn list(&self, _: &str, _: Option<&str>, _: Option<&str>) -> Result<ListPage> {
        unreachable!("single-file fixture")
    }
}

#[derive(Debug)]
struct Gate {
    held: AtomicBool,
    entered: tokio::sync::Semaphore,
    release: tokio::sync::Semaphore,
}
#[derive(Debug)]
struct GatedConfig(Arc<Gate>);
#[derive(Debug)]
struct GatedEngine {
    real: Arc<dyn IoEngine>,
    gate: Arc<Gate>,
}
impl IoEngineConfig for GatedConfig {
    fn build(
        self: Box<Self>,
        context: foyer::IoEngineBuildContext,
    ) -> futures_util::future::BoxFuture<'static, foyer::Result<Arc<dyn IoEngine>>> {
        Box::pin(async move {
            let real = Box::new(foyer::PsyncIoEngineConfig::new())
                .build(context)
                .await?;
            Ok(Arc::new(GatedEngine { real, gate: self.0 }) as Arc<dyn IoEngine>)
        })
    }
}
impl IoEngine for GatedEngine {
    fn read(
        &self,
        buffer: foyer_storage::IoReadBuffer,
        partition: &dyn foyer_storage::Partition,
        offset: u64,
    ) -> foyer::IoHandle {
        let operation = self.real.read(buffer, partition, offset);
        let held = self.gate.held.load(Ordering::SeqCst);
        let gate = self.gate.clone();
        foyer::IoHandle::from(Box::pin(async move {
            if held {
                gate.entered.add_permits(1);
                gate.release.acquire().await.unwrap().forget();
            }
            operation.await
        }) as futures_util::future::BoxFuture<'static, _>)
    }
    fn write(
        &self,
        buffer: foyer_storage::IoWriteBuffer,
        partition: &dyn foyer_storage::Partition,
        offset: u64,
    ) -> foyer::IoHandle {
        self.real.write(buffer, partition, offset)
    }
}

#[tokio::test]
async fn qualified_range_stops_disk_waits_after_timeout_and_keeps_actual_worker_owned() {
    let directory = tempfile::tempdir().unwrap();
    let mount = tempfile::tempdir().unwrap();
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let bytes: Vec<_> = (1..=32u8).flat_map(|byte| vec![byte; BLOCK]).collect();
    let source = Arc::new(Source {
        bytes,
        ranges: Mutex::new(Vec::new()),
        started: tokio::sync::Notify::new(),
        release: tokio::sync::Semaphore::new(0),
    });
    let cache = DiskCache::open(
        PreparedDiskCache::prepare(directory.path(), (16 * MIB) as u64).unwrap(),
        &Budget::new(32 * MIB).unwrap(),
    )
    .await
    .unwrap();
    for index in 0..32 {
        let key = DiskKey::new(
            &NAMESPACE,
            b"slow.rpm",
            b"generation-one",
            source.bytes.len() as u64,
            (index * BLOCK) as u64,
            BLOCK,
        )
        .unwrap();
        let block = Budget::new(BLOCK)
            .unwrap()
            .try_reserve(BLOCK)
            .unwrap()
            .copy(&source.bytes[index * BLOCK..(index + 1) * BLOCK])
            .unwrap();
        assert!(matches!(cache.try_demote(key, block), Demotion::Admitted));
        cache.drain(DEADLINE).await.unwrap();
    }
    cache.close(DEADLINE).await.unwrap();
    drop(cache);
    let gate = Arc::new(Gate {
        held: AtomicBool::new(false),
        entered: tokio::sync::Semaphore::new(0),
        release: tokio::sync::Semaphore::new(0),
    });
    let mut config = Config::try_parse_from([
        "geesefs-rs",
        "--endpoint=http://fixture.invalid",
        "--memory-limit=64",
        "bucket",
        "/tmp/mount",
    ])
    .unwrap();
    config.mountpoint = mount.path().into();
    config.disk_cache_dir = Some(directory.path().into());
    config.disk_cache_max_size = Some((16 * MIB) as u64);
    config.disk_cache_regex = Some(r"\.rpm$".parse().unwrap());
    config.read_ahead = 4096;
    config.read_ahead_small = 4096;
    config.read_ahead_large = 4096;
    config.read_ahead_parallel = 4096;
    config.max_requests = 1;
    let fs = Filesystem::new(config, source.clone()).unwrap();
    fs.attach_disk_cache(
        PreparedDiskCache::prepare(directory.path(), (16 * MIB) as u64)
            .unwrap()
            .with_test_io(Box::new(GatedConfig(gate.clone()))),
    )
    .await
    .unwrap();
    let ino = fs.lookup(1, "slow.rpm").await.unwrap().ino;
    let baseline = fs.effective_budgets().data_used_bytes;
    gate.held.store(true, Ordering::SeqCst);
    let reader = tokio::spawn({
        let fs = fs.clone();
        async move { fs.read(ino, 0, MIB).await }
    });
    // Entering actual payload I/O proves qualification already succeeded. Measure
    // only from this controlled boundary, not cold recovery or scheduler startup.
    let entered = tokio::time::timeout(DEADLINE, gate.entered.acquire()).await;
    if entered.is_err() {
        eprintln!(
            "no I/O: stats={:?} decisions={:?} source={:?} finished={}",
            fs.disk_cache_stats(),
            fs.disk_read_decisions(),
            source.ranges.lock().unwrap(),
            reader.is_finished()
        );
        gate.held.store(false, Ordering::SeqCst);
        gate.release.add_permits(64);
        source.release.add_permits(64);
        reader.abort();
        let _ = reader.await;
        tokio::time::timeout(DEADLINE, fs.shutdown())
            .await
            .unwrap()
            .unwrap();
        panic!("qualified read did not enter gated I/O");
    }
    entered.unwrap().unwrap().forget();
    let started = std::time::Instant::now();
    let first_get = tokio::time::timeout(Duration::from_secs(1), source.started.notified()).await;
    let first_get_elapsed = started.elapsed();
    let pending = fs.disk_cache_stats().unwrap();
    let funded = fs.effective_budgets().data_used_bytes;
    // On RED unblock discovery for cleanup. On GREEN source delivery must
    // finish while the actual timed-out disk worker remains gated and funded.
    if first_get.is_err() {
        gate.held.store(false, Ordering::SeqCst);
        gate.release.add_permits(64);
    }
    source.release.add_permits(64);
    let data = tokio::time::timeout(DEADLINE, reader)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(data.as_slice(), &source.bytes[..MIB]);
    for index in 1..4 {
        let data = tokio::time::timeout(DEADLINE, fs.read(ino, (index * MIB) as u64, MIB))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            data.as_slice(),
            &source.bytes[index * MIB..(index + 1) * MIB]
        );
    }
    let before_release = fs.effective_budgets().data_used_bytes;
    let active_before_release = fs.disk_cache_stats().unwrap().active_read_jobs;
    gate.held.store(false, Ordering::SeqCst);
    gate.release.add_permits(64);
    fs.drain_disk_cache(DEADLINE).await.unwrap();
    let after_release = fs.effective_budgets().data_used_bytes;
    assert_eq!(fs.disk_cache_stats().unwrap().active_read_jobs, 0);
    tokio::time::timeout(DEADLINE, fs.shutdown())
        .await
        .unwrap()
        .unwrap();
    eprintln!(
        "qualified first GET: {first_get_elapsed:?}; pending actual reads={}; timeout count={}; I/O funding released={}",
        pending.active_read_jobs,
        pending.read_fallbacks[ReadFallback::Timeout as usize],
        before_release.saturating_sub(after_release)
    );
    assert!(
        first_get.is_ok(),
        "first source GET must not wait for 32 renewed optional deadlines"
    );
    assert_eq!(
        pending.state,
        State::Ready,
        "timeout is not permanent bypass"
    );
    assert_eq!(
        pending.active_read_jobs, 1,
        "timed-out actual worker retains its slot"
    );
    assert_eq!(pending.read_fallbacks[ReadFallback::Timeout as usize], 1);
    assert_eq!(
        active_before_release, 1,
        "source completion cannot release actual disk ownership"
    );
    assert_eq!(
        before_release - after_release,
        BLOCK + crate::disk_cache::READ_WAIT_BYTES,
        "actual I/O retains exactly its output and waiter credit through source completion"
    );
    assert!(
        funded >= baseline + BLOCK,
        "actual output stays funded until I/O finishes"
    );
    assert_eq!(*source.ranges.lock().unwrap(), [(0, 4 * MIB)]);
}
