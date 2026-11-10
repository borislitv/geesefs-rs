use super::{
    DiskKey, Funding,
    funding::{JOBS, POOL},
    key::{DiskValue, RECORD_CAP},
    policy::Recent,
};
use crate::{
    budget::{Block, Budget, Reservation},
    error::{Error, Result},
};
use foyer::{
    BlockEngineConfig, Cache, CacheBuilder, DefaultHasher, DeviceBuilder, FileDeviceBuilder,
    HybridCacheBuilder, HybridCacheProperties, IndexUsage, InvalidRatioPicker, RecoverMode,
    StorageFilter, Store, SubmissionLimits,
};
use futures_util::{
    FutureExt,
    future::{BoxFuture, Shared},
};
use std::{
    fs::{File, OpenOptions},
    hash::BuildHasher,
    num::NonZeroUsize,
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{OwnedRwLockReadGuard, OwnedSemaphorePermit, RwLock, Semaphore};

const DATA_FILE: &str = "payload-v1.cache";
const LOCK_FILE: &str = ".lock";
const OPEN_DEADLINE: Duration = Duration::from_secs(120);
type Backend = Store<DiskKey, DiskValue, DefaultHasher, HybridCacheProperties>;
type Source = Cache<DiskKey, DiskValue, DefaultHasher, HybridCacheProperties>;
type Close = Shared<BoxFuture<'static, std::result::Result<(), Arc<str>>>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum State {
    Prepared,
    Recovering,
    Ready,
    Closing,
    Closed,
    Bypass,
}

#[derive(Clone)]
pub struct DiskCacheStatus(Arc<Startup>);

/// Completed operations only; nanoseconds use a monotonic clock. Concurrent
/// snapshots are observational, not a transaction across independent atomics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LatencyStats {
    pub count: u64,
    pub total_duration_ns: u64,
    pub max_duration_ns: u64,
}

#[derive(Default)]
struct Latency {
    count: AtomicU64,
    total: AtomicU64,
    max: AtomicU64,
}
impl Latency {
    fn record(&self, started: Instant) {
        let ns = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
        self.total.fetch_add(ns, Ordering::Relaxed);
        self.max.fetch_max(ns, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Release);
    }
    fn stats(&self) -> LatencyStats {
        LatencyStats {
            count: self.count.load(Ordering::Acquire),
            total_duration_ns: self.total.load(Ordering::Relaxed),
            max_duration_ns: self.max.load(Ordering::Relaxed),
        }
    }
}

/// Per-open phases, each with count 0 (not finished/not attempted) or 1.
/// Startup is open-entry through owned completion, excluding prepare().
/// Allocation includes blocking-pool queue, file validation/preallocation/sync.
/// Recovery is subsequent engine construction/strict recovery, not allocation.
/// Failed phases are included; caller timeout/cancellation does not end a phase.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StartupStats {
    pub startup: LatencyStats,
    pub allocation: LatencyStats,
    pub recovery: LatencyStats,
}

#[derive(Default)]
struct Startup {
    state: AtomicU8,
    startup: Latency,
    allocation: Latency,
    recovery: Latency,
    file_allocated_bytes: AtomicU64,
    file_observed: std::sync::atomic::AtomicBool,
}
impl DiskCacheStatus {
    pub fn state(&self) -> State {
        match self.0.state.load(Ordering::Acquire) {
            0 => State::Prepared,
            1 => State::Recovering,
            2 => State::Ready,
            3 => State::Closing,
            4 => State::Closed,
            _ => State::Bypass,
        }
    }
    fn set(&self, state: State) {
        self.0.state.store(state as u8, Ordering::Release);
    }
    /// Available even while open is pending, or after it fails/is cancelled.
    pub fn startup(&self) -> StartupStats {
        StartupStats {
            startup: self.0.startup.stats(),
            allocation: self.0.allocation.stats(),
            recovery: self.0.recovery.stats(),
        }
    }
    /// One startup st_blocks * 512 observation, never a live filesystem scrape
    /// or a claim about physical device/cache residency. None means unobserved.
    pub fn opened_file_allocated_bytes(&self) -> Option<u64> {
        self.0
            .file_observed
            .load(Ordering::Acquire)
            .then(|| self.0.file_allocated_bytes.load(Ordering::Relaxed))
    }
}

