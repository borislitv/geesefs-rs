//! Controlled native-engine operations for adapter tests; real file I/O remains in Foyer.
use super::{DiskKey, key::DiskValue};
use foyer::{
    BlockEngineConfig, Device, Engine, EngineBuildContext, EngineConfig, HybridCacheProperties,
    Load, PieceRef, StorageFilterResult,
};
use futures_util::future::BoxFuture;
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, Ordering},
};

type Native = dyn Engine<DiskKey, DiskValue, HybridCacheProperties>;

#[derive(Debug)]
pub struct TestGate {
    pub hold_reads: AtomicBool,
    pub fail_read: AtomicBool,
    pub entered: tokio::sync::Semaphore,
    pub release: tokio::sync::Semaphore,
    flush: foyer_storage::test_utils::Switch,
    engine: Mutex<Option<Weak<Native>>>,
    device: Mutex<Option<Weak<dyn Device>>>,
}

impl TestGate {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            hold_reads: AtomicBool::new(false),
            fail_read: AtomicBool::new(false),
            entered: tokio::sync::Semaphore::new(0),
            release: tokio::sync::Semaphore::new(0),
            flush: foyer_storage::test_utils::Switch::default(),
            engine: Mutex::new(None),
            device: Mutex::new(None),
        })
    }

    pub fn hold_writes(&self) {
        self.flush.on();
    }

    pub fn device_released(&self) -> bool {
        self.device
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .upgrade()
            .is_none()
    }

    pub fn release_writes(&self) {
        self.flush.off();
        if let Some(engine) = self.engine.lock().unwrap().as_ref().and_then(Weak::upgrade) {
            // The upstream polling Switch needs a channel event after release.
            tokio::spawn(engine.wait());
        }
    }

    async fn before_read(&self) -> foyer::Result<()> {
        if self.hold_reads.load(Ordering::SeqCst) {
            self.entered.add_permits(1);
            self.release.acquire().await.unwrap().forget();
        }
        if self.fail_read.swap(false, Ordering::SeqCst) {
            return Err(foyer::Error::io_error(std::io::Error::from_raw_os_error(
                libc::EIO,
            )));
        }
        Ok(())
    }

    pub(super) fn wrap(
        self: &Arc<Self>,
        config: BlockEngineConfig<DiskKey, DiskValue, HybridCacheProperties>,
    ) -> Box<dyn EngineConfig<DiskKey, DiskValue, HybridCacheProperties>> {
        Box::new(GateConfig {
            config: config.with_flush_switch(self.flush.clone()),
            gate: self.clone(),
        })
    }
}

#[derive(Debug)]
struct GateConfig {
    config: BlockEngineConfig<DiskKey, DiskValue, HybridCacheProperties>,
    gate: Arc<TestGate>,
}

impl EngineConfig<DiskKey, DiskValue, HybridCacheProperties> for GateConfig {
    fn build(
        self: Box<Self>,
        context: EngineBuildContext,
    ) -> BoxFuture<'static, foyer::Result<Arc<Native>>> {
        Box::pin(async move {
            self.gate.before_read().await?;
            let inner: Arc<Native> = Box::new(self.config).build(context).await?;
            *self.gate.engine.lock().unwrap() = Some(Arc::downgrade(&inner));
            *self.gate.device.lock().unwrap() = Some(Arc::downgrade(inner.device()));
            Ok(Arc::new(GateEngine {
                inner,
                gate: self.gate,
            }) as Arc<Native>)
        })
    }
}

#[derive(Debug)]
struct GateEngine {
    inner: Arc<Native>,
    gate: Arc<TestGate>,
}

impl Engine<DiskKey, DiskValue, HybridCacheProperties> for GateEngine {
    fn device(&self) -> &Arc<dyn Device> {
        self.inner.device()
    }
    fn filter(&self, hash: u64, size: usize) -> StorageFilterResult {
        self.inner.filter(hash, size)
    }
    fn enqueue(&self, piece: PieceRef<DiskKey, DiskValue, HybridCacheProperties>, size: usize) {
        self.inner.enqueue(piece, size);
        if self.gate.flush.is_on() {
            self.gate.entered.add_permits(1);
        }
    }
    fn load(
        &self,
        hash: u64,
    ) -> BoxFuture<'static, foyer::Result<Load<DiskKey, DiskValue, HybridCacheProperties>>> {
        let gate = self.gate.clone();
        let inner = self.inner.clone();
        Box::pin(async move {
            gate.before_read().await?;
            inner.load(hash).await
        })
    }
    fn delete(&self, hash: u64) {
        self.inner.delete(hash);
    }
    fn may_contains(&self, hash: u64) -> bool {
        self.inner.may_contains(hash)
    }
    fn destroy(&self) -> BoxFuture<'static, foyer::Result<()>> {
        self.inner.destroy()
    }
    fn wait(&self) -> BoxFuture<'static, ()> {
        self.inner.wait()
    }
    fn close(&self) -> BoxFuture<'static, foyer::Result<()>> {
        self.inner.close()
    }
}