fn io(error: std::io::Error) -> Error {
    Error::Fs(error.raw_os_error().unwrap_or(libc::EIO))
}
fn engine(error: impl std::fmt::Display) -> Error {
    Error::Config(format!("optional L2: {error}"))
}
fn private(metadata: &std::fs::Metadata) -> Result<()> {
    if metadata.uid() != rustix::process::geteuid().as_raw() || metadata.mode() & 0o077 != 0 {
        return Err(Error::Config(
            "disk cache must be private to the daemon uid".into(),
        ));
    }
    Ok(())
}
fn options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    options
}

/// Startup ownership, acquired before engine open/preallocation. Preparation is
/// blocking filesystem work; invoke at startup, not on a request executor path.
pub struct PreparedDiskCache {
    directory: PathBuf,
    capacity: usize,
    _lock: File,
    status: DiskCacheStatus,
}
impl PreparedDiskCache {
    pub fn prepare(directory: &Path, capacity: u64) -> Result<Self> {
        // Validate geometry without allocating engine resources.
        let funding = Funding::derive(capacity, usize::MAX / 4)?;
        let directory = Self::prepare_directory(directory)?;
        let lock = Self::lock_directory(&directory)?;
        Self::validate_inventory(&directory, funding.capacity)?;
        Ok(Self {
            directory,
            capacity: funding.capacity,
            _lock: lock,
            status: DiskCacheStatus(Arc::new(Startup::default())),
        })
    }

    fn prepare_directory(directory: &Path) -> Result<PathBuf> {
        match std::fs::DirBuilder::new().mode(0o700).create(directory) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(io(e)),
        }
        let metadata = std::fs::symlink_metadata(directory).map_err(io)?;
        if !metadata.is_dir() {
            return Err(Error::Config(
                "disk cache directory must not be a symlink".into(),
            ));
        }
        private(&metadata)?;
        let directory = directory.canonicalize().map_err(io)?;
        let directory_file = File::open(&directory).map_err(io)?;
        let fs = rustix::fs::fstatfs(&directory_file).map_err(|e| Error::Fs(e.raw_os_error()))?;
        // Known local Linux filesystems, fail closed for network/FUSE/unknown
        // types. Overlay backing placement remains an operator responsibility.
        if ![
            0xef53, 0x58465342, 0x9123683e, 0x01021994, 0x794c7630, 0x858458f6, 0xf2f52010,
            0x2fc12fc1, 0xca451a4e,
        ]
        .contains(&(fs.f_type as u64))
        {
            return Err(Error::Config(
                "disk cache requires a local, non-FUSE filesystem".into(),
            ));
        }
        Ok(directory)
    }

    fn lock_directory(directory: &Path) -> Result<File> {
        let lock = options()
            .create(true)
            .truncate(false)
            .open(directory.join(LOCK_FILE))
            .map_err(io)?;
        private(&lock.metadata().map_err(io)?)?;
        if !lock.metadata().map_err(io)?.is_file() || lock.metadata().map_err(io)?.nlink() != 1 {
            return Err(Error::Config("invalid disk cache lock file".into()));
        }
        lock.try_lock().map_err(|error| match error {
            std::fs::TryLockError::WouldBlock => Error::Fs(libc::EWOULDBLOCK),
            std::fs::TryLockError::Error(error) => io(error),
        })?;
        Ok(lock)
    }

    fn validate_inventory(directory: &Path, capacity: usize) -> Result<()> {
        for entry in std::fs::read_dir(directory).map_err(io)? {
            let name = entry.map_err(io)?.file_name();
            if name != DATA_FILE && name != LOCK_FILE {
                return Err(Error::Unsupported(
                    "unrecognized files in private disk cache; no automatic reset".into(),
                ));
            }
        }
        let data = directory.join(DATA_FILE);
        if let Ok(meta) = std::fs::symlink_metadata(&data) {
            private(&meta)?;
            if !meta.is_file() || meta.nlink() != 1 || meta.len() != capacity as u64 {
                return Err(Error::Unsupported(
                    "cache geometry changed/unsafe; media left untouched".into(),
                ));
            }
        }
        Ok(())
    }
    pub fn status(&self) -> DiskCacheStatus {
        self.status.clone()
    }
    pub fn funding(&self, available_payload_bytes: usize) -> Result<Funding> {
        Funding::derive(self.capacity as u64, available_payload_bytes)
    }

    fn allocate(&self) -> Result<Option<u64>> {
        let path = self.directory.join(DATA_FILE);
        match options().create_new(true).open(&path) {
            Ok(file) => self.allocate_new(file),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                self.validate_allocation(&path)
            }
            Err(e) => Err(io(e)),
        }
    }

    fn allocate_new(&self, file: File) -> Result<Option<u64>> {
        // Exactly one actual preallocation, only for a newly owned file.
        // Failure leaves media for explicit operator handling, never reset.
        rustix::fs::fallocate(
            &file,
            rustix::fs::FallocateFlags::empty(),
            0,
            self.capacity as u64,
        )
        .map_err(|e| Error::Fs(e.raw_os_error()))?;
        file.sync_all().map_err(io)?;
        File::open(&self.directory)
            .map_err(io)?
            .sync_all()
            .map_err(io)?;
        // Telemetry failure must not turn a successful preallocation into an
        // unavailable tier. Retain only this observation, never the File/path.
        Ok(file
            .metadata()
            .ok()
            .map(|meta| meta.blocks().saturating_mul(512)))
    }

    fn validate_allocation(&self, path: &Path) -> Result<Option<u64>> {
        let file = options().open(path).map_err(io)?;
        let meta = file.metadata().map_err(io)?;
        if meta.len() != self.capacity as u64 || meta.blocks().saturating_mul(512) < meta.len() {
            return Err(Error::Unsupported(
                "incomplete preallocation/changed media; refusing destructive repair".into(),
            ));
        }
        Ok(Some(meta.blocks().saturating_mul(512)))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Demotion {
    /// Adapter copy/job admission only, never a publication/durability receipt.
    Admitted,
    /// Advisory hash-index presence, not a validated cache hit.
    IndexedHint,
    Dropped,
}

#[derive(Debug)]
pub struct Stats {
    pub state: State,
    pub active_jobs: usize,
    pub copied_bytes: usize,
    pub submission_bytes: usize,
    pub submission_items: usize,
    pub index_entries: usize,
    pub recent_entries: usize,
    pub hits: u64,
    pub misses: u64,
    pub drops: u64,
    pub errors: u64,
    pub managed_bytes: usize,
    pub io_read_bytes: u64,
    pub io_write_bytes: u64,
    /// Exact copied payload admitted by the adapter; excludes IndexedHint.
    pub admitted_demotions: u64,
    /// Fallible Store enqueue accepted, not a payload publication/durability ack.
    pub engine_admitted_demotions: u64,
    pub spill_no_space: u64,
    /// Non-pressure enqueue errors or observed guarded drain failures.
    pub spill_errors: u64,
    /// Admitted copies: try_demote entry through enqueue rejection or joined
    /// guarded drain, including copying, task queue and I/O. Not per-record fsync.
    pub spill_operations: LatencyStats,
    /// Job-admitted reads: reserved-read entry through load/validation/output
    /// copy completion, including task queue/I/O, even after caller cancellation.
    pub read_operations: LatencyStats,
    /// Actual completed read results; independent of caller-delivered hits/misses.
    pub read_operation_hits: u64,
    pub read_operation_misses: u64,
    /// Backend load errors, including I/O/channel and decode/integrity failures.
    pub read_operation_errors: u64,
    /// Engine region-eviction callbacks, not per-record eviction counts.
    pub region_reclaims_started: u64,
    pub startup: StartupStats,
    pub opened_file_allocated_bytes: Option<u64>,
}

struct Inner {
    backend: Backend,
    factory: foyer::HybridCache<DiskKey, DiskValue>,
    source: Source,
    prepared: Mutex<Option<PreparedDiskCache>>,
    funding: Funding,
    copies: Budget,
    jobs: Arc<Semaphore>,
    activity: Arc<RwLock<()>>,
    close: Mutex<Option<Close>>,
    status: DiskCacheStatus,
    limits: SubmissionLimits,
    index: IndexUsage,
    recent: Recent,
    runtime: tokio::runtime::Handle,
    hits: AtomicU64,
    misses: AtomicU64,
    drops: AtomicU64,
    errors: AtomicU64,
    admitted_demotions: AtomicU64,
    engine_admitted_demotions: AtomicU64,
    spill_no_space: AtomicU64,
    spill_errors: AtomicU64,
    spill_operations: Latency,
    read_operations: Latency,
    read_operation_hits: AtomicU64,
    read_operation_misses: AtomicU64,
    read_operation_errors: AtomicU64,
    // Last field: every funded owner is destroyed before returning admission.
    _funding_owner: Reservation,
}
struct Handle(Arc<Inner>);
impl Drop for Handle {
    fn drop(&mut self) {
        drop(Inner::begin_close(&self.0));
    }
}
#[derive(Clone)]
pub struct DiskCache(Arc<Handle>);

struct Job {
    _permit: OwnedSemaphorePermit,
    _activity: OwnedRwLockReadGuard<()>,
}
impl Inner {
    fn admit(&self) -> Option<Job> {
        let _boundary = self.close.lock().unwrap();
        if self.status.state() != State::Ready {
            return None;
        }
        if self.limits.is_poisoned() {
            self.fail();
            return None;
        }
        Some(Job {
            _permit: self.jobs.clone().try_acquire_owned().ok()?,
            _activity: self.activity.clone().try_read_owned().ok()?,
        })
    }
    fn fail(&self) {
        self.errors.fetch_add(1, Ordering::Relaxed);
        self.status.set(State::Bypass);
    }
    fn begin_close(this: &Arc<Self>) -> Close {
        let mut close = this.close.lock().unwrap();
        if let Some(close) = &*close {
            return close.clone();
        }
        this.status.set(State::Closing);
        let inner = this.clone();
        let task = this.runtime.spawn(async move {
            let _exclusive = inner.activity.clone().write_owned().await;
            match inner.factory.close().await {
                Ok(()) => {
                    drop(inner.prepared.lock().unwrap().take());
                    inner.status.set(State::Closed);
                    Ok(())
                }
                Err(error) => {
                    inner.fail();
                    Err(Arc::<str>::from(error.to_string()))
                }
            }
        });
        let future = async move { task.await.map_err(|e| Arc::<str>::from(e.to_string()))? }
            .boxed()
            .shared();
        *close = Some(future.clone());
        future
    }
}

impl DiskCache {
    pub async fn open(prepared: PreparedDiskCache, resources: &Budget) -> Result<Self> {
        Self::start(prepared, resources, None, None).await
    }
    #[cfg(test)]
    pub async fn open_test(
        prepared: PreparedDiskCache,
        resources: &Budget,
        io: Option<Box<dyn foyer::IoEngineConfig>>,
    ) -> Result<Self> {
        Self::start(prepared, resources, io, None).await
    }
    /// Exercise the real Store's pressure boundary with less than the funded
    /// submission allowance. Production admission/funding remain unchanged.
    #[cfg(test)]
    pub async fn open_test_with_submission_limit(
        prepared: PreparedDiskCache,
        resources: &Budget,
        io: Box<dyn foyer::IoEngineConfig>,
        bytes: NonZeroUsize,
    ) -> Result<Self> {
        if bytes.get() > prepared.funding(resources.limit())?.submission_bytes {
            return Err(Error::Config(
                "test submission limit exceeds funding".into(),
            ));
        }
        let limits = SubmissionLimits::new(bytes, NonZeroUsize::new(2 * JOBS + 4).unwrap())
            .map_err(engine)?;
        Self::start(prepared, resources, Some(io), Some(limits)).await
    }
    async fn start(
        prepared: PreparedDiskCache,
        resources: &Budget,
        io_config: Option<Box<dyn foyer::IoEngineConfig>>,
        limits: Option<SubmissionLimits>,
    ) -> Result<Self> {
        let started = Instant::now();
        let funding = prepared.funding(resources.limit()).inspect_err(|_| {
            prepared.status.0.startup.record(started);
            prepared.status.set(State::Bypass);
        })?;
        let owner = resources
            .try_reserve(funding.required_bytes)
            .inspect_err(|_| {
                prepared.status.0.startup.record(started);
                prepared.status.set(State::Bypass);
            })?;
        let status = prepared.status();
        status.set(State::Recovering);
        let observer = status.clone();
        // Owned startup retains directory and funding if its caller is cancelled.
        let task = tokio::spawn(async move {
            let result = Self::build(prepared, funding, owner, io_config, limits).await;
            observer.0.startup.record(started);
            if result.is_err() {
                observer.set(State::Bypass);
            }
            result
        });
        tokio::time::timeout(OPEN_DEADLINE, task)
            .await
            .map_err(|_| Error::Timeout("L2 recovery"))?
            .map_err(engine)?
    }
    async fn build(
        prepared: PreparedDiskCache,
        funding: Funding,
        owner: Reservation,
        io_config: Option<Box<dyn foyer::IoEngineConfig>>,
        limits: Option<SubmissionLimits>,
    ) -> Result<Self> {
        let status = prepared.status();
        let allocation_started = Instant::now();
        let allocation = tokio::task::spawn_blocking(move || {
            let allocated_bytes = prepared.allocate()?;
            Ok::<_, Error>((prepared, allocated_bytes))
        })
        .await;
        status.0.allocation.record(allocation_started);
        let (prepared, allocated_bytes) = allocation.map_err(engine)??;
        if let Some(bytes) = allocated_bytes {
            status
                .0
                .file_allocated_bytes
                .store(bytes, Ordering::Relaxed);
            status.0.file_observed.store(true, Ordering::Release);
        }
        let recovery_started = Instant::now();
        let result = Self::build_engine(prepared, funding, owner, io_config, limits).await;
        status.0.recovery.record(recovery_started);
        result
    }
    async fn build_engine(
        prepared: PreparedDiskCache,
        funding: Funding,
        owner: Reservation,
        io_config: Option<Box<dyn foyer::IoEngineConfig>>,
        limits: Option<SubmissionLimits>,
    ) -> Result<Self> {
        let recent = Recent::new(funding.recent_entries);
        let limits = match limits {
            Some(limits) => limits,
            None => SubmissionLimits::new(
                NonZeroUsize::new(funding.submission_bytes).unwrap(),
                NonZeroUsize::new(2 * JOBS + 4).unwrap(),
            )
            .map_err(engine)?,
        };
        let device = FileDeviceBuilder::new(prepared.directory.join(DATA_FILE))
            .with_capacity(funding.capacity)
            .build()
            .map_err(engine)?;
        let config = BlockEngineConfig::new(device)
            .with_block_size(funding.region_bytes)
            .with_buffer_pool_size(POOL)
            .with_indexer_shards(1)
            .with_index_capacity(NonZeroUsize::new(funding.index_entries).unwrap())
            .with_recovery_scratch_limit(NonZeroUsize::new(funding.recovery_bytes).unwrap())
            .with_recover_concurrency(1)
            .with_flushers(1)
            .with_reclaimers(1)
            .with_clean_block_threshold((funding.capacity / funding.region_bytes / 20).max(1))
            .with_tombstone_log(true)
            .with_read_limits(
                NonZeroUsize::new(RECORD_CAP).unwrap(),
                foyer::Compression::None,
            )
            .with_submission_limits(limits.clone())
            .with_eviction_pickers(vec![
                Box::new(InvalidRatioPicker::new(0.8)),
                Box::new(recent.picker()),
            ])
            .with_reinsertion_filter(StorageFilter::new().with_condition(recent.clone()));
        let index = config.index_usage();
        // Public StoreBuilder requires an unexported metrics type. Use
        // the public builder only as a passive factory. Retain its empty shell
        // until explicit close (dropping it would close its Store). NEVER call
        // HybridCache insert/writer/eviction APIs; flush-on-close stays disabled.
        let builder = HybridCacheBuilder::new()
            .with_flush_on_close(false)
            .memory(1)
            .storage()
            .with_recover_mode(RecoverMode::Strict)
            .with_compression(foyer::Compression::None)
            .with_engine_config(config);
        let builder = if let Some(io) = io_config {
            builder.with_io_engine_config(io)
        } else {
            builder
        };
        let shell = builder.build().await.map_err(engine)?;
        let backend = shell.storage().clone();
        // Rejecting the memory admission filter yields detached phantom Pieces,
        // with no automatic spill listener or retained second RAM payload cache.
        let source = CacheBuilder::new(1)
            .with_shards(1)
            .with_filter(|_: &DiskKey, _: &DiskValue| false)
            .build();
        let status = prepared.status();
        let inner = Arc::new(Inner {
            backend,
            factory: shell,
            source,
            prepared: Mutex::new(Some(prepared)),
            copies: Budget::new(funding.copy_bytes)?,
            funding,
            _funding_owner: owner,
            jobs: Arc::new(Semaphore::new(JOBS)),
            activity: Arc::new(RwLock::new(())),
            close: Mutex::new(None),
            status,
            limits,
            index,
            recent,
            runtime: tokio::runtime::Handle::current(),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            drops: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            admitted_demotions: AtomicU64::new(0),
            engine_admitted_demotions: AtomicU64::new(0),
            spill_no_space: AtomicU64::new(0),
            spill_errors: AtomicU64::new(0),
            spill_operations: Latency::default(),
            read_operations: Latency::default(),
            read_operation_hits: AtomicU64::new(0),
            read_operation_misses: AtomicU64::new(0),
            read_operation_errors: AtomicU64::new(0),
        });
        inner.status.set(State::Ready);
        Ok(Self(Arc::new(Handle(inner))))
    }
    fn inner(&self) -> &Arc<Inner> {
        &self.0.0
    }
    pub fn funding(&self) -> &Funding {
        &self.inner().funding
    }
    pub fn status(&self) -> DiskCacheStatus {
        self.inner().status.clone()
    }
    pub fn stats(&self) -> Stats {
        let i = self.inner();
        Stats {
            state: i.status.state(),
            active_jobs: JOBS - i.jobs.available_permits(),
            copied_bytes: i.copies.used(),
            submission_bytes: i.limits.bytes(),
            submission_items: i.limits.items(),
            index_entries: i.index.entries(),
            recent_entries: i.recent.len(),
            hits: i.hits.load(Ordering::Relaxed),
            misses: i.misses.load(Ordering::Relaxed),
            drops: i.drops.load(Ordering::Relaxed),
            errors: i.errors.load(Ordering::Relaxed),
            managed_bytes: i.funding.capacity,
            io_read_bytes: i.backend.statistics().disk_read_bytes() as u64,
            io_write_bytes: i.backend.statistics().disk_write_bytes() as u64,
            admitted_demotions: i.admitted_demotions.load(Ordering::Relaxed),
            engine_admitted_demotions: i.engine_admitted_demotions.load(Ordering::Relaxed),
            spill_no_space: i.spill_no_space.load(Ordering::Relaxed),
            spill_errors: i.spill_errors.load(Ordering::Relaxed),
            spill_operations: i.spill_operations.stats(),
            read_operations: i.read_operations.stats(),
            read_operation_hits: i.read_operation_hits.load(Ordering::Relaxed),
            read_operation_misses: i.read_operation_misses.load(Ordering::Relaxed),
            read_operation_errors: i.read_operation_errors.load(Ordering::Relaxed),
            region_reclaims_started: i.recent.region_reclaims_started(),
            startup: i.status.startup(),
            opened_file_allocated_bytes: i.status.opened_file_allocated_bytes(),
        }
    }

    /// Best-effort immutable-key population. Consumes and promptly drops the
    /// victim; loans are rejected. Only an admitted exact visible-range copy may
    /// cross the async boundary, never a clone of a larger sliced backing.
    pub fn try_demote(&self, key: DiskKey, victim: Block) -> Demotion {
        let started = Instant::now();
        let i = self.inner();
        let outcome = (|| {
            if victim.is_loaned() || victim.as_slice().len() != key.covered_len() {
                return None;
            }
            let job = i.admit()?;
            if i.backend.may_contains(&key) {
                return Some(Demotion::IndexedHint);
            }
            let copy = i
                .copies
                .try_reserve(key.covered_len())
                .ok()?
                .copy(victim.as_slice())
                .ok()?;
            i.admitted_demotions.fetch_add(1, Ordering::Relaxed);
            let inner = i.clone();
            i.runtime.spawn(async move {
                let _job = job;
                let entry = inner.source.insert(key, DiskValue::Source(copy));
                let admitted = inner.backend.try_enqueue(entry.piece(), true);
                drop(entry);
                match admitted {
                    Err(error) => {
                        inner.drops.fetch_add(1, Ordering::Relaxed);
                        if error.kind() == foyer::ErrorKind::NoSpace {
                            inner.spill_no_space.fetch_add(1, Ordering::Relaxed);
                        } else {
                            inner.spill_errors.fetch_add(1, Ordering::Relaxed);
                            inner.fail();
                        }
                    }
                    Ok(_) => {
                        inner
                            .engine_admitted_demotions
                            .fetch_add(1, Ordering::Relaxed);
                        if inner.backend.wait_guarded().await.is_err() {
                            inner.spill_errors.fetch_add(1, Ordering::Relaxed);
                            inner.fail();
                        }
                    }
                }
                inner.spill_operations.record(started);
            });
            Some(Demotion::Admitted)
        })();
        // victim is never captured by the spawned future.
        drop(victim);
        outcome.unwrap_or_else(|| {
            i.drops.fetch_add(1, Ordering::Relaxed);
            Demotion::Dropped
        })
    }

    /// Full exact-key/coverage read. No waiting for optional memory or jobs; a
    /// miss lets the caller use S3. Output is admitted BEFORE allocation/I/O.
    /// Spawned ownership retains permits if the request future is cancelled.
    /// The default optional wait is at most 250 ms; integration may supply its
    /// remaining request deadline with [`Self::read_block_with_deadline`].
    pub async fn read_block(&self, key: &DiskKey, output: &Budget) -> Option<Block> {
        self.read_block_with_deadline(key, output, Duration::from_millis(250))
            .await
    }

    pub async fn read_block_with_deadline(
        &self,
        key: &DiskKey,
        output: &Budget,
        deadline: Duration,
    ) -> Option<Block> {
        let Ok(output) = output.try_reserve(key.covered_len()) else {
            self.inner().misses.fetch_add(1, Ordering::Relaxed);
            return None;
        };
        self.read_block_reserved(key, output, deadline).await
    }

    /// Transfer existing caller admission, avoiding a second reservation in an
    /// already-admitted read/fill pipeline. Size must match exact record coverage.
    pub async fn read_block_reserved(
        &self,
        key: &DiskKey,
        output: Reservation,
        deadline: Duration,
    ) -> Option<Block> {
        let started = Instant::now();
        let i = self.inner();
        if output.size() != key.covered_len() {
            i.misses.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        let Some(job) = i.admit() else {
            i.misses.fetch_add(1, Ordering::Relaxed);
            return None;
        };
        let inner = i.clone();
        let key = key.clone();
        let task = i.runtime.spawn(async move {
            let _job = job;
            let loaded = inner.backend.load(&key).await;
            let load_error = loaded.is_err();
            let result = match loaded {
                Ok(load) => load.kv().and_then(|(actual, value)| {
                    if actual != key || value.bytes().len() != key.covered_len() {
                        return None;
                    }
                    output.copy(value.bytes()).ok()
                }),
                Err(error) => {
                    if matches!(
                        error.kind(),
                        foyer::ErrorKind::Io
                            | foyer::ErrorKind::Join
                            | foyer::ErrorKind::ChannelClosed
                    ) {
                        inner.fail();
                    } else {
                        inner.errors.fetch_add(1, Ordering::Relaxed);
                    }
                    None
                }
            };
            if result.is_some() {
                inner.recent.touch(DefaultHasher::default().hash_one(&key));
                inner.read_operation_hits.fetch_add(1, Ordering::Relaxed);
            } else if load_error {
                inner.read_operation_errors.fetch_add(1, Ordering::Relaxed);
            } else {
                inner.read_operation_misses.fetch_add(1, Ordering::Relaxed);
            }
            inner.read_operations.record(started);
            result
        });
        let result = tokio::time::timeout(deadline, task)
            .await
            .ok()
            .and_then(|joined| joined.ok())
            .flatten();
        if result.is_some() {
            i.hits.fetch_add(1, Ordering::Relaxed);
        } else {
            i.misses.fetch_add(1, Ordering::Relaxed);
        }
        result
    }

    /// Advisory only: caller already validated this exact L2 identity through L1.
    /// Touching a missing/pending record neither creates it nor marks new spills.
    pub fn touch_validated(&self, key: &DiskKey) -> bool {
        let i = self.inner();
        if i.status.state() == State::Ready && i.backend.touch(key) {
            i.recent.touch(DefaultHasher::default().hash_one(key));
            true
        } else {
            false
        }
    }
    pub async fn drain(&self, deadline: Duration) -> Result<()> {
        let i = self.inner();
        tokio::time::timeout(deadline, async {
            let _exclusive = i.activity.clone().write_owned().await;
            i.backend.wait_guarded().await.map_err(engine)
        })
        .await
        .map_err(|_| Error::Timeout("L2 drain"))?
    }
    /// Deadline limits this caller, not kernel I/O. Owned close continues and
    /// retains directory/funding until real jobs and engine shutdown finish.
    pub async fn close(&self, deadline: Duration) -> Result<()> {
        tokio::time::timeout(deadline, Inner::begin_close(self.inner()))
            .await
            .map_err(|_| Error::Timeout("L2 close"))?
            .map_err(engine)
    }
}
