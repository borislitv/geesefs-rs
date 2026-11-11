use std::{
    collections::BTreeMap,
    future::Future,
    ops::Bound::{Excluded, Included, Unbounded},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

use crate::{
    backend::{Store, s3::ObjectHead},
    budget::{Block, Budget},
    config::{Config, PayloadBudgets},
    error::{Error, Result},
    inode::{Inode, State, file_type},
    metrics::{self, Stage},
    namespace::{MoveLease, Namespace, child},
    read,
    write::{self, MAX_IO},
};
use fuser::{FileAttr, FileType};
use futures_util::{
    FutureExt, StreamExt,
    future::{BoxFuture, Either, Shared, ready},
    stream::FuturesUnordered,
};
use tokio::sync::{Notify, RwLock, Semaphore};

pub struct Filesystem {
    pub config: Arc<Config>,
    pub store: Arc<dyn Store>,
    pub namespace: Arc<Mutex<Namespace>>,
    inodes: Arc<crate::namespace::InodeIndex>,
    pub ingress: Budget,
    metadata: crate::metadata_budget::MetadataBudget,
    data: Budget,
    cache: crate::cache::ReadCache,
    disk: Arc<Mutex<DiskTier>>,
    disk_funding_bytes: usize,
    disk_read_bytes: std::sync::atomic::AtomicU64,
    disk_decisions: Arc<[std::sync::atomic::AtomicU64; 16]>,
    cache_published: Notify,
    flush_lane: Budget,
    flush_activity: Arc<RwLock<()>>,
    flushers: Arc<Semaphore>,
    upload_admission: write::UploadAdmission,
    metadata_slots: Arc<Semaphore>,
    directory_pages: moka::sync::Cache<(u64, u64, String, usize), Arc<DirectoryPage>>,
    subtree_pages: moka::sync::Cache<(u64, String), Arc<SubtreePage>>,
    preloaded_directories:
        moka::sync::Cache<u64, Arc<(std::time::Instant, crate::metadata_budget::MetadataCharge)>>,
    closing: AtomicBool,
    read_metrics: crate::metrics::ReadMetrics,
    pub(crate) diagnostics: crate::metrics::Diagnostics,
}

struct DirectoryPage {
    metadata_charge: crate::metadata_budget::MetadataCharge,
    fetched: std::time::Instant,
    entries: Vec<DirEntry>,
}

// One bounded flat response, shared by the complete directories it covers.
// The namespace epoch invalidates speculative ranges on local name changes.
struct SubtreePage {
    _metadata_charge: crate::metadata_budget::MetadataCharge,
    fetched: Instant,
    objects: Vec<(crate::metadata_budget::SharedKey, DirectoryMetadata)>,
}

impl DirectoryPage {
    fn remaining(&self, after: &str) -> &[DirEntry] {
        let start = self
            .entries
            .partition_point(|entry| entry.name.as_str() <= after);
        &self.entries[start..]
    }

    fn remainder(&self, after: &str) -> Result<DirectoryRemainder> {
        Ok(DirectoryRemainder {
            entries: self
                .remaining(after)
                .iter()
                .map(|entry| entry.try_clone(self.metadata_charge.budget()))
                .collect::<Result<_>>()?,
            next_after: self
                .entries
                .last()
                .map(|entry| {
                    let name = entry.name.as_str().max(after);
                    let charge = self.metadata_charge.budget().charge(name.len())?;
                    Ok(crate::metadata_budget::MetadataString::new(
                        name.to_owned(),
                        charge,
                    ))
                })
                .transpose()?,
        })
    }
}

struct DirectoryMetadata {
    _metadata_charge: crate::metadata_budget::MetadataCharge,
    directory: bool,
    head: Option<crate::backend::metadata::CachedRemote>,
    last_modified: Option<SystemTime>,
    key: Option<crate::metadata_budget::SharedKey>,
    known: bool,
    fetched: std::time::Instant,
}

impl DirectoryMetadata {
    fn try_clone(
        &self,
        budget: &crate::metadata_budget::MetadataBudget,
        key_len: usize,
    ) -> Result<Self> {
        Ok(Self {
            _metadata_charge: budget.charge(std::mem::size_of::<Self>() + key_len)?,
            directory: self.directory,
            head: self.head.clone(),
            last_modified: self.last_modified,
            key: self.key.clone(),
            known: self.known,
            fetched: self.fetched,
        })
    }
}

type DirectorySelection = BTreeMap<String, DirectoryMetadata>;

fn share_listed_owners(
    ns: &Namespace,
    key: &mut crate::metadata_budget::SharedKey,
    entry: &mut DirectoryMetadata,
) {
    if let Some(node) = ns.named(key) {
        let state = node.state.lock().unwrap();
        *key = state.key.clone();
        entry.key = Some(key.clone());
        if let (Some(cached), Some(listed)) = (&state.remote, &entry.head)
            && cached.size == listed.size
            && cached.etag == listed.etag
            && cached.metadata == listed.metadata
        {
            entry.head = Some(cached.clone());
        }
    }
}

struct DirectorySnapshot {
    node: Arc<Inode>,
    prefix: String,
    epoch: u64,
    fetched: std::time::Instant,
}

fn direct_directory_name(name: &str) -> bool {
    !name.is_empty() && !name.contains('/')
}

fn same_copy_metadata(expected: &ObjectHead, actual: &ObjectHead) -> bool {
    let left = &expected.properties;
    let right = &actual.properties;
    expected.size == actual.size
        && expected.metadata == actual.metadata
        && left.expires == right.expires
        && [
            &left.content_type,
            &left.content_encoding,
            &left.content_disposition,
            &left.content_language,
            &left.cache_control,
            &left.website_redirect_location,
        ] == [
            &right.content_type,
            &right.content_encoding,
            &right.content_disposition,
            &right.content_language,
            &right.cache_control,
            &right.website_redirect_location,
        ]
}

fn compact_copy_head(head: &mut ObjectHead) {
    // A response's spare parser capacity is not part of the copied generation.
    // Normalize it inside the pre-admitted COPY working set before retention.
    let mut metadata = std::collections::HashMap::with_capacity(head.metadata.len());
    for (mut key, mut value) in head.metadata.drain() {
        key.shrink_to_fit();
        value.shrink_to_fit();
        metadata.insert(key, value);
    }
    head.metadata = metadata;
    head.etag.shrink_to_fit();
    for value in [
        &mut head.properties.content_type,
        &mut head.properties.content_encoding,
        &mut head.properties.content_disposition,
        &mut head.properties.content_language,
        &mut head.properties.cache_control,
        &mut head.properties.website_redirect_location,
    ]
    .into_iter()
    .flatten()
    {
        value.shrink_to_fit();
    }
}

fn trim_directory_selection(selected: &mut DirectorySelection, limit: usize) {
    if selected.len() > limit {
        selected.pop_last();
    }
}

fn listed_object(
    mut object: crate::backend::extended_list::ListedObject,
    fetched: std::time::Instant,
    budget: &crate::metadata_budget::MetadataBudget,
) -> Result<(crate::metadata_budget::SharedKey, DirectoryMetadata)> {
    let mut charge = budget.charge_with(|| {
        std::mem::size_of::<DirectoryMetadata>()
            + 2 * object.key.capacity()
            + std::mem::size_of::<ObjectHead>()
            + 256
            + object.etag.as_ref().map_or(0, String::capacity)
            + object
                .metadata
                .as_ref()
                .map_or(0, crate::metadata_budget::encoding_bytes)
    })?;
    let known =
        object.metadata.is_some() && object.etag.as_ref().is_some_and(|etag| !etag.is_empty());
    let metadata = crate::backend::metadata::CachedMetadata::from_admitted(
        object.metadata.unwrap_or_default(),
        &mut charge,
    )?;
    let head = ObjectHead {
        size: object.size,
        etag: object.etag.unwrap_or_default(),
        last_modified: object
            .last_modified
            .and_then(|date| SystemTime::try_from(date).ok()),
        ..Default::default()
    };
    let directory = object.key.ends_with('/');
    object.key.truncate(object.key.trim_end_matches('/').len());
    let head = crate::backend::metadata::CachedRemote::from_admitted(head, metadata, &mut charge)?;
    let key_len = object.key.len();
    let key = crate::metadata_budget::SharedKey::from_admitted(object.key, &mut charge)?;
    charge.resize(budget.estimate(|| {
        std::mem::size_of::<DirectoryMetadata>() + key_len + crate::metadata_budget::LINKS
    }))?;
    Ok((
        key.clone(),
        DirectoryMetadata {
            _metadata_charge: charge,
            directory,
            head: Some(head),
            key: Some(key),
            last_modified: object
                .last_modified
                .and_then(|date| SystemTime::try_from(date).ok()),
            known,
            fetched,
        },
    ))
}

#[derive(Debug)]
pub struct DirEntry {
    pub ino: u64,
    pub name: crate::metadata_budget::MetadataString,
    pub kind: FileType,
}

impl DirEntry {
    fn try_clone(&self, budget: &crate::metadata_budget::MetadataBudget) -> Result<Self> {
        let charge = budget.charge(4 * std::mem::size_of::<Self>() + self.name.len())?;
        Ok(Self {
            ino: self.ino,
            name: crate::metadata_budget::MetadataString::new(
                self.name.as_str().to_owned(),
                charge,
            ),
            kind: self.kind,
        })
    }
}

pub struct DirectoryRemainder {
    pub entries: Vec<DirEntry>,
    /// An exhausted page still advances; only a backend EOF has no next name.
    pub next_after: Option<crate::metadata_budget::MetadataString>,
}

/// An open file retains its inode until the last admitted user releases it.
pub struct OpenFile {
    fs: Arc<Filesystem>,
    node: Arc<Inode>,
    pattern: Mutex<read::Pattern>,
}

impl OpenFile {
    pub fn ino(&self) -> u64 {
        self.node.ino
    }

    pub async fn read(&self, offset: u64, length: usize) -> Result<Block> {
        Ok(self
            .fs
            .read_inner(
                self.node.ino,
                offset,
                length,
                false,
                Some(&self.pattern),
                &mut None::<fn(Result<Block>)>,
            )
            .await?
            .expect("a read without early publication retains its result"))
    }

    pub async fn read_reply_data(
        &self,
        offset: u64,
        length: usize,
        ready: impl FnOnce(Result<&[u8]>) + Send,
    ) -> Option<Error> {
        self.fs
            .read_reply_data_with_pattern(self.node.ino, offset, length, Some(&self.pattern), ready)
            .await
    }
}

impl Drop for OpenFile {
    fn drop(&mut self) {
        let mut state = self.node.state.lock().unwrap();
        state.opens -= 1;
    }
}

/// Effective owner budgets, not RSS or allocator measurements. The cache shares
/// the data lane and can borrow revocable, idle flush capacity for clean owners.
#[derive(Debug, serde::Serialize)]
pub struct EffectiveBudgets {
    pub payload_limit_bytes: usize,
    pub data_shared_bytes: usize,
    pub disk_cache_funding_bytes: usize,
    /// Upper bound including revocable idle flush loans. With Ready L2,
    /// selected victims demote before loans, so selected residency may exclude
    /// that lane even though this shared policy upper bound is unchanged.
    pub read_cache_policy_bytes: usize,
    pub read_cache_loaned_bytes: usize,
    pub ingress_bytes: usize,
    pub flush_bytes: usize,
    pub data_used_bytes: usize,
    pub ingress_used_bytes: usize,
    pub flush_used_bytes: usize,
    pub requested_read_ahead_bytes: usize,
    pub requested_small_window_bytes: usize,
    pub requested_large_window_bytes: usize,
    pub max_effective_window_bytes: usize,
    pub max_overlay_window_bytes: usize,
    pub read_parallel_chunk_bytes: usize,
    pub upload_chunk_bytes: usize,
    pub max_read_tasks_per_window: usize,
    pub max_request_tasks: usize,
    pub max_flush_operations: usize,
    pub cache_entry_limit: u64,
    pub single_part_limit_bytes: u64,
    pub max_object_bytes: u64,
    pub negative_entries_per_directory: usize,
    pub max_parallel_parts_per_file: usize,
    pub max_upload_requests: usize,
    pub max_copy_requests: usize,
    pub max_queued_upload_jobs: usize,
    pub max_metadata_resolution_jobs: usize,
    pub metadata_resolution_jobs_in_use: usize,
}

type DiskStartup = Shared<BoxFuture<'static, std::result::Result<(), Arc<str>>>>;

struct DiskTier {
    namespace: Option<[u8; 32]>,
    bypass: Option<crate::backend::cache_scope::CacheBypass>,
    status: Option<crate::disk_cache::DiskCacheStatus>,
    cache: Option<crate::disk_cache::DiskCache>,
    attaching: bool,
    deferred: Option<crate::disk_cache::FundedPreparedDiskCache>,
    startup: Option<DiskStartup>,
    closing: bool,
}

impl Drop for DiskTier {
    fn drop(&mut self) {
        if let Some(deferred) = self.deferred.take() {
            deferred.close();
        }
    }
}

pub fn not_found(error: &Error) -> bool {
    matches!(error, Error::Fs(code) if *code == libc::ENOENT)
        || matches!(error, Error::Backend { code,.. } if matches!(code.as_str(), "NotFound"|"NoSuchKey"|"404"))
}

impl Filesystem {
    fn lock_namespace(&self) -> metrics::NamespaceGuard<'_, Namespace> {
        metrics::namespace_lock(&self.namespace, &self.diagnostics)
    }

    fn again(&self, reason: &'static str) -> Error {
        self.diagnostics.event(reason);
        if let Some(trace) = metrics::context() {
            trace.again(reason);
        }
        Error::Fs(libc::EAGAIN)
    }
    /// Constant-time observations; no inode walk, storage I/O or cache refresh.
    pub(crate) fn metrics_snapshot(&self) -> crate::metrics::Snapshot {
        let namespace = self.namespace.try_lock().ok().map(|ns| {
            (
                ns.nodes.len(),
                ns.names.len(),
                ns.evictions,
                ns.capacity_failures,
            )
        });
        let cache = self.cache.try_stats();
        crate::metrics::Snapshot {
            namespace,
            cache,
            entry_limit: self.config.entry_limit,
            lanes: [
                ("data", self.data.used(), self.data.limit()),
                ("ingress", self.ingress.used(), self.ingress.limit()),
                ("flush", self.flush_lane.used(), self.flush_lane.limit()),
            ],
            reads: self.read_metrics.clone(),
        }
    }

    pub fn effective_budgets(&self) -> EffectiveBudgets {
        let mut window = self
            .data
            .limit()
            .saturating_sub(self.disk_funding_bytes)
            .saturating_sub(MAX_IO)
            .min(u32::MAX as usize - MAX_IO);
        if self.disk_funding_bytes > 0 {
            // Source/disk output plus caller-funded queue bookkeeping, including
            // alignment at both ends. Reserve the fixed gap-pipeline pool from
            // this same data ceiling, not by increasing payload or L2 funding.
            let block = read::READ_BLOCK_BYTES;
            let wait = crate::disk_cache::READ_WAIT_BYTES;
            let without_pipeline =
                window.saturating_sub(2 * (block + wait)) / (2 * block + wait) * block;
            window = window
                .saturating_sub(
                    self.disk_gap_pipeline_bytes(without_pipeline.saturating_add(2 * block)),
                )
                .saturating_sub(2 * (block + wait))
                / (2 * block + wait)
                * block;
        }
        EffectiveBudgets {
            payload_limit_bytes: self.ingress.limit() + self.data.limit() + self.flush_lane.limit(),
            data_shared_bytes: self.data.limit(),
            disk_cache_funding_bytes: self.disk_funding_bytes,
            read_cache_policy_bytes: self.data.limit() + self.flush_lane.limit()
                - self.disk_funding_bytes,
            read_cache_loaned_bytes: self.cache.loaned_bytes(),
            ingress_bytes: self.ingress.limit(),
            flush_bytes: self.flush_lane.limit(),
            data_used_bytes: self.data.used(),
            ingress_used_bytes: self.ingress.used(),
            flush_used_bytes: self.flush_lane.used(),
            requested_read_ahead_bytes: self.config.read_ahead * 1024,
            requested_small_window_bytes: self.config.read_ahead_small * 1024,
            requested_large_window_bytes: self.config.read_ahead_large * 1024,
            max_effective_window_bytes: window,
            max_overlay_window_bytes: window / 2,
            read_parallel_chunk_bytes: (self.config.read_ahead_parallel * 1024)
                .max(if self.disk_funding_bytes > 0 {
                    read::READ_BLOCK_BYTES
                } else {
                    1
                })
                .min(window),
            upload_chunk_bytes: crate::upload_body::CHUNK_SIZE,
            max_read_tasks_per_window: self.config.max_requests,
            max_request_tasks: self.config.max_requests,
            max_flush_operations: self.config.max_flushers,
            cache_entry_limit: self.config.entry_limit,
            single_part_limit_bytes: self.config.single_part.max(5) * 1024 * 1024,
            max_object_bytes: self.config.part_sizes.max_size(),
            negative_entries_per_directory: self.config.negative_lookup_cache_size,
            max_parallel_parts_per_file: self.config.max_parallel_parts,
            max_upload_requests: self.config.max_flushers,
            max_copy_requests: self.config.max_parallel_copy,
            max_queued_upload_jobs: self.config.max_requests,
            max_metadata_resolution_jobs: self.config.max_requests,
            metadata_resolution_jobs_in_use: self.config.max_requests
                - self.metadata_slots.available_permits(),
        }
    }

    fn disk_gap_pipeline_bytes(&self, coverage: usize) -> usize {
        // L2 chunks end on block boundaries and are at least one block apart.
        // A small window cannot instantiate every configured request slot.
        self.config
            .max_requests
            .min(coverage.div_ceil(read::READ_BLOCK_BYTES))
            * read::GAP_PIPELINE_BYTES
    }

    pub fn metadata_usage(&self) -> crate::metadata_budget::MetadataUsage {
        self.metadata.usage()
    }

    pub fn disk_cache_stats(&self) -> Option<crate::disk_cache::Stats> {
        self.disk
            .lock()
            .unwrap()
            .cache
            .as_ref()
            .map(|cache| cache.stats())
    }

    /// Validated L2 bytes delivered into the read pipeline, including read-ahead.
    pub fn disk_cache_read_bytes(&self) -> u64 {
        self.disk_read_bytes.load(Ordering::Relaxed)
    }

    /// Configured deduction from L1, including during recovery/bypass. Actual
    /// ownership is also visible in the shared data lane's used-byte counter.
    pub fn disk_cache_funding(&self) -> Option<crate::disk_cache::Funding> {
        if self.disk_funding_bytes == 0 {
            return None;
        }
        crate::disk_cache::Funding::derive(self.config.disk_cache_max_size?, self.data.limit()).ok()
    }

    pub fn disk_cache_state(&self) -> Option<crate::disk_cache::State> {
        let disk = self.disk.lock().unwrap();
        self.config.disk_cache_dir.as_ref()?;
        Some(disk.status.as_ref().map_or(
            if disk.bypass.is_some() {
                crate::disk_cache::State::Bypass
            } else {
                crate::disk_cache::State::Prepared
            },
            |status| status.state(),
        ))
    }

    pub fn disk_cache_bypass_reason(&self) -> Option<crate::backend::cache_scope::CacheBypass> {
        self.disk.lock().unwrap().bypass
    }

    /// Explicit startup step: prepare/lock the directory before backend startup,
    /// then attach only after the backend has established its persistent scope.
    /// Recovery is owned independently of caller cancellation; requests fall back
    /// while it runs, and a concurrent shutdown closes rather than installs it.
    pub async fn attach_disk_cache(
        self: &Arc<Self>,
        prepared: crate::disk_cache::PreparedDiskCache,
    ) -> Result<()> {
        let startup = {
            let mut disk = self.disk.lock().unwrap();
            if self.config.disk_cache_dir.is_none() || disk.attaching || self.is_closing() {
                return Err(Error::Config(
                    "invalid or repeated disk cache attachment".into(),
                ));
            }
            disk.attaching = true;
            if let Some(reason) = disk.bypass {
                eprintln!("disk cache bypass: {reason:?}");
                return Ok(());
            }
            let funding = prepared.funding(self.data.limit())?;
            if funding.required_bytes != self.disk_funding_bytes
                || Some(funding.capacity as u64) != self.config.disk_cache_max_size
            {
                return Err(Error::Config(
                    "disk cache preparation does not match configuration".into(),
                ));
            }
            disk.status = Some(prepared.status());
            let funded = prepared.fund(&self.data)?;
            if !funded.has_inventory() {
                disk.deferred = Some(funded);
                return Ok(());
            }
            self.start_disk_cache(&mut disk, funded)
        };
        startup
            .await
            .map_err(|error| Error::Config(error.to_string()))
    }

    fn start_disk_cache(
        &self,
        disk: &mut DiskTier,
        funded: crate::disk_cache::FundedPreparedDiskCache,
    ) -> DiskStartup {
        let owner = self.disk.clone();
        let deadline = self.config.request_timeout;
        // Spawn now, not when a waiter polls the shared completion. No victim,
        // loan or payload backing is retained by this single startup owner.
        let task = tokio::spawn(async move {
            let cache = crate::disk_cache::DiskCache::open_funded(funded)
                .await
                .map_err(|error| Arc::<str>::from(error.to_string()))?;
            let closing = {
                let mut disk = owner.lock().unwrap();
                if disk.closing {
                    true
                } else {
                    disk.cache = Some(cache.clone());
                    false
                }
            };
            if closing {
                cache
                    .close(deadline)
                    .await
                    .map_err(|error| Arc::<str>::from(error.to_string()))?;
            }
            Ok(())
        });
        let startup = async move {
            task.await
                .map_err(|error| Arc::<str>::from(error.to_string()))?
        }
        .boxed()
        .shared();
        disk.startup = Some(startup.clone());
        startup
    }

    fn start_disk_for_capacity(&self, node: &Inode, generation: u64) {
        let deferred = self.disk.lock().unwrap().deferred.is_some();
        if !deferred || !self.disk_eligible(node, generation) {
            return;
        }
        let mut disk = self.disk.lock().unwrap();
        if !disk.closing
            && let Some(funded) = disk.deferred.take()
        {
            drop(self.start_disk_cache(&mut disk, funded));
        }
    }

    pub async fn drain_disk_cache(&self, deadline: Duration) -> Result<()> {
        let startup = self.disk.lock().unwrap().startup.clone();
        tokio::time::timeout(deadline, async {
            if let Some(startup) = startup {
                startup
                    .await
                    .map_err(|error| Error::Config(error.to_string()))?;
            }
            let cache = self.disk.lock().unwrap().cache.clone();
            if let Some(cache) = cache {
                cache.drain(deadline).await?;
            }
            Ok(())
        })
        .await
        .map_err(|_| Error::Timeout("L2 drain"))?
    }

    fn disk_read(&self, node: &Inode, generation: u64) -> Option<read::DiskRead> {
        let (cache, namespace) = {
            let disk = self.disk.lock().unwrap();
            (disk.cache.clone()?, disk.namespace?)
        };
        self.disk_eligible(node, generation)
            .then_some(read::DiskRead { cache, namespace })
    }

    fn disk_eligible(&self, node: &Inode, generation: u64) -> bool {
        self.check_disk_eligible(node, generation).is_ok()
    }

    pub fn disk_read_decisions(&self) -> [u64; 16] {
        std::array::from_fn(|index| self.disk_decisions[index].load(Ordering::Relaxed))
    }

    fn record_disk_decision(&self, reason: read::DiskDecision) {
        self.disk_decisions[reason as usize].fetch_add(1, Ordering::Relaxed);
        if let Some(trace) = metrics::context() {
            trace.debug(
                "l2_read_eligibility",
                || serde_json::json!({"reason": reason.label()}),
            );
        }
    }

    async fn disk_read_window(
        &self,
        node: Arc<Inode>,
        generation: u64,
        snapshot: Arc<write::Snapshot>,
        charge: &mut crate::budget::Reservation,
        deadline: tokio::time::Instant,
    ) -> Option<read::DiskRead> {
        if self.is_closing() {
            self.record_disk_decision(read::DiskDecision::Closing);
            return None;
        }
        let (cache, namespace) = {
            let disk = self.disk.lock().unwrap();
            let Some(cache) = disk.cache.clone() else {
                self.record_disk_decision(read::DiskDecision::Unavailable);
                return None;
            };
            let Some(namespace) = disk.namespace else {
                self.record_disk_decision(read::DiskDecision::NamespaceIdentity);
                return None;
            };
            (cache, namespace)
        };
        // Borrow one existing per-block bookkeeping credit for qualification.
        // Only four funded read-work owners can enter the blocking executor;
        // the remaining windows wait key-free on the existing FIFO semaphore.
        let slot = match cache
            .read_admission(
                charge.split(crate::disk_cache::READ_WAIT_BYTES),
                deadline.saturating_duration_since(tokio::time::Instant::now()),
            )
            .await
        {
            Ok(slot) => slot,
            Err(reason) => {
                self.record_disk_decision(read::DiskDecision::Admission);
                read::debug_disk_fallback(&cache, reason);
                return None;
            }
        };
        let ns = self.namespace.clone();
        let config = self.config.clone();
        let counts = self.disk_decisions.clone();
        let trace = metrics::context();
        let qualified = slot
            .qualify(move || {
                let record = |reason: read::DiskDecision| {
                    counts[reason as usize].fetch_add(1, Ordering::Relaxed);
                    if let Some(trace) = &trace {
                        trace.debug(
                            "l2_read_eligibility",
                            || serde_json::json!({"reason": reason.label()}),
                        );
                    }
                };
                // No namespace guard is held while waiting for an inode, and no
                // guard escapes into disk/source I/O. The canonical snapshot remains
                // charged to its original metadata owner; no identity Vec is made.
                let check_namespace = || {
                    let ns = match ns.try_lock() {
                        Ok(ns) => ns,
                        Err(std::sync::TryLockError::WouldBlock) => {
                            record(read::DiskDecision::NamespaceContended);
                            ns.lock().map_err(|_| read::DiskDecision::LockPoisoned)?
                        }
                        Err(_) => return Err(read::DiskDecision::LockPoisoned),
                    };
                    Self::check_disk_namespace(&ns, &snapshot.key)
                };
                check_namespace()?;
                let state = match node.state.try_lock() {
                    Ok(state) => state,
                    Err(std::sync::TryLockError::WouldBlock) => {
                        record(read::DiskDecision::InodeContended);
                        node.state
                            .lock()
                            .map_err(|_| read::DiskDecision::LockPoisoned)?
                    }
                    Err(_) => return Err(read::DiskDecision::LockPoisoned),
                };
                Self::check_disk_state(&config, &state, generation)?;
                if state.key != snapshot.key || state.size != snapshot.size {
                    return Err(read::DiskDecision::Generation);
                }
                drop(state);
                // A parent MoveLease can begin while the child inode is held.
                // Revalidate names after that wait, without nesting the locks.
                check_namespace()
            })
            .await;
        match qualified {
            Ok((slot, decision)) => {
                charge.merge(slot.release_credit());
                self.record_disk_decision(match decision {
                    Ok(()) => read::DiskDecision::Eligible,
                    Err(reason) => reason,
                });
                decision.ok().map(|()| read::DiskRead { cache, namespace })
            }
            Err(reason) => {
                // Actual blocked work retains its credit. Source output was
                // independently admitted and does not reclaim that live owner.
                self.record_disk_decision(read::DiskDecision::Admission);
                read::debug_disk_fallback(&cache, reason);
                None
            }
        }
    }

    fn check_disk_eligible(
        &self,
        node: &Inode,
        generation: u64,
    ) -> std::result::Result<(), read::DiskDecision> {
        if self.is_closing() {
            return Err(read::DiskDecision::Closing);
        }
        // Optional work never waits for namespace/inode ownership. In particular
        // capacity eviction cannot hold up a FIFO reader or a writer's loan.
        let ns = self
            .namespace
            .try_lock()
            .map_err(|_| read::DiskDecision::NamespaceContended)?;
        let state = node
            .state
            .try_lock()
            .map_err(|_| read::DiskDecision::InodeContended)?;
        Self::check_disk_state(&self.config, &state, generation)?;
        Self::check_disk_namespace(&ns, &state.key)
    }

    fn check_disk_state(
        config: &Config,
        state: &State,
        generation: u64,
    ) -> std::result::Result<(), read::DiskDecision> {
        if state.deleted {
            return Err(read::DiskDecision::Deleted);
        }
        if state.version != state.durable
            || !state.extents.is_empty()
            || state.visible_remote_size < state.size
        {
            return Err(read::DiskDecision::Dirty);
        }
        if state.pending_upload.is_some()
            || state.rename_target.is_some()
            || state.data_key.is_some()
        {
            return Err(read::DiskDecision::Pending);
        }
        if file_type(state.mode) != FileType::RegularFile {
            return Err(read::DiskDecision::NotRegular);
        }
        if state
            .remote
            .as_ref()
            .is_none_or(|head| head.size != state.size || head.etag.is_empty())
        {
            return Err(read::DiskDecision::RemoteIdentity);
        }
        if read::generation(state.version, state.remote.as_ref()) != generation {
            return Err(read::DiskDecision::Generation);
        }
        if !config
            .disk_cache_regex
            .as_ref()
            .is_some_and(|regex| regex.matches(&format!("/{}", state.key)))
        {
            return Err(read::DiskDecision::Excluded);
        }
        Ok(())
    }

    fn check_disk_namespace(
        ns: &Namespace,
        key: &str,
    ) -> std::result::Result<(), read::DiskDecision> {
        if key
            .match_indices('/')
            .map(|(end, _)| end)
            .chain(std::iter::once(key.len()))
            .any(|end| {
                let prefix = &key[..end];
                ns.moving.contains(prefix)
                    || ns.pending_moves.contains_key(prefix)
                    || ns.pending_deletes.contains_key(prefix)
                    || ns.tombstones.contains_key(prefix)
            })
        {
            return Err(read::DiskDecision::Pending);
        }
        Ok(())
    }

    fn demote_capacity(&self, victim: crate::cache::CapacityVictim) {
        if victim.block.is_loaned() {
            return;
        }
        let Ok(node) = self.node(victim.inode) else {
            return;
        };
        self.start_disk_for_capacity(&node, victim.generation);
        let Some(disk) = self.disk_read(&node, victim.generation) else {
            return;
        };
        let mut relative = 0;
        while relative < victim.block.as_slice().len() {
            let at = victim.offset + relative as u64;
            let key = {
                let Ok(state) = node.state.try_lock() else {
                    return;
                };
                if state.deleted
                    || state.version != state.durable
                    || state.rename_target.is_some()
                    || read::generation(state.version, state.remote.as_ref()) != victim.generation
                {
                    return;
                }
                let Some(head) = &state.remote else {
                    return;
                };
                let amount = (read::block_end(at, state.size).saturating_sub(at) as usize)
                    .min(victim.block.as_slice().len() - relative);
                crate::disk_cache::DiskKey::new(
                    &disk.namespace,
                    state.key.as_bytes(),
                    head.etag.as_bytes(),
                    state.size,
                    at,
                    amount,
                )
                .ok()
            };
            let Some(key) = key else {
                return;
            };
            let end = relative + key.covered_len();
            // No cache/inode/namespace lock and no await while copying. Adapter
            // owns only its admitted visible copy, never this full L1 backing.
            disk.cache
                .try_demote(key, victim.block.slice(relative..end));
            relative = end;
        }
    }

    fn evict_cache(&self) -> bool {
        let Some(victim) = self.cache.capacity_victim() else {
            return false;
        };
        self.demote_capacity(victim);
        true
    }

    fn touch_disk_hit(&self, node: &Inode, generation: u64, offset: u64, length: usize) {
        let Some(disk) = self.disk_read(node, generation) else {
            return;
        };
        let mut next = offset;
        while next < offset.saturating_add(length as u64) {
            let Some((start, len)) = self.cache.span_at(node.ino, generation, next) else {
                break;
            };
            let block_start = next / read::READ_BLOCK_BYTES as u64 * read::READ_BLOCK_BYTES as u64;
            let at = block_start.max(start);
            let (full, fragment) = {
                let Ok(state) = node.state.try_lock() else {
                    return;
                };
                let Some(head) = &state.remote else {
                    return;
                };
                if read::generation(state.version, state.remote.as_ref()) != generation {
                    return;
                }
                let end = read::block_end(at, state.size).min(start.saturating_add(len as u64));
                let fragment = crate::disk_cache::DiskKey::new(
                    &disk.namespace,
                    state.key.as_bytes(),
                    head.etag.as_bytes(),
                    state.size,
                    at,
                    (end - at) as usize,
                )
                .ok();
                let full_end = read::block_end(block_start, state.size);
                let full = (at != block_start || end != full_end)
                    .then(|| {
                        crate::disk_cache::DiskKey::new(
                            &disk.namespace,
                            state.key.as_bytes(),
                            head.etag.as_bytes(),
                            state.size,
                            block_start,
                            (full_end - block_start) as usize,
                        )
                        .ok()
                    })
                    .flatten();
                (full, fragment)
            };
            let Some(key) = fragment else {
                return;
            };
            next = key.offset() + key.covered_len() as u64;
            if !full
                .as_ref()
                .is_some_and(|full| disk.cache.touch_validated(full))
            {
                disk.cache.touch_validated(&key);
            }
        }
    }
    pub(crate) fn charge_metadata(
        &self,
        bytes: usize,
    ) -> Result<crate::metadata_budget::MetadataCharge> {
        self.metadata.charge(bytes)
    }
    pub(crate) fn metadata_enabled(&self) -> bool {
        self.metadata.enabled()
    }

    async fn admit_metadata(
        &self,
        bytes: usize,
        new_inode: bool,
    ) -> Result<crate::metadata_budget::MetadataCharge> {
        let mut cleared = false;
        let deadline = Instant::now() + self.config.request_timeout;
        loop {
            if Instant::now() >= deadline {
                return Err(Error::Timeout("metadata admission"));
            }
            if (!new_inode || !self.lock_namespace().count_full())
                && let Ok(charge) = self.metadata.charge(bytes)
            {
                return Ok(charge);
            }
            if !cleared && self.metadata.usage().limit.is_some() {
                self.directory_pages.invalidate_all();
                self.subtree_pages.invalidate_all();
                self.preloaded_directories.invalidate_all();
                self.directory_pages.run_pending_tasks();
                self.subtree_pages.run_pending_tasks();
                self.preloaded_directories.run_pending_tasks();
                cleared = true;
                continue;
            }
            let reclaimed = {
                let mut ns = self.lock_namespace();
                let reclaimed = ns.reclaim_batch();
                if matches!(reclaimed, crate::namespace::Reclaim::Exhausted)
                    && new_inode
                    && ns.count_full()
                {
                    ns.capacity_failures += 1;
                }
                reclaimed
            };
            match reclaimed {
                crate::namespace::Reclaim::Exhausted => return Err(Error::Fs(libc::ENOMEM)),
                crate::namespace::Reclaim::More | crate::namespace::Reclaim::Removed => {
                    tokio::task::yield_now().await
                }
            }
        }
    }
    // A failed/cancelled directory move may already have drained source keys.
    // Keep mutators from resurrecting them until that bounded move is retried.
    fn pending_rename(ns: &Namespace, key: &str, except: Option<u64>) -> bool {
        key.match_indices('/')
            .map(|(end, _)| end)
            .chain(std::iter::once(key.len()))
            .any(|end| {
                ns.pending_moves
                    .get(&key[..end])
                    .is_some_and(|ino| Some(*ino) != except)
            })
    }
    pub fn new(config: Config, store: Arc<dyn Store>) -> Result<Arc<Self>> {
        config.validate()?;
        let PayloadBudgets {
            data_bytes,
            ingress_bytes,
            flush_bytes,
        } = config.payload_budgets()?;
        let disk_config = config.disk_cache_config()?;
        let scope = if disk_config.is_some() {
            Some(store.persistent_cache_scope()?)
        } else {
            None
        };
        let disk_namespace = scope.and_then(|scope| scope.namespace());
        let bypass = match scope {
            Some(crate::backend::cache_scope::CacheScope::Bypass(reason)) => Some(reason),
            _ => None,
        };
        let disk_funding_bytes = if disk_namespace.is_some() {
            crate::disk_cache::Funding::derive(disk_config.as_ref().unwrap().max_bytes, data_bytes)?
                .required_bytes
        } else {
            0
        };
        let config = Arc::new(config);
        let metadata = crate::metadata_budget::MetadataBudget::new(config.metadata_limit)?;
        let root = Inode::try_new(1, "".into(), true, None, &config, &metadata)?;
        root.state.lock().unwrap().lookups = 1;
        let namespace = Namespace::new(root, config.entry_limit as usize);
        let diagnostics = store
            .diagnostics()
            .filter(crate::metrics::Diagnostics::enabled)
            .unwrap_or_else(|| {
                crate::metrics::Diagnostics::new(
                    config.metrics_listen.is_some() && !config.no_diagnostics,
                    config.diagnostic_log_level,
                )
            });
        Ok(Arc::new(Self {
            inodes: namespace.inodes.clone(),
            namespace: Arc::new(Mutex::new(namespace)),
            ingress: Budget::new(ingress_bytes)?,
            metadata,
            data: Budget::new(data_bytes)?,
            disk: Arc::new(Mutex::new(DiskTier {
                namespace: disk_namespace,
                bypass,
                status: None,
                cache: None,
                attaching: false,
                deferred: None,
                startup: None,
                closing: false,
            })),
            disk_funding_bytes,
            disk_read_bytes: std::sync::atomic::AtomicU64::new(0),
            disk_decisions: Arc::new(std::array::from_fn(|_| {
                std::sync::atomic::AtomicU64::new(0)
            })),
            cache: crate::cache::ReadCache::with_entry_limit(
                (data_bytes + flush_bytes - disk_funding_bytes) as u64,
                config.entry_limit as usize,
            ),
            cache_published: Notify::new(),
            flush_lane: Budget::new(flush_bytes)?,
            flush_activity: Arc::new(RwLock::new(())),
            flushers: Arc::new(Semaphore::new(config.max_flushers)),
            upload_admission: write::UploadAdmission::new(&config),
            metadata_slots: Arc::new(Semaphore::new(config.max_requests)),
            directory_pages: moka::sync::Cache::builder()
                .max_capacity(config.entry_limit)
                .weigher(|_, page: &Arc<DirectoryPage>| page.entries.len().max(1) as u32)
                .build(),
            subtree_pages: moka::sync::Cache::builder()
                .max_capacity(config.entry_limit)
                .weigher(|_, page: &Arc<SubtreePage>| page.objects.len().max(1) as u32)
                .time_to_live(config.stat_cache_ttl.max(Duration::from_nanos(1)))
                .build(),
            preloaded_directories: moka::sync::Cache::new(config.entry_limit),
            closing: AtomicBool::new(false),
            read_metrics: crate::metrics::ReadMetrics::default(),
            diagnostics,
            config,
            store,
        }))
    }

    pub fn node(&self, ino: u64) -> Result<Arc<Inode>> {
        self.inodes
            .pin()
            .get(&ino)
            .and_then(std::sync::Weak::upgrade)
            .ok_or(Error::Fs(libc::ENOENT))
    }

    pub fn check_write(&self) -> Result<()> {
        if self.config.mount_options.iter().any(|v| v == "ro") {
            return Err(Error::Fs(libc::EROFS));
        }
        if self.closing.load(Ordering::Acquire) {
            return Err(Error::Cancelled);
        }
        Ok(())
    }

    fn child_key(&self, parent: u64, name: &str) -> Result<String> {
        let parent = self.node(parent)?;
        let state = parent.state.lock().unwrap();
        if state.deleted {
            return Err(Error::Fs(libc::ENOENT));
        }
        if file_type(state.mode) != FileType::Directory {
            return Err(Error::Fs(libc::ENOTDIR));
        }
        child(&state.key, name)
    }

    pub async fn lookup(&self, parent: u64, name: &str) -> Result<Arc<Inode>> {
        let started = std::time::Instant::now();
        let key = self.child_key(parent, name)?;
        if self.config.effective_list_type() == crate::config::ListType::Extended
            && !self.config.stat_cache_ttl.is_zero()
        {
            let preload = {
                let mut ns = self.lock_namespace();
                let negative = ns
                    .get(parent)?
                    .state
                    .lock()
                    .unwrap()
                    .negative_lookups
                    .get(name)
                    .is_some_and(|fetched| fetched.elapsed() < self.config.stat_cache_ttl);
                !ns.is_moving(&key)
                    && !ns.is_deleting(&key)
                    && !ns.hidden(&key)
                    && ns.named(&key).is_none()
                    && !negative
            };
            if preload {
                // Speculation is optional. An absent/failed/standard-metadata
                // page never replaces the normal authoritative lookup below.
                let _ =
                    tokio::time::timeout(self.config.http_timeout, self.preload_directory(parent))
                        .await;
            }
        }
        let (epoch, observed) = {
            let mut ns = self.lock_namespace();
            if ns.is_moving(&key) {
                return Err(self.again("lookup_moving"));
            }
            if ns.is_deleting(&key) {
                return Err(self.again("lookup_deleting"));
            }
            if ns.hidden(&key) {
                return Err(Error::Fs(libc::ENOENT));
            }
            if let Some(node) = ns.named(&key) {
                let state = node.state.lock().unwrap();
                if state.rename_target.is_some()
                    || state.data_key.is_some()
                    || state.version > state.durable
                    || state.metadata_known && state.fetched.elapsed() < self.config.stat_cache_ttl
                    || file_type(state.mode) == FileType::Directory
                        && ns.has_dirty_descendants(&key)
                {
                    drop(state);
                    return Ok(node);
                }
            }
            if self.config.negative_lookup_cache_size > 0 {
                let parent = ns.get(parent)?;
                let mut state = parent.state.lock().unwrap();
                if state
                    .negative_lookups
                    .get(name)
                    .is_some_and(|when| when.elapsed() < self.config.stat_cache_ttl)
                {
                    return Err(Error::Fs(libc::ENOENT));
                }
                state.negative_lookups.remove(name);
            }
            (
                ns.epoch,
                ns.named(&key).map(|node| {
                    let s = node.state.lock().unwrap();
                    (
                        node.ino,
                        s.version,
                        s.remote.as_ref().map(|h| h.etag.clone()),
                    )
                }),
            )
        };
        let (directory, head) = match self.store.head(&key).await {
            Ok(head) => (false, Some(head)),
            Err(error) if not_found(&error) => {
                let marker = if self.config.no_dir_object {
                    None
                } else {
                    match self.store.head(&format!("{key}/")).await {
                        Ok(head) => Some(head),
                        Err(error) if not_found(&error) => None,
                        Err(error) => return Err(error),
                    }
                };
                if let Some(head) = marker {
                    (true, Some(head))
                } else {
                    if self.config.no_implicit_dir {
                        return Err(self.missing(parent, name, &key, epoch, started, &observed)?);
                    }
                    let page = self.store.list(&format!("{key}/"), Some("/"), None).await?;
                    if page.objects.is_empty() && page.common_prefixes.is_empty() {
                        return Err(self.missing(parent, name, &key, epoch, started, &observed)?);
                    }
                    (true, None)
                }
            }
            Err(error) => return Err(error),
        };
        let charge = self
            .admit_metadata(
                if self.metadata.enabled() {
                    Inode::metadata_estimate(&key, head.as_ref())
                } else {
                    0
                },
                observed.is_none(),
            )
            .await?;
        let mut ns = self.lock_namespace();
        {
            let parent = ns.get(parent)?;
            let state = parent.state.lock().unwrap();
            if state.deleted || child(&state.key, name)? != key {
                return Err(self.again("lookup_parent_changed"));
            }
        }
        if ns.is_moving(&key) || ns.is_deleting(&key) || ns.hidden(&key) {
            return Err(self.again("lookup_binding_busy"));
        }
        if let Some(node) = ns.named(&key) {
            let mut fresh =
                Inode::new_admitted(node.ino, key.clone(), directory, head, &self.config, charge)?
                    .state
                    .into_inner()
                    .unwrap();
            fresh.fetched = started;
            let mut state = node.state.lock().unwrap();
            if observed.as_ref()
                != Some(&(
                    node.ino,
                    state.version,
                    state.remote.as_ref().map(|h| h.etag.clone()),
                ))
            {
                return Err(self.again("lookup_generation_changed"));
            }
            if state.can_replace_metadata(started) {
                state.replace_metadata(fresh)?;
            }
            drop(state);
            return Ok(node);
        }
        if observed.is_some() {
            return Err(self.again("lookup_binding_removed"));
        }
        ns.try_install(&key, |ino| {
            let node =
                Inode::new_admitted(ino, key.clone(), directory, head, &self.config, charge)?;
            node.state.lock().unwrap().fetched = started;
            Ok(node)
        })
    }

    async fn preload_directory(&self, parent: u64) -> Result<()> {
        let node = self.node(parent)?;
        let _gate = node.preload.lock().await;
        let (prefix, epoch) = {
            let ns = self.lock_namespace();
            if self
                .preloaded_directories
                .get(&parent)
                .is_some_and(|fetched| fetched.0.elapsed() < self.config.stat_cache_ttl)
            {
                return Ok(());
            }
            let state = node.state.lock().unwrap();
            if state.deleted || ns.is_moving(&state.key) || ns.is_deleting(&state.key) {
                return Err(self.again("preload_binding_busy"));
            }
            let prefix = if state.key.is_empty() {
                String::new()
            } else {
                format!("{}/", state.key)
            };
            (prefix, ns.epoch)
        };
        let fetched = std::time::Instant::now();
        let page = self.store.list(&prefix, Some("/"), None).await;
        let mut ns = self.lock_namespace();
        if ns.epoch != epoch {
            return Err(self.again("preload_epoch_changed"));
        }
        // This records an attempt, not an authoritative negative listing.
        if let Ok(charge) = self
            .metadata
            .charge(std::mem::size_of::<(u64, Instant)>() + crate::metadata_budget::LINKS)
        {
            self.preloaded_directories
                .insert(parent, Arc::new((fetched, charge)));
        }
        ns.hidden("");
        for object in page?
            .objects
            .into_iter()
            .take(crate::backend::extended_list::MAX_PAGE_ENTRIES)
        {
            let Some(name) = object.key.strip_prefix(&prefix) else {
                return Err(Error::InvalidResponse("preload prefix escape".into()));
            };
            if name.is_empty()
                || name.contains('/')
                || ns.tombstones.contains_key(&object.key)
                || ns.is_moving(&object.key)
                || ns.is_deleting(&object.key)
                || ns.named(&object.key).is_some()
            {
                continue;
            }
            child(prefix.trim_end_matches('/'), name)?;
            if object.metadata.is_none() || object.etag.as_ref().is_none_or(|etag| etag.is_empty())
            {
                continue;
            }
            let (key, entry) = listed_object(object, fetched, &self.metadata)?;
            let charge = self
                .metadata
                .charge_with(|| Inode::metadata_estimate(&key, None))?;
            ns.try_install(&key, |ino| {
                let node = Inode::cached_admitted(
                    ino,
                    key.clone(),
                    false,
                    entry.head,
                    entry.last_modified,
                    &self.config,
                    charge,
                )?;
                node.state.lock().unwrap().fetched = fetched;
                Ok(node)
            })?;
        }
        Ok(())
    }

    pub fn forget(&self, ino: u64, count: u64) {
        if let Ok(node) = self.node(ino) {
            let mut s = node.state.lock().unwrap();
            s.lookups = s.lookups.saturating_sub(count);
        }
    }

    /// Refresh getattr/fstat by stable inode identity, never by a possibly
    /// replaced name. Root is synthetic; implicit directories are checked by
    /// LIST, not by assuming a marker exists. Dirty attributes remain local.
    pub async fn refresh_inode_if_expired(&self, ino: u64) -> Result<Arc<Inode>> {
        self.prepare_inode_refresh(ino).await?.await
    }

    /// Admit before creating network jobs, but never queue fresh attributes
    /// behind unrelated metadata I/O. The caller owns and cancels the future.
    pub(crate) async fn prepare_inode_refresh(
        &self,
        ino: u64,
    ) -> Result<impl Future<Output = Result<Arc<Inode>>> + Send + '_> {
        let started = std::time::Instant::now();
        let node = self.node(ino)?;
        let (key, directory, remote, version, epoch, list_only, snapshot_charge) = {
            let ns = self.lock_namespace();
            let s = node.state.lock().unwrap();
            if s.deleted {
                return Err(Error::Fs(libc::ENOENT));
            }
            if ino == 1 {
                return Ok(Either::Left(ready(Ok(node.clone()))));
            }
            if ns.is_moving(&s.key) || ns.is_deleting(&s.key) {
                return Err(self.again("refresh_binding_busy"));
            }
            if s.version > s.durable
                || s.rename_target.is_some()
                || s.data_key.is_some()
                || s.metadata_known && s.fetched.elapsed() < self.config.stat_cache_ttl
            {
                return Ok(Either::Left(ready(Ok(node.clone()))));
            }
            let directory = file_type(s.mode) == FileType::Directory;
            if directory && ns.has_dirty_descendants(&s.key) {
                return Ok(Either::Left(ready(Ok(node.clone()))));
            }
            let snapshot_charge = self
                .metadata
                .charge(s.key.len() + s.remote.as_ref().map_or(0, |head| head.etag.len()))?;
            (
                s.key.clone(),
                directory,
                s.remote.as_ref().map(|head| head.etag.clone()),
                s.version,
                ns.epoch,
                directory && self.config.no_dir_object,
                snapshot_charge,
            )
        };
        let permit = self.metadata_permit().await?;
        Ok(Either::Right(async move {
            let _permit = permit;
            let _snapshot_charge = snapshot_charge;
            // An implicit directory may acquire a marker with authoritative metadata.
            // Skip its HEAD only when directory objects are explicitly disabled.
            let fetched = if list_only {
                let page = self.store.list(&format!("{key}/"), Some("/"), None).await;
                match page {
                    Ok(page) if !page.objects.is_empty() || !page.common_prefixes.is_empty() => {
                        Ok(None)
                    }
                    Ok(_) => Err(Error::Fs(libc::ENOENT)),
                    Err(e) => Err(e),
                }
            } else {
                match self
                    .store
                    .head(&if directory {
                        format!("{key}/")
                    } else {
                        key.to_string()
                    })
                    .await
                {
                    Ok(head) => Ok(Some(head)),
                    Err(error)
                        if directory && not_found(&error) && !self.config.no_implicit_dir =>
                    {
                        match self.store.list(&format!("{key}/"), Some("/"), None).await {
                            Ok(page)
                                if !page.objects.is_empty() || !page.common_prefixes.is_empty() =>
                            {
                                Ok(None)
                            }
                            Ok(_) => Err(Error::Fs(libc::ENOENT)),
                            Err(e) => Err(e),
                        }
                    }
                    Err(e) => Err(e),
                }
            };
            let charge = if let Ok(head) = &fetched {
                Some(
                    self.admit_metadata(
                        if self.metadata.enabled() {
                            Inode::metadata_estimate(&key, head.as_ref())
                        } else {
                            0
                        },
                        false,
                    )
                    .await?,
                )
            } else {
                None
            };
            let mut ns = self.lock_namespace();
            let mut s = node.state.lock().unwrap();
            if s.deleted
                || s.key != key
                || ns.names.get(&key) != Some(&ino)
                || ns.is_moving(&key)
                || ns.is_deleting(&key)
            {
                return Err(self.again("refresh_binding_changed"));
            }
            if s.version != version
                || s.remote.as_ref().map(|h| &h.etag) != remote.as_ref()
                || !s.can_replace_metadata(started)
                || directory && ns.has_dirty_descendants(&key)
            {
                return Ok(node.clone());
            }
            if directory && ns.epoch != epoch && fetched.as_ref().is_err_and(not_found) {
                return Err(self.again("refresh_directory_changed"));
            }
            match fetched {
                Ok(head) => {
                    let mut fresh = Inode::new_admitted(
                        ino,
                        key.to_string(),
                        directory,
                        head,
                        &self.config,
                        charge.expect("successful refresh admission"),
                    )?
                    .state
                    .into_inner()
                    .unwrap();
                    fresh.fetched = started;
                    s.replace_metadata(fresh)?;
                }
                Err(error) if not_found(&error) => {
                    s.deleted = true;
                    ns.names.remove(&key);
                    ns.changed(&key);
                    return Err(error);
                }
                Err(error) => return Err(error),
            }
            drop(s);
            Ok(node)
        }))
    }
    fn missing(
        &self,
        parent: u64,
        name: &str,
        key: &str,
        epoch: u64,
        started: std::time::Instant,
        observed: &Option<(u64, u64, Option<String>)>,
    ) -> Result<Error> {
        let mut ns = self.lock_namespace();
        if ns.epoch != epoch {
            return Ok(self.again("missing_epoch_changed"));
        }
        if let Some(node) = ns.named(key) {
            let mut state = node.state.lock().unwrap();
            if state.version > state.durable
                || file_type(state.mode) == FileType::Directory && ns.has_dirty_descendants(key)
                || observed.as_ref()
                    != Some(&(
                        node.ino,
                        state.version,
                        state.remote.as_ref().map(|h| h.etag.clone()),
                    ))
            {
                return Ok(self.again("missing_generation_changed"));
            }
            state.deleted = true;
            drop(state);
            ns.names.remove(key);
            ns.changed(key);
        }
        if self.config.negative_lookup_cache_size > 0
            && started.elapsed() < self.config.stat_cache_ttl
        {
            let node = ns.get(parent)?;
            let mut state = node.state.lock().unwrap();
            state
                .negative_lookups
                .retain(|_, when| when.elapsed() < self.config.stat_cache_ttl);
            if state.negative_lookups.len() >= self.config.negative_lookup_cache_size
                && !state.negative_lookups.contains_key(name)
                && let Some(oldest) = state
                    .negative_lookups
                    .iter()
                    .min_by_key(|(_, when)| *when)
                    .map(|(name, _)| name.clone())
            {
                state.negative_lookups.remove(&oldest);
            }
            state.settle_metadata();
            let growth = name.len()
                + std::mem::size_of::<(String, std::time::Instant)>()
                + crate::metadata_budget::LINKS;
            // Negative caching is optional: lack of cache space must not turn
            // an authoritative ENOENT into an admission failure.
            if state.admit_metadata(growth).is_ok() {
                state.negative_lookups.insert(name.into(), started);
                state.settle_metadata();
            }
        }
        Ok(Error::Fs(libc::ENOENT))
    }

    pub fn reference(&self, node: &Inode) {
        node.state.lock().unwrap().lookups += 1;
    }

    pub fn parent_ino(&self, ino: u64) -> u64 {
        let Ok(node) = self.node(ino) else {
            return 1;
        };
        let key = node.state.lock().unwrap().key.clone();
        let parent = key.rsplit_once('/').map_or("", |(parent, _)| parent);
        self.lock_namespace()
            .named(parent)
            .map_or(1, |node| node.ino)
    }

    pub fn open(&self, ino: u64, writable: bool) -> Result<()> {
        if writable {
            self.check_write()?;
        }
        let node = self.node(ino)?;
        let mut state = node.state.lock().unwrap();
        if state.deleted {
            return Err(Error::Fs(libc::ENOENT));
        }
        if state.opens == 0 {
            *node.pattern.lock().unwrap() = read::Pattern::default();
        }
        state.opens += 1;
        Ok(())
    }

    pub fn release(&self, ino: u64) {
        if let Ok(node) = self.node(ino) {
            let mut state = node.state.lock().unwrap();
            state.opens = state.opens.saturating_sub(1);
        }
    }

    pub fn open_file(self: &Arc<Self>, ino: u64, writable: bool) -> Result<Arc<OpenFile>> {
        let node = self.node(ino)?;
        if node.attr().kind == FileType::Directory {
            return Err(Error::Fs(libc::EISDIR));
        }
        self.open(ino, writable)?;
        Ok(Arc::new(OpenFile {
            fs: self.clone(),
            node,
            pattern: Mutex::new(read::Pattern::default()),
        }))
    }

    pub async fn create(
        &self,
        parent: u64,
        name: &str,
        mode: u32,
        target: Option<&[u8]>,
        rdev: u32,
    ) -> Result<Arc<Inode>> {
        self.create_inner(parent, name, mode, target, rdev, None)
            .await
    }

    async fn create_inner(
        &self,
        parent: u64,
        name: &str,
        mode: u32,
        target: Option<&[u8]>,
        rdev: u32,
        identity: Option<(u32, u32)>,
    ) -> Result<Arc<Inode>> {
        self.check_write()?;
        let key = self.child_key(parent, name)?;
        match self.lookup(parent, name).await {
            Ok(_) => return Err(Error::Fs(libc::EEXIST)),
            Err(error) if not_found(&error) => {}
            Err(error) => return Err(error),
        }
        let directory = mode & libc::S_IFMT == libc::S_IFDIR;
        let charge = self
            .admit_metadata(Inode::metadata_estimate(&key, None), true)
            .await?;
        let mut candidate =
            Inode::new_admitted(0, key.clone(), directory, None, &self.config, charge)?;
        {
            let mut state = candidate.state.lock().unwrap();
            state.admit_metadata(target.map_or(0, |value| {
                value.len()
                    + self.config.symlink_attr.len()
                    + std::mem::size_of::<(String, Vec<u8>)>()
                    + crate::metadata_budget::LINKS
            }))?;
            state.mode = mode;
            state.rdev = rdev;
            state.version = 1;
            if self.config.enable_perms
                && let Some((uid, gid)) = identity
            {
                state.uid = uid;
                state.gid = gid;
                state.version += 1;
            }
            if let Some(target) = target {
                state
                    .metadata
                    .try_insert(&self.config.symlink_attr, target)?;
            }
            state.settle_metadata();
            self.prepare_metadata(&mut state)?;
        }
        let mut ns = self.lock_namespace();
        {
            let p = ns.get(parent)?;
            let s = p.state.lock().unwrap();
            if s.deleted
                || ns.names.get(&s.key) != Some(&parent)
                || s.rename_target.is_some()
                || child(&s.key, name)? != key
            {
                return Err(Error::Fs(libc::ENOENT));
            }
        }
        if ns.is_moving(&key) || ns.is_deleting(&key) || Self::pending_rename(&ns, &key, None) {
            return Err(self.again("create_binding_busy"));
        }
        if ns.named(&key).is_some() {
            return Err(Error::Fs(libc::EEXIST));
        }
        let node = ns.install(&key, |ino| {
            candidate.ino = ino;
            candidate
        })?;
        ns.remove_tombstone(&key);
        ns.changed(&key);
        ns.get(parent)?
            .state
            .lock()
            .unwrap()
            .negative_lookups
            .remove(name);
        {
            let state = node.state.lock().unwrap();
            ns.track_dirty(node.ino, &state);
        }
        Ok(node)
    }

    /// FUSE creation passes the request identity only with enable-perms, as the
    /// reference's CreateFile/MkDir/CreateSymlink SetAttributes calls do. There is
    /// no userspace setgid-parent inheritance in that contract.
    pub async fn create_with_identity(
        &self,
        parent: u64,
        name: &str,
        mode: u32,
        target: Option<&[u8]>,
        rdev: u32,
        identity: (u32, u32),
    ) -> Result<Arc<Inode>> {
        self.create_inner(parent, name, mode, target, rdev, Some(identity))
            .await
    }

    /// Publish the requested result while retaining the caller-owned load future.
    /// A returned error belongs to read-ahead after a successful publication.
    pub async fn read_reply(
        &self,
        ino: u64,
        offset: u64,
        length: usize,
        ready: impl FnOnce(Result<Block>) + Send,
    ) -> Option<Error> {
        self.read_reply_inner(ino, offset, length, false, None, ready)
            .await
    }

    /// Send bytes synchronously without allowing a reply to pin its GET owner.
    pub async fn read_reply_data(
        &self,
        ino: u64,
        offset: u64,
        length: usize,
        ready: impl FnOnce(Result<&[u8]>) + Send,
    ) -> Option<Error> {
        self.read_reply_data_with_pattern(ino, offset, length, None, ready)
            .await
    }

    async fn read_reply_data_with_pattern(
        &self,
        ino: u64,
        offset: u64,
        length: usize,
        pattern: Option<&Mutex<read::Pattern>>,
        ready: impl FnOnce(Result<&[u8]>) + Send,
    ) -> Option<Error> {
        self.read_reply_inner(
            ino,
            offset,
            length,
            true,
            pattern,
            move |result| match result {
                Ok(block) => ready(Ok(block.as_slice())),
                Err(error) => ready(Err(error)),
            },
        )
        .await
    }

    async fn read_reply_inner(
        &self,
        ino: u64,
        offset: u64,
        length: usize,
        borrow_reply: bool,
        pattern: Option<&Mutex<read::Pattern>>,
        ready: impl FnOnce(Result<Block>) + Send,
    ) -> Option<Error> {
        let mut ready = Some(ready);
        let result = self
            .read_inner(ino, offset, length, borrow_reply, pattern, &mut ready)
            .await;
        if let Some(ready) = ready {
            let cached_length = result
                .as_ref()
                .ok()
                .and_then(Option::as_ref)
                .map_or(0, |block| block.as_slice().len());
            ready(result.map(|block| block.expect("unpublished read owns its result")));
            if cached_length == 0 {
                None
            } else {
                self.refill_after_hit(ino, offset + cached_length as u64, pattern)
                    .await
                    .err()
            }
        } else {
            result.err()
        }
    }

    /// Keep optional refill in the same admitted, cancellable reply future. Ready
    /// bytes have already been published and no reply owner is held across admission.
    async fn refill_after_hit(
        &self,
        ino: u64,
        consumed: u64,
        pattern: Option<&Mutex<read::Pattern>>,
    ) -> Result<()> {
        let node = self.node(ino)?;
        let Some((size, generation, version)) = read::refill_state(&node, consumed) else {
            return Ok(());
        };
        let budgets = self.effective_budgets();
        let window = pattern
            .unwrap_or(&node.pattern)
            .lock()
            .unwrap()
            .window(&self.config)
            .min(budgets.max_effective_window_bytes);
        if window == 0 {
            return Ok(());
        }
        let Some(range) = self.refill_range(&node, generation, consumed, size, window) else {
            return Ok(());
        };
        let length = (range.end - range.start) as usize;
        let read::FillAdmission::Leader(fill) =
            read::start_fill(&node, generation, range.start, range.end)
        else {
            return Ok(());
        };
        // A concurrent fill may have published and relinquished its interval
        // between planning and ownership. Retry speculation on a later read.
        if self.cache.gap(ino, generation, range.clone()) != Some(range.clone()) {
            return Ok(());
        }
        let admission = length
            + if self.disk_funding_bytes > 0 {
                read::disk_window_admission(range.start, range.end, size)
                    + self.disk_gap_pipeline_bytes(read::disk_coverage(
                        range.start,
                        range.end,
                        size,
                    ))
            } else {
                0
            };
        let Some(charge) = self.try_refill_admission(ino, generation, consumed, admission) else {
            return Ok(());
        };
        let snapshot = read::snapshot(&node)?;
        if snapshot.version != version
            || read::generation(snapshot.version, snapshot.remote.as_ref()) != generation
            || !write::remote_only(&snapshot, range.start, length)
        {
            return Ok(());
        }
        let start = range.start;
        self.load_window(
            read::Window {
                snapshot: Arc::new(snapshot),
                range,
                charge,
                single_buffer: true,
                cache_key: Some((ino, generation)),
                fill,
            },
            start,
            0,
            false,
            &mut None::<fn(Result<Block>)>,
        )
        .await?;
        Ok(())
    }

    fn refill_range(
        &self,
        node: &Inode,
        generation: u64,
        consumed: u64,
        size: u64,
        window: usize,
    ) -> Option<std::ops::Range<u64>> {
        let trigger = consumed.saturating_add((window / 2) as u64).min(size - 1);
        if read::fill_covers(node, generation, trigger)
            || self.cache.contains(node.ino, generation, trigger)
        {
            return None;
        }
        let mut cursor = consumed;
        loop {
            let gap = self.cache.gap(node.ino, generation, cursor..size)?;
            cursor = gap.end;
            if let Some(range) = read::unfilled(node, generation, gap) {
                return Some(range.start..range.end.min(range.start.saturating_add(window as u64)));
            }
            if cursor == size {
                return None;
            }
        }
    }

    fn try_refill_admission(
        &self,
        ino: u64,
        generation: u64,
        consumed: u64,
        length: usize,
    ) -> Option<crate::budget::Reservation> {
        // No waiting, flushing, or bypassing FIFO for speculative bytes. Reclaim
        // old owners only while the still-needed current backing stays intact.
        let cache = &self.cache;
        loop {
            if let Ok(charge) = self.data.try_reserve(length) {
                return Some(charge);
            }
            let victim = cache.consumed_victim(ino, generation, consumed)?;
            self.demote_capacity(victim);
        }
    }

    pub async fn read(&self, ino: u64, offset: u64, length: usize) -> Result<Block> {
        Ok(self
            .read_inner(
                ino,
                offset,
                length,
                false,
                None,
                &mut None::<fn(Result<Block>)>,
            )
            .await?
            .expect("a read without early publication retains its result"))
    }

    async fn read_inner(
        &self,
        ino: u64,
        offset: u64,
        length: usize,
        borrow_reply: bool,
        pattern: Option<&Mutex<read::Pattern>>,
        ready: &mut Option<impl FnOnce(Result<Block>) + Send>,
    ) -> Result<Option<Block>> {
        if length > MAX_IO {
            return Err(Error::Fs(libc::EINVAL));
        }
        let node = self.node(ino)?;
        let (size, clean, generation, version, etag, mut single_buffer) = {
            let state = node.state.lock().unwrap();
            if state.deleted {
                return Err(Error::Fs(libc::ENOENT));
            }
            (
                state.size,
                state.version == state.durable,
                read::generation(state.version, state.remote.as_ref()),
                state.version,
                state.remote.as_ref().map(|remote| remote.etag.clone()),
                state.version == state.durable
                    && state.extents.is_empty()
                    && state.remote.is_some()
                    && state.visible_remote_size >= state.size,
            )
        };
        let length = length.min(size.saturating_sub(offset) as usize);
        // Track every nonempty caller read, including cache hits. Backend misses
        // are not the access stream: skipping hits turns contiguous kernel reads
        // into apparent seeks at the next read-ahead boundary.
        let window = if length > 0 {
            pattern
                .unwrap_or(&node.pattern)
                .lock()
                .unwrap()
                .prefetch(offset, length, &self.config)
        } else {
            0
        };
        // Participate in pressure reclamation before borrowing a cache block:
        // an earlier FIFO waiter may need bytes published after its eviction.
        // One-block hits share their allocation; fragment spans use this output.
        let reply = self.admit(length).await?;
        if !read::current(&node, generation) {
            return Err(self.again("read_generation_after_admission"));
        }
        if length == 0 {
            return reply.copy(&[]).map(Some);
        }
        let cached = {
            let timer = self.diagnostics.stage(Stage::CacheRead);
            let cached = self.cache.read(ino, generation, offset, length, reply);
            timer.ok();
            cached
        };
        self.diagnostics.debug("read_cache", || {
            serde_json::json!({
                "hit": cached.is_some(), "offset": offset, "bytes": length,
            })
        });
        if let Some(block) = cached {
            self.touch_disk_hit(&node, generation, offset, length);
            self.read_metrics.hits.inc();
            self.read_metrics.hit_bytes.inc_by(length as u64);
            return Ok(Some(block));
        }
        self.read_metrics.misses.inc();
        self.read_metrics.miss_bytes.inc_by(length as u64);
        // Miss admission reserves output and scratch atomically below.
        let budgets = self.effective_budgets();
        let mut start = if clean {
            offset / read::READ_BLOCK_BYTES as u64 * read::READ_BLOCK_BYTES as u64
        } else {
            offset
        };
        let window_limit = if single_buffer {
            budgets.max_effective_window_bytes
        } else {
            budgets.max_overlay_window_bytes
        };
        if offset - start + length as u64 > window_limit as u64 {
            start = offset;
        }
        let relative = (offset - start) as usize;
        let mut prefetch = if clean {
            window
                .max(relative + length)
                .min(window_limit)
                .min(size.saturating_sub(start) as usize)
        } else {
            length
        };
        let chunk = budgets.read_parallel_chunk_bytes.max(1);
        // A budget-limited large window should end on a configured parallel
        // chunk boundary, not add another partial GET at every refill. EOF keeps
        // its exact tail, and the caller's requested range always fits.
        if clean && prefetch < window && start + (prefetch as u64) < size && prefetch >= chunk {
            prefetch = (prefetch / chunk * chunk).max(relative + length);
        }
        // FUSE may issue neighboring reads before the first window completes.
        // Waiting readers must not reserve another whole window and evict the
        // very fill they can reuse. Recheck after wakeup and after taking ownership.
        let fill = loop {
            let admission = read::start_fill(&node, generation, start, start + prefetch as u64);
            let reply = self.admit(length).await?;
            if !read::current(&node, generation) {
                return Err(self.again("read_generation_after_fill_wait"));
            }
            let cached = self.cache.read(ino, generation, offset, length, reply);
            if let Some(block) = cached {
                self.touch_disk_hit(&node, generation, offset, length);
                return Ok(Some(block));
            }
            match admission {
                read::FillAdmission::Leader(owner) => {
                    self.diagnostics.debug("read_fill", || {
                        serde_json::json!({
                            "role": "leader", "offset": start, "bytes": prefetch,
                        })
                    });
                    break owner;
                }
                read::FillAdmission::Wait(mut done) => {
                    self.diagnostics.debug("read_fill", || {
                        serde_json::json!({
                            "role": "follower", "offset": start, "bytes": prefetch,
                        })
                    });
                    // Subscribe before the cache check so a completed chunk
                    // cannot be missed between lookup and waiting. Closing also
                    // wakes followers after error or cancellation; retry normally.
                    metrics::observe(&self.diagnostics, Stage::FillWait, async {
                        let _ = tokio::time::timeout(self.config.request_timeout, done.changed())
                            .await
                            .map_err(|_| Error::Timeout("overlapping read fill"))?;
                        Ok(())
                    })
                    .await?;
                }
            }
        };
        // Atomically admit all payload/GET/output buffers before the first
        // allocation; otherwise competing readers could each pin half a lane.
        let (charge, snapshot) = loop {
            let charge = self
                .admit(
                    (if single_buffer { 1 } else { 2 }) * prefetch
                        + length
                        + if single_buffer && self.disk_funding_bytes > 0 {
                            read::disk_window_admission(start, start + prefetch as u64, size)
                                + self.disk_gap_pipeline_bytes(read::disk_coverage(
                                    start,
                                    start + prefetch as u64,
                                    size,
                                ))
                        } else {
                            0
                        },
                )
                .await?;
            // Capture payload owners only after admission. Pinning dirty extents
            // while waiting for their own permits would deadlock a pressured read.
            let snapshot = read::snapshot(&node)?;
            let available = (snapshot.size.saturating_sub(start)).min(prefetch as u64) as usize;
            if single_buffer && !write::remote_only(&snapshot, start, available) {
                drop(snapshot);
                drop(charge);
                single_buffer = false;
                prefetch = prefetch.min(budgets.max_overlay_window_bytes);
                if prefetch < window && start + (prefetch as u64) < size && prefetch >= chunk {
                    prefetch = (prefetch / chunk * chunk).max(relative + length);
                }
                continue;
            }
            // A clean L2 plan includes short-EOF disk coverage for the original
            // size/generation. Admission can flush writers while it waits: drop
            // obsolete admission and let the caller retry before any splits.
            // Dirty-overlay replanning above and RAM-only reads stay unchanged.
            if single_buffer
                && self.disk_funding_bytes > 0
                && (snapshot.size != size
                    || read::generation(snapshot.version, snapshot.remote.as_ref()) != generation)
            {
                return Err(self.again("read_generation_after_window_admission"));
            }
            break (charge, Arc::new(snapshot));
        };
        let cacheable = clean
            && snapshot.version == version
            && snapshot.remote.as_ref().map(|head| head.etag.clone()) == etag;
        self.load_window(
            read::Window {
                snapshot,
                range: start..start + prefetch as u64,
                charge,
                single_buffer,
                cache_key: cacheable.then_some((ino, generation)),
                fill,
            },
            offset,
            length,
            borrow_reply,
            ready,
        )
        .await
    }

    async fn load_window(
        &self,
        window: read::Window,
        offset: u64,
        length: usize,
        borrow_reply: bool,
        ready: &mut Option<impl FnOnce(Result<Block>) + Send>,
    ) -> Result<Option<Block>> {
        let read::Window {
            snapshot,
            range,
            mut charge,
            single_buffer,
            cache_key,
            fill,
        } = window;
        let deadline = self.config.request_timeout.min(Duration::from_millis(250));
        let disk = if let Some((_, generation)) = cache_key.filter(|_| {
            single_buffer
                && self.disk_funding_bytes > 0
                && range.start < range.end.min(snapshot.size)
        }) {
            self.disk_read_window(
                fill.node().clone(),
                generation,
                snapshot.clone(),
                &mut charge,
                tokio::time::Instant::now() + deadline,
            )
            .await
        } else {
            if self.disk_funding_bytes > 0 {
                self.record_disk_decision(read::DiskDecision::Window);
            }
            None
        };
        let disk_reserved = single_buffer && self.disk_funding_bytes > 0;
        // At most max_requests range tasks coexist. Their fixed metadata pool
        // is admitted atomically with the window and shared, not re-reserved for
        // each hole/task turnover. Clones keep it funded through task cancellation.
        let pipeline_funding = disk_reserved.then(|| {
            Arc::new(
                charge.split(self.disk_gap_pipeline_bytes(read::disk_coverage(
                    range.start,
                    range.end,
                    snapshot.size,
                ))),
            )
        });
        let chunk = self.effective_budgets().read_parallel_chunk_bytes.max(1);
        let mut output = charge.split(length);
        // The snapshot may have shrunk while admission flushed another generation.
        let available = length.min(snapshot.size.saturating_sub(offset) as usize);
        let mut reply = Some(output.split(available));
        drop(output);
        let mut result = if available == 0 {
            Some(reply.take().expect("empty reply admission").zeroed())
        } else {
            None
        };
        let mut remaining = available;
        if remaining == 0
            && let Some(ready) = ready.take()
        {
            ready(Ok(result.take().expect("empty requested result")));
        }
        let end = range.end.min(snapshot.size);
        let mut tasks = tokio::task::JoinSet::new();
        let (sender, mut chunks) = tokio::sync::mpsc::channel(self.config.max_requests);
        let mut next = range.start;
        let mut prefetch_error = None;
        while next < end || !tasks.is_empty() || !chunks.is_empty() {
            while next < end && tasks.len() < self.config.max_requests {
                let at = next;
                let mut amount = (end - next).min(chunk as u64) as usize;
                if disk_reserved && next + (amount as u64) < end {
                    let aligned_end = (next + amount as u64) / read::READ_BLOCK_BYTES as u64
                        * read::READ_BLOCK_BYTES as u64;
                    if aligned_end > next {
                        amount = (aligned_end - next) as usize;
                    }
                }
                next += amount as u64;
                let mut owner = charge.split((if single_buffer { 1 } else { 2 }) * amount);
                let disk_charge = (disk_reserved && disk.is_some())
                    .then(|| charge.split(read::disk_window_admission(at, next, snapshot.size)));
                let disk = disk.clone();
                let node = fill.node().clone();
                let generation = cache_key.map(|(_, generation)| generation);
                let (store, snapshot) = (self.store.clone(), snapshot.clone());
                let metrics = self.read_metrics.clone();
                let diagnostics = self.diagnostics.clone();
                let context = metrics::context();
                let sender = sender.clone();
                let pipeline_funding = pipeline_funding.clone();
                tasks.spawn(metrics::scope(context, async move {
                    let _pipeline_funding = pipeline_funding;
                    metrics.fill_jobs.inc();
                    let result = async {
                        let local = Budget::new(2 * amount)?;
                        let output =
                            crate::backend::RangeSink::new(sender.clone(), owner.split(amount))
                                .with_diagnostics(diagnostics);
                        if single_buffer {
                            if let (Some(disk), Some(admission), Some(generation)) =
                                (disk, disk_charge, generation)
                            {
                                return read::load_disk_range(
                                    &store,
                                    &snapshot,
                                    at..at + amount as u64,
                                    &local,
                                    &output,
                                    &sender,
                                    read::DiskRange {
                                        disk,
                                        node,
                                        generation,
                                        admission,
                                        deadline,
                                    },
                                )
                                .await;
                            }
                            store
                                .read_range_chunks(
                                    &snapshot.key,
                                    at,
                                    amount,
                                    &snapshot
                                        .remote
                                        .as_ref()
                                        .expect("clean remote snapshot")
                                        .read_head(),
                                    &local,
                                    &output,
                                )
                                .await
                        } else {
                            let block =
                                write::materialize(&store, &snapshot, at, amount, &local).await?;
                            output.send(at, block).await
                        }
                    }
                    .await;
                    if result.is_err() {
                        metrics.fill_errors.inc();
                    }
                    (at, result)
                }));
            }
            let completed = tokio::select! {
                // A producer can finish with a late body error after enqueueing
                // valid requested bytes. Process those bytes before its result.
                biased;
                value = chunks.recv(), if !chunks.is_empty() || !tasks.is_empty() => {
                    Some(value.expect("window retains its delivery sender"))
                }
                completed = tasks.join_next(), if !tasks.is_empty() => {
                    let (at, outcome) = match completed.expect("nonempty read task set") {
                        Ok(value) => value,
                        Err(_) => {
                            tasks.abort_all();
                            while tasks.join_next().await.is_some() {}
                            return Err(Error::Cancelled);
                        }
                    };
                    if let Err(error) = outcome {
                        let chunk_end = (at + chunk as u64).min(end);
                        if at >= offset + available as u64 || chunk_end <= offset {
                            // Optional read-ahead cannot cancel a healthy requested
                            // dependency, irrespective of completion order.
                            prefetch_error.get_or_insert(error);
                            continue;
                        }
                        tasks.abort_all();
                        while tasks.join_next().await.is_some() {}
                        return Err(error);
                    }
                    None
                }
            };
            let Some((at, block)) = completed else {
                continue;
            };
            if block.is_disk_origin() {
                self.disk_read_bytes
                    .fetch_add(block.as_slice().len() as u64, Ordering::Relaxed);
            } else if single_buffer {
                self.read_metrics
                    .remote_bytes
                    .inc_by(block.as_slice().len() as u64);
            }
            let left = at.max(offset);
            let right = (at + block.as_slice().len() as u64).min(offset + available as u64);
            if right > left {
                if borrow_reply && remaining == available && right - left == available as u64 {
                    // The delivered immutable owner already covers the reply.
                    // Retain its complete backing charge instead of allocating a copy.
                    result = Some(block.slice((left - at) as usize..(right - at) as usize));
                    drop(reply.take());
                } else {
                    result
                        .get_or_insert_with(|| {
                            reply.take().expect("composed reply admission").zeroed()
                        })
                        .as_mut_slice()
                        [(left - offset) as usize..(right - offset) as usize]
                        .copy_from_slice(
                            &block.as_slice()[(left - at) as usize..(right - at) as usize],
                        );
                }
                remaining -= (right - left) as usize;
            }
            if remaining == 0
                && let Some(ready) = ready.take()
            {
                ready(Ok(result.take().expect("completed requested result")));
            }
            if let Some((ino, generation)) =
                cache_key.filter(|(_, generation)| read::current(fill.node(), *generation))
            {
                // Publish into the cache followers inspect, including after
                // pressure reclamation while the GET was in flight.
                let cache = &self.cache;
                // Delivery remains sliced above. Retain a validated disk record
                // under its original coverage so later eviction does not create
                // a different fragment identity for an already resident record.
                let (block, prefix) = block.into_cache_backing();
                let at = at - prefix as u64;
                cache.insert_with_eviction(ino, generation, at, block, |victim| {
                    self.demote_capacity(victim)
                });
                if right > left {
                    cache.consumed(
                        ino,
                        generation,
                        at,
                        (left - at) as usize,
                        (right - left) as usize,
                    );
                }
                fill.published();
                self.cache_published.notify_waiters();
            }
        }
        if let Some(error) = prefetch_error {
            return Err(error);
        }
        Ok(result)
    }

    async fn admit(&self, size: usize) -> Result<crate::budget::Reservation> {
        match self.data.try_reserve(size) {
            Ok(reservation) => Ok(reservation),
            Err(Error::Fs(code)) if code == libc::ENOMEM => {
                return metrics::observe(
                    &self.diagnostics,
                    Stage::PayloadAdmission,
                    self.admit_pressured(size),
                )
                .await;
            }
            Err(error) => Err(error),
        }
    }

    async fn admit_pressured(&self, size: usize) -> Result<crate::budget::Reservation> {
        // Reclaim only as much old cache ownership as admission needs.
        // Pinned borrowers retain their permits; FIFO waiters stay ahead.
        while self.reclaim_read_cache() {
            if let Ok(reservation) = self.data.try_reserve(size) {
                return Ok(reservation);
            }
        }
        if let Ok(reservation) = self.data.try_reserve(size) {
            return Ok(reservation);
        }
        if self.config.use_enomem {
            return self.data.try_reserve(size);
        }
        self.flush_all().await?;
        tokio::time::timeout(Duration::from_secs(30), async {
            // Retain the same FIFO reservation while reclaiming owners
            // published by fills that were in flight at the first sweep.
            let reservation = self.data.reserve(size);
            tokio::pin!(reservation);
            loop {
                let published = self.cache_published.notified();
                // notify_waiters covers futures created before the
                // cache check, including ones not yet polled.
                tokio::select! {
                    biased;
                    result = &mut reservation => return result,
                    _ = async {
                        let reclaimed = self.reclaim_read_cache();
                        if !reclaimed {
                            published.await;
                        }
                    } => {}
                }
            }
        })
        .await
        .map_err(|_| Error::Timeout("payload admission"))?
    }

    fn reclaim_read_cache(&self) -> bool {
        // Loan-first reclamation permanently hides byte-pressure victims from
        // L2: subsequent evictions remove loaned owners, which must not demote.
        // Prefer an eligible capacity victim before transferring its charge.
        if self.config.disk_cache_regex.is_some()
            && let Some(key) = self.cache.loan_candidate()
            && let Ok(node) = self.node(key.0)
        {
            self.start_disk_for_capacity(&node, key.1);
            if let Some(disk) = self.disk_read(&node, key.1)
                && disk.cache.status().state() == crate::disk_cache::State::Ready
                && let Some(victim) = self.cache.unloaned_capacity_victim(key)
            {
                self.demote_capacity(victim);
                return true;
            }
        }
        // Never wait for optional capacity or lend while an upload/body owns it.
        let loaned = self
            .flush_activity
            .try_write()
            .is_ok_and(|_activity| self.cache.loan_lru(&self.flush_lane));
        loaned || self.evict_cache()
    }

    pub async fn write(&self, ino: u64, offset: u64, bytes: &[u8]) -> Result<usize> {
        self.check_write()?;
        if bytes.len() > MAX_IO {
            return Err(Error::Fs(libc::EINVAL));
        }
        if offset
            .checked_add(bytes.len() as u64)
            .is_none_or(|end| end > self.config.part_sizes.max_size())
        {
            return Err(Error::Fs(libc::EFBIG));
        }
        let node = self.node(ino)?;
        if file_type(node.state.lock().unwrap().mode) != FileType::RegularFile {
            return Err(Error::Fs(libc::EINVAL));
        }
        let reservation = self.admit(bytes.len()).await?;
        let block = reservation.copy(bytes)?;
        let mut ns = self.lock_namespace();
        let mut state = node.state.lock().unwrap();
        if ns.is_moving(&state.key) || ns.is_deleting(&state.key) {
            return Err(self.again("write_binding_busy"));
        }
        write::insert_extent(&mut state, offset, block)?;
        ns.track_dirty(node.ino, &state);
        Ok(bytes.len())
    }

    pub fn truncate(&self, ino: u64, size: u64) -> Result<()> {
        self.check_write()?;
        if size > self.config.part_sizes.max_size() {
            return Err(Error::Fs(libc::EFBIG));
        }
        let node = self.node(ino)?;
        let mut ns = self.lock_namespace();
        let mut state = node.state.lock().unwrap();
        if ns.is_moving(&state.key) || ns.is_deleting(&state.key) {
            return Err(self.again("truncate_binding_busy"));
        }
        if file_type(state.mode) != FileType::RegularFile {
            return Err(Error::Fs(libc::EINVAL));
        }
        let result = write::truncate(&mut state, size);
        ns.track_dirty(node.ino, &state);
        result
    }

    fn prepare_metadata(&self, state: &mut State) -> Result<()> {
        write::prepare_metadata(&self.config, state)
    }

    pub async fn fsync(&self, ino: u64) -> Result<()> {
        let node = self.node(ino)?;
        let _gate = node.flush.lock().await;
        let key = node.state.lock().unwrap().key.clone();
        let busy = {
            let ns = self.lock_namespace();
            ns.is_moving(&key) || ns.is_deleting(&key) || Self::pending_rename(&ns, &key, None)
        };
        if busy {
            return Err(self.again("fsync_binding_busy"));
        }
        self.flush_node_locked(&node).await
    }

    async fn flush_node_locked(&self, node: &Inode) -> Result<()> {
        self.flush_node_locked_before(node, None).await
    }

    async fn flush_node_locked_before(
        &self,
        node: &Inode,
        before: Option<&mut write::BeforeMutation<'_>>,
    ) -> Result<()> {
        let result = self.commit_node_locked(node, before).await;
        let mut ns = self.lock_namespace();
        let state = node.state.lock().unwrap();
        ns.track_dirty(node.ino, &state);
        result
    }

    async fn commit_node_locked(
        &self,
        node: &Inode,
        before: Option<&mut write::BeforeMutation<'_>>,
    ) -> Result<()> {
        {
            let mut state = node.state.lock().unwrap();
            if state.deleted {
                return Err(Error::Fs(libc::ENOENT));
            }
            // A durability barrier with no pending generation owns no upload
            // work and must not reclaim the read cache's idle flush capacity.
            if state.version == state.durable {
                return Ok(());
            }
            if self.config.no_dir_object && file_type(state.mode) == FileType::Directory {
                state.durable = state.version;
                return Ok(());
            }
        }
        let flusher = self
            .flushers
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| Error::Cancelled)?;
        // Shared guards preserve independent flush concurrency. The budget carries
        // activity into cloned SDK bodies and jobs, including cancellation cleanup.
        let lane = self
            .flush_lane
            .with_activity(self.flush_activity.clone().read_owned().await);
        self.cache.reclaim_loans();
        write::flush_locked_before(
            &self.store,
            node,
            &lane,
            flusher,
            &self.config,
            &self.upload_admission,
            before,
        )
        .await
    }

    pub async fn flush_all(&self) -> Result<()> {
        self.reconcile_pending_deletes().await?;
        // Cost follows pending writes, never the clean namespace cardinality.
        let nodes = self.lock_namespace().dirty_nodes();
        for node in nodes {
            self.fsync(node.ino).await?;
        }
        Ok(())
    }

    pub async fn directory_chunk(
        &self,
        ino: u64,
        after: &str,
        limit: usize,
    ) -> Result<Vec<DirEntry>> {
        Ok(self
            .directory_chunk_inner(ino, after, limit, true, after)
            .await?
            .entries)
    }

    pub async fn directory_remainder(
        &self,
        ino: u64,
        page_after: &str,
        after: &str,
        limit: usize,
    ) -> Result<DirectoryRemainder> {
        self.directory_chunk_inner(ino, page_after, limit, true, after)
            .await
    }

    fn listed_node(
        &self,
        ns: &mut Namespace,
        key: &str,
        entry: &mut DirectoryMetadata,
        mut charge: crate::metadata_budget::MetadataCharge,
    ) -> Result<Arc<Inode>> {
        let make = |ino| {
            let key = match entry.key.take().filter(|owner| owner.as_str() == key) {
                Some(owner) => owner,
                None => {
                    crate::metadata_budget::SharedKey::from_admitted(key.to_owned(), &mut charge)?
                }
            };
            let node = Inode::cached_admitted(
                ino,
                key,
                entry.directory,
                entry.head.take(),
                entry.last_modified,
                &self.config,
                charge,
            )?;
            {
                let mut state = node.state.lock().unwrap();
                state.metadata_known = entry.known;
                state.fetched = entry.fetched;
            }
            Ok(node)
        };
        if let Some(node) = ns.named(key) {
            let dirty_descendants = entry.directory && ns.has_dirty_descendants(key);
            let mut state = node.state.lock().unwrap();
            if !dirty_descendants && state.can_replace_metadata(entry.fetched) {
                if entry.known {
                    state.replace_metadata(make(node.ino)?.state.into_inner().unwrap())?;
                } else if entry.directory
                    && state.remote.is_none()
                    && file_type(state.mode) == FileType::Directory
                {
                    // A common prefix validates an implicit directory's presence,
                    // not authoritative user metadata for a marker object.
                    state.fetched = entry.fetched;
                }
            }
            drop(state);
            return Ok(node);
        }
        ns.try_install(key, make)
    }

    async fn directory_chunk_inner(
        &self,
        ino: u64,
        after: &str,
        limit: usize,
        resolve_types: bool,
        resume_after: &str,
    ) -> Result<DirectoryRemainder> {
        let fetched = std::time::Instant::now();
        let mut snapshot = {
            let ns = self.lock_namespace();
            let snapshot = self.directory_snapshot(&ns, ino, resolve_types, fetched)?;
            if resolve_types
                && let Some(page) =
                    self.cached_directory_page(&ns, &snapshot, after, limit, resume_after)
            {
                return page.remainder(resume_after);
            }
            snapshot
        };
        let mut selected = self
            .local_directory_entries(&snapshot.prefix, after, limit)
            .await?;
        let slurped = if resolve_types && !self.config.stat_cache_ttl.is_zero() && limit > 0 {
            self.subtree_directory_entries(&snapshot.prefix, after, limit, &mut selected)
                .await?
        } else {
            None
        };
        if let Some(fetched) = slurped {
            snapshot.fetched = fetched;
        } else {
            self.remote_directory_entries(&snapshot.prefix, after, limit, fetched, &mut selected)
                .await?;
        }
        // Standard LIST cannot supply a safe concrete fuser type. Resolve only
        // this bounded page; extended authoritative metadata avoids these HEADs.
        if resolve_types {
            self.resolve_directory_types(ino, &mut selected).await?;
        }
        self.publish_directory_page(snapshot, after, limit, selected, resolve_types)
            .await?
            .remainder(resume_after)
    }

    fn directory_snapshot(
        &self,
        ns: &Namespace,
        ino: u64,
        resolve_types: bool,
        fetched: std::time::Instant,
    ) -> Result<DirectorySnapshot> {
        let node = ns.get(ino)?;
        let state = node.state.lock().unwrap();
        if file_type(state.mode) != FileType::Directory {
            return Err(Error::Fs(libc::ENOTDIR));
        }
        if state.deleted {
            return Err(Error::Fs(libc::ENOENT));
        }
        if resolve_types && (ns.is_moving(&state.key) || ns.is_deleting(&state.key)) {
            return Err(self.again("directory_binding_busy"));
        }
        let prefix = if state.key.is_empty() {
            String::new()
        } else {
            format!("{}/", state.key)
        };
        let epoch = node.directory_epoch.load(Ordering::Relaxed);
        drop(state);
        Ok(DirectorySnapshot {
            node,
            prefix,
            epoch,
            fetched,
        })
    }

    fn cached_directory_page(
        &self,
        ns: &Namespace,
        snapshot: &DirectorySnapshot,
        after: &str,
        limit: usize,
        resume_after: &str,
    ) -> Option<Arc<DirectoryPage>> {
        let page =
            self.directory_pages
                .get(&(snapshot.node.ino, snapshot.epoch, after.into(), limit))?;
        (page.fetched.elapsed() < self.config.stat_cache_ttl
            && page
                .remaining(resume_after)
                .iter()
                .all(|entry| self.directory_entry_fresh(ns, entry)))
        .then_some(page)
    }

    fn directory_entry_fresh(&self, ns: &Namespace, entry: &DirEntry) -> bool {
        // READDIR retains no lookup refs; pressure may have evicted a cached
        // child's inode or replaced its name/type after this page was recorded.
        ns.nodes.get(&entry.ino).is_some_and(|node| {
            let state = node.state.lock().unwrap();
            !state.deleted
                && ns.names.get(&state.key) == Some(&entry.ino)
                && file_type(state.mode) == entry.kind
                && (state.version > state.durable
                    || state.fetched.elapsed() < self.config.stat_cache_ttl)
        })
    }

    fn fresh_local_directory_entry(
        &self,
        ns: &Namespace,
        name: &str,
    ) -> Result<Option<DirectoryMetadata>> {
        let node = ns.named(name).unwrap();
        let state = node.state.lock().unwrap();
        if state.deleted {
            return Ok(None);
        }
        let fresh = state.version > state.durable
            || state.rename_target.is_some()
            || state.data_key.is_some()
            || state.fetched.elapsed() < self.config.stat_cache_ttl
            || file_type(state.mode) == FileType::Directory && ns.has_dirty_descendants(name);
        if !fresh {
            return Ok(None);
        }
        let charge = self
            .metadata
            .charge_with(|| std::mem::size_of::<DirectoryMetadata>() + name.len())?;
        Ok(Some(DirectoryMetadata {
            _metadata_charge: charge,
            directory: file_type(state.mode) == FileType::Directory,
            head: state.remote.clone(),
            key: Some(state.key.clone()),
            last_modified: Some(state.mtime),
            known: state.metadata_known,
            fetched: state.fetched,
        }))
    }

    async fn local_directory_entries(
        &self,
        prefix: &str,
        after: &str,
        limit: usize,
    ) -> Result<DirectorySelection> {
        let mut selected = DirectorySelection::new();
        if limit == 0 {
            return Ok(selected);
        }
        let mut cursor = Excluded(format!("{prefix}{after}"));
        loop {
            {
                let ns = self.lock_namespace();
                // Bound each critical section even when every cached child has
                // expired. Descendants are skipped by ordered seek, not visited.
                for _ in 0..128 {
                    let Some((name, _)) = ns
                        .names
                        .range::<str, _>((cursor.as_ref().map(String::as_str), Unbounded))
                        .next()
                    else {
                        return Ok(selected);
                    };
                    let Some(relative) = name.strip_prefix(prefix) else {
                        return Ok(selected);
                    };
                    if let Some((directory, _)) = relative.split_once('/') {
                        // '/' is immediately before '0' in UTF-8 byte ordering.
                        // Names such as 'a!' and 'a.' have already been visited.
                        cursor = Included(format!("{prefix}{directory}0"));
                        continue;
                    }
                    cursor = Excluded(name.to_string());
                    if ns
                        .tombstones
                        .get(name.as_str())
                        .is_none_or(|until| *until <= Instant::now())
                        && let Some(entry) = self.fresh_local_directory_entry(&ns, name)?
                    {
                        selected.insert(relative.into(), entry);
                        if selected.len() >= limit {
                            return Ok(selected);
                        }
                    }
                }
            }
            tokio::task::yield_now().await;
        }
    }

    async fn subtree_directory_entries(
        &self,
        prefix: &str,
        after: &str,
        limit: usize,
        selected: &mut DirectorySelection,
    ) -> Result<Option<Instant>> {
        let epoch = self.lock_namespace().epoch;
        let key = (epoch, prefix.to_owned());
        let cached = self.subtree_pages.get(&key).filter(|page| {
            page.fetched.elapsed() < self.config.stat_cache_ttl && {
                let ns = self.lock_namespace();
                page.objects.iter().all(|(key, _)| {
                    let Some(relative) = key.strip_prefix(prefix) else {
                        return true;
                    };
                    let name = relative.split('/').next().unwrap_or("");
                    ns.named(&format!("{prefix}{name}")).is_none_or(|node| {
                        let state = node.state.lock().unwrap();
                        state.version > state.durable
                            || state.fetched.elapsed() < self.config.stat_cache_ttl
                    })
                })
            }
        });
        let page = if let Some(page) = cached {
            page
        } else {
            // Go 0.43.9: one flat page at the start of readdir, then ordinary
            // delimiter pages if that response does not cover this directory.
            if !after.is_empty() {
                return Ok(None);
            }
            let fetched = Instant::now();
            let page = self
                .store
                .list_after(
                    "",
                    None,
                    None,
                    (!prefix.is_empty()).then(|| prefix.trim_end_matches('/')),
                )
                .await?;
            if !page.common_prefixes.is_empty() {
                return Err(Error::InvalidResponse(
                    "flat listing returned common prefixes".into(),
                ));
            }
            let last = page
                .objects
                .iter()
                .map(|object| object.key.as_str())
                .max()
                .unwrap_or("");
            let charge = self.metadata.charge_with(|| {
                let index_bytes = page
                    .objects
                    .iter()
                    .map(|object| {
                        object
                            .key
                            .match_indices('/')
                            .map(|(slash, _)| {
                                slash
                                    + 1
                                    + std::mem::size_of::<(u64, String)>()
                                    + crate::metadata_budget::LINKS
                            })
                            .sum::<usize>()
                    })
                    .sum::<usize>()
                    + prefix.len()
                    + std::mem::size_of::<SubtreePage>();
                page.objects.capacity()
                    * std::mem::size_of::<(crate::metadata_budget::SharedKey, DirectoryMetadata)>()
                    + index_bytes
            })?;
            let mut directories = std::collections::BTreeSet::from([prefix.to_owned()]);
            'objects: for object in &page.objects {
                for (slash, _) in object.key.match_indices('/') {
                    if directories.len() >= crate::backend::extended_list::MAX_PAGE_ENTRIES {
                        break 'objects;
                    }
                    directories.insert(object.key[..=slash].to_owned());
                }
            }
            directories.retain(|directory| {
                directory.as_str() >= prefix
                    && (page.next_token.is_none()
                        || !directory.is_empty()
                            && format!("{}0", directory.trim_end_matches('/')).as_str() <= last)
            });
            let objects = page
                .objects
                .into_iter()
                .map(|object| {
                    let (mut key, mut entry) = listed_object(object, fetched, &self.metadata)?;
                    share_listed_owners(&self.lock_namespace(), &mut key, &mut entry);
                    entry._metadata_charge.resize(self.metadata.estimate(|| {
                        std::mem::size_of::<DirectoryMetadata>() + crate::metadata_budget::LINKS
                    }))?;
                    Ok((key, entry))
                })
                .collect::<Result<Vec<_>>>()?;
            let page = Arc::new(SubtreePage {
                _metadata_charge: charge,
                fetched,
                objects,
            });
            // Do not publish speculative siblings across a concurrent mutation.
            if self.lock_namespace().epoch == epoch {
                for directory in &directories {
                    self.subtree_pages
                        .insert((epoch, directory.clone()), page.clone());
                }
                self.subtree_pages.run_pending_tasks();
            }
            if !directories.contains(prefix) {
                return Ok(None);
            }
            page
        };
        let mut remote = DirectorySelection::new();
        for (key, cached) in &page.objects {
            let Some(relative) = key.strip_prefix(prefix) else {
                continue;
            };
            let (name, metadata) = if let Some((directory, _)) = relative.split_once('/') {
                if !direct_directory_name(directory) {
                    continue;
                }
                let marker = format!("{prefix}{directory}/");
                let metadata = if *key == marker {
                    cached.try_clone(&self.metadata, key.len())?
                } else {
                    DirectoryMetadata {
                        _metadata_charge: self
                            .metadata
                            .charge(std::mem::size_of::<DirectoryMetadata>() + relative.len())?,
                        directory: true,
                        head: None,
                        last_modified: None,
                        key: None,
                        known: true,
                        fetched: page.fetched,
                    }
                };
                (directory, metadata)
            } else {
                (relative, cached.try_clone(&self.metadata, key.len())?)
            };
            if direct_directory_name(name) && name > after {
                remote.entry(name.to_owned()).or_insert(metadata);
            }
        }
        let mut ns = self.lock_namespace();
        for (name, metadata) in remote {
            if !ns.hidden(&format!("{prefix}{name}")) {
                selected.entry(name).or_insert(metadata);
                trim_directory_selection(selected, limit);
            }
        }
        Ok(Some(page.fetched))
    }

    async fn remote_directory_entries(
        &self,
        prefix: &str,
        after: &str,
        limit: usize,
        fetched: std::time::Instant,
        selected: &mut DirectorySelection,
    ) -> Result<()> {
        let mut token = None;
        let start_after = (!after.is_empty()).then(|| format!("{prefix}{after}"));
        loop {
            let page = self
                .store
                .list_after(prefix, Some("/"), token.as_deref(), start_after.as_deref())
                .await?;
            let (page_last, next_token) =
                self.select_directory_page(prefix, after, limit, page, fetched, selected)?;
            if next_token.is_none()
                || selected.len() == limit
                    && selected
                        .last_key_value()
                        .is_some_and(|(name, _)| *name <= page_last)
            {
                break;
            }
            token = next_token;
        }
        Ok(())
    }

    fn select_directory_page(
        &self,
        prefix: &str,
        after: &str,
        limit: usize,
        page: crate::backend::extended_list::ListPage,
        fetched: std::time::Instant,
        selected: &mut DirectorySelection,
    ) -> Result<(String, Option<String>)> {
        let mut page_last = String::new();
        for entry in page
            .objects
            .into_iter()
            .filter(|o| o.key != prefix)
            .map(|object| {
                let (mut key, mut entry) = listed_object(object, fetched, &self.metadata)?;
                share_listed_owners(&self.lock_namespace(), &mut key, &mut entry);
                Ok((key, entry))
            })
            .chain(page.common_prefixes.into_iter().map(|mut p| {
                let mut charge = self
                    .metadata
                    .charge(std::mem::size_of::<DirectoryMetadata>() + 2 * p.len() + 128)?;
                p.truncate(p.trim_end_matches('/').len());
                let p = crate::metadata_budget::SharedKey::from_admitted(p, &mut charge)?;
                Ok((
                    p.clone(),
                    DirectoryMetadata {
                        _metadata_charge: charge,
                        directory: true,
                        head: None,
                        last_modified: None,
                        key: Some(p),
                        known: false,
                        fetched,
                    },
                ))
            }))
        {
            let (name, entry) = entry?;
            let Some(name) = name.strip_prefix(prefix) else {
                return Err(Error::InvalidResponse("directory prefix escape".into()));
            };
            let name = name.trim_end_matches('/');
            if !direct_directory_name(name) {
                continue;
            }
            page_last = page_last.max(name.into());
            if name <= after {
                continue;
            }
            let full = child(prefix.trim_end_matches('/'), name)?;
            if self.lock_namespace().hidden(&full) {
                continue;
            }
            selected.entry(name.into()).or_insert(entry);
            trim_directory_selection(selected, limit);
        }
        Ok((page_last, page.next_token))
    }

    async fn resolve_directory_types(
        &self,
        ino: u64,
        selected: &mut DirectorySelection,
    ) -> Result<()> {
        let mut jobs = FuturesUnordered::new();
        let mut pending = selected
            .iter_mut()
            .filter(|(_, entry)| !entry.directory && !entry.known);
        let mut next = pending.next();
        while next.is_some() || !jobs.is_empty() {
            tokio::select! {
                completed = jobs.next(), if !jobs.is_empty() => {
                    completed.expect("admitted metadata job")?;
                }
                admitted = self.metadata_permit(), if next.is_some() => {
                    let permit = admitted?;
                    let (name, entry) = next.take().expect("selected unknown type");
                    // Admission precedes creation of each future: concurrent
                    // pages cannot accumulate per-entry semaphore waiters.
                    // Borrowed futures drop synchronously on error/cancel.
                    jobs.push(self.resolve_directory_entry(ino, name, entry, permit));
                    next = pending.next();
                }
            }
        }
        Ok(())
    }

    async fn metadata_permit(&self) -> Result<tokio::sync::OwnedSemaphorePermit> {
        metrics::observe(&self.diagnostics, Stage::MetadataSlots, async {
            tokio::time::timeout(
                self.config.http_timeout,
                self.metadata_slots.clone().acquire_owned(),
            )
            .await
            .map_err(|_| Error::Timeout("directory metadata admission"))?
            .map_err(|_| Error::Cancelled)
        })
        .await
    }

    async fn resolve_directory_entry(
        &self,
        ino: u64,
        name: &str,
        entry: &mut DirectoryMetadata,
        permit: tokio::sync::OwnedSemaphorePermit,
    ) -> Result<()> {
        let _permit = permit;
        let resolved = tokio::time::timeout(self.config.http_timeout, self.lookup(ino, name))
            .await
            .map_err(|_| Error::Timeout("directory metadata lookup"))??;
        let state = resolved.state.lock().unwrap();
        let charge = self
            .metadata
            .charge_with(std::mem::size_of::<DirectoryMetadata>)?;
        *entry = DirectoryMetadata {
            _metadata_charge: charge,
            directory: file_type(state.mode) == FileType::Directory,
            head: state.remote.clone(),
            key: Some(state.key.clone()),
            last_modified: Some(state.mtime),
            known: state.metadata_known,
            fetched: state.fetched,
        };
        Ok(())
    }

    async fn publish_directory_page(
        &self,
        snapshot: DirectorySnapshot,
        after: &str,
        limit: usize,
        selected: DirectorySelection,
        resolve_types: bool,
    ) -> Result<Arc<DirectoryPage>> {
        let page_charge = self.metadata.charge(
            std::mem::size_of::<DirectoryPage>()
                + std::mem::size_of::<(u64, u64, String, usize)>()
                + after.len()
                + crate::metadata_budget::LINKS,
        )?;
        let ino = snapshot.node.ino;
        let mut entries = Vec::with_capacity(selected.len());
        let deadline = Instant::now() + self.config.request_timeout;
        for (name, mut entry) in selected {
            let entry_charge = self
                .metadata
                .charge(4 * std::mem::size_of::<DirEntry>() + name.capacity())?;
            let full = child(snapshot.prefix.trim_end_matches('/'), &name)?;
            let node = loop {
                if Instant::now() >= deadline {
                    return Err(Error::Timeout("directory metadata admission"));
                }
                let new_inode = self.lock_namespace().named(&full).is_none();
                let charge = self
                    .admit_metadata(
                        if self.metadata.enabled() {
                            Inode::metadata_estimate(&full, None)
                        } else {
                            0
                        },
                        new_inode,
                    )
                    .await?;
                let result = {
                    let mut ns = self.lock_namespace();
                    if snapshot.node.directory_epoch.load(Ordering::Relaxed) != snapshot.epoch
                        || ns.names.get(snapshot.prefix.trim_end_matches('/')) != Some(&ino)
                    {
                        return Err(self.again("directory_generation_changed"));
                    }
                    self.listed_node(&mut ns, &full, &mut entry, charge)
                };
                match result {
                    Err(Error::Fs(libc::EAGAIN)) => tokio::task::yield_now().await,
                    other => break other?,
                }
            };
            entries.push(DirEntry {
                ino: node.ino,
                name: crate::metadata_budget::MetadataString::new(name, entry_charge),
                kind: node.attr().kind,
            });
        }
        let page = Arc::new(DirectoryPage {
            metadata_charge: page_charge,
            fetched: snapshot.fetched,
            entries,
        });
        let ns = self.lock_namespace();
        if snapshot.node.directory_epoch.load(Ordering::Relaxed) != snapshot.epoch
            || ns.names.get(snapshot.prefix.trim_end_matches('/')) != Some(&ino)
        {
            return Err(self.again("directory_generation_changed"));
        }
        if resolve_types {
            self.directory_pages
                .insert((ino, snapshot.epoch, after.into(), limit), page.clone());
            self.directory_pages.run_pending_tasks();
        }
        Ok(page)
    }

    pub async fn unlink(&self, parent: u64, name: &str, directory: bool) -> Result<()> {
        self.check_write()?;
        let key = self.child_key(parent, name)?;
        let pending = {
            let ns = self.lock_namespace();
            ns.pending_deletes
                .get(&key)
                .and_then(|ino| ns.nodes.get(ino))
                .cloned()
        };
        let node = match pending {
            Some(node) => node,
            None => self.lookup(parent, name).await?,
        };
        if (node.attr().kind == FileType::Directory) != directory {
            return Err(Error::Fs(if directory {
                libc::ENOTDIR
            } else {
                libc::EISDIR
            }));
        }
        self.unlink_node(node, key, directory, Some((parent, name)))
            .await
    }

    /// Resolve owned ambiguous DELETEs before shutdown/retry. The retained name
    /// binding excludes recreation; only the original inode can complete it.
    pub async fn reconcile_pending_deletes(&self) -> Result<()> {
        let pending: Vec<_> = {
            let ns = self.lock_namespace();
            ns.pending_deletes
                .iter()
                .map(|(key, ino)| Ok((key.clone(), ns.get(*ino)?)))
                .collect::<Result<_>>()?
        };
        for (key, node) in pending {
            let directory = node.attr().kind == FileType::Directory;
            self.unlink_node(node, key, directory, None).await?;
        }
        Ok(())
    }

    async fn unlink_node(
        &self,
        node: Arc<Inode>,
        key: String,
        directory: bool,
        parent: Option<(u64, &str)>,
    ) -> Result<()> {
        let _gate = node.flush.lock().await;
        {
            let ns = self.lock_namespace();
            let state = node.state.lock().unwrap();
            if state.deleted
                || state.key != key
                || ns.names.get(key.as_str()) != Some(&node.ino)
                || Self::pending_rename(&ns, &key, None)
            {
                return Err(Error::Fs(libc::ENOENT));
            }
        }
        let _reservation = MoveLease::begin_delete(&self.namespace, key.clone(), node.ino)?;
        {
            let ns = self.lock_namespace();
            let state = node.state.lock().unwrap();
            if state.deleted || state.key != key || ns.names.get(key.as_str()) != Some(&node.ino) {
                return Err(Error::Fs(libc::ENOENT));
            }
            if let Some((parent, name)) = parent {
                let parent = ns.get(parent)?;
                let parent = parent.state.lock().unwrap();
                if parent.deleted || child(&parent.key, name)? != key {
                    return Err(Error::Fs(libc::ENOENT));
                }
            }
        }
        // An emptiness probe publishes no dirent types and holds this prefix's
        // reservation; it must not recursively look up children through that lease.
        if directory
            && !self
                .directory_chunk_inner(node.ino, "", 1, false, "")
                .await?
                .entries
                .is_empty()
        {
            return Err(Error::Fs(libc::ENOTEMPTY));
        }
        {
            let mut ns = self.lock_namespace();
            ns.begin_pending_delete(&key, node.ino)?;
        }
        let remote_key = if directory {
            format!("{key}/")
        } else {
            key.clone()
        };
        // Cancellation and ambiguous errors leave a bounded pending record, not
        // an irreversibly hidden/deleted dirty inode. Retrying unlink reconciles
        // the idempotent DELETE while recreation and conflicting moves stay out.
        if let Err(error) = self.store.delete(&remote_key).await {
            // A modeled rejection is not an ambiguous applied DELETE. Restore
            // ordinary lookup on authorization/validation failures only.
            if matches!(&error,Error::Backend{code,..} if matches!(code.as_str(),"AccessDenied"|"InvalidAccessKeyId"|"SignatureDoesNotMatch"|"InvalidArgument"|"InvalidRequest"))
            {
                let mut ns = self.lock_namespace();
                ns.remove_pending_delete(&key);
                ns.remove_tombstone(&key);
                ns.changed(&key);
            }
            return Err(error);
        }
        let mut ns = self.lock_namespace();
        ns.remove_pending_delete(&key);
        ns.names.remove(key.as_str());
        ns.changed(&key);
        let mut state = node.state.lock().unwrap();
        state.deleted = true;
        state.extents.clear();
        state.durable = state.version;
        ns.track_dirty(node.ino, &state);
        Ok(())
    }

    pub async fn rename(
        &self,
        parent: u64,
        name: &str,
        newparent: u64,
        newname: &str,
    ) -> Result<()> {
        self.check_write()?;
        let source = self.lookup(parent, name).await?;
        let destination = self.child_key(newparent, newname)?;
        let old = source.state.lock().unwrap().key.clone();
        {
            let ns = self.lock_namespace();
            if Self::pending_rename(&ns, &old, Some(source.ino))
                || Self::pending_rename(&ns, &destination, Some(source.ino))
                || [old.as_str(), destination.as_str()].into_iter().any(|key| {
                    let prefix = format!("{key}/");
                    ns.pending_moves
                        .range(prefix.clone()..)
                        .take_while(|(key, _)| key.starts_with(&prefix))
                        .any(|(_, ino)| *ino != source.ino)
                })
            {
                return Err(Error::Fs(libc::EBUSY));
            }
        }
        if old == destination {
            return Ok(());
        }
        if destination.starts_with(&format!("{old}/")) {
            return Err(Error::Fs(libc::EINVAL));
        }
        let directory = source.attr().kind == FileType::Directory;
        let retry =
            source.state.lock().unwrap().rename_target.as_deref() == Some(destination.as_str());
        let target = match self.lookup(newparent, newname).await {
            Ok(target) => {
                if (target.attr().kind == FileType::Directory) != directory {
                    return Err(Error::Fs(if directory {
                        libc::ENOTDIR
                    } else {
                        libc::EISDIR
                    }));
                }
                if directory && !retry && !self.directory_chunk(target.ino, "", 1).await?.is_empty()
                {
                    return Err(Error::Fs(libc::ENOTEMPTY));
                }
                Some(target)
            }
            Err(error) if not_found(&error) => None,
            Err(error) => return Err(error),
        };
        let _moving =
            MoveLease::begin(&self.namespace, vec![old.to_string(), destination.clone()])?;
        let bindings = self.lock_namespace().bindings(&old, directory)?;
        let _pin_charge = self
            .metadata
            .charge_with(|| (bindings.len() + 1) * (std::mem::size_of::<Arc<Inode>>() * 4 + 128))?;
        let mut pinned: Vec<_> = {
            let ns = self.lock_namespace();
            bindings
                .iter()
                .map(|(_, ino)| ns.nodes[ino].clone())
                .collect()
        };
        if let Some(target) = &target {
            pinned.push(target.clone());
        }
        pinned.sort_by_key(|node| node.ino);
        pinned.dedup_by_key(|node| node.ino);
        let mut gates = Vec::with_capacity(pinned.len());
        for node in &pinned {
            gates.push(node.flush.clone().lock_owned().await);
        }
        // Metadata can change externally without changing the content ETag, and
        // an implicit cached directory can acquire a marker. Discover the COPY
        // generation's actual metadata before committing a byte plan, not after
        // creating a destination. The disabled path retains its old I/O shape.
        let mut copy_heads = BTreeMap::new();
        let source_was_dirty = {
            let state = source.state.lock().unwrap();
            state.version > state.durable
        };
        if self.metadata.enabled() {
            for node in &pinned {
                let (key, has_remote, dirty, _key_charge) = {
                    let state = node.state.lock().unwrap();
                    if state.key != old && !state.key.starts_with(&format!("{old}/")) {
                        continue;
                    }
                    let backing = state.data_key.as_ref().unwrap_or(&state.key);
                    let charge = self.metadata.charge(backing.len() + 1)?;
                    let mut key = String::with_capacity(backing.len() + 1);
                    key.push_str(backing);
                    if file_type(state.mode) == FileType::Directory {
                        key.push('/');
                    }
                    (
                        key,
                        state.remote.is_some(),
                        state.version > state.durable,
                        charge,
                    )
                };
                if !has_remote && dirty {
                    continue;
                }
                let head = match self.store.head(&key).await {
                    Ok(head) => head,
                    Err(error) if !has_remote && not_found(&error) => continue,
                    Err(error) => return Err(error),
                };
                let charge = self.metadata.charge_with(|| {
                    crate::metadata_budget::head_bytes(&head)
                        + std::mem::size_of::<(u64, ObjectHead)>()
                        + crate::metadata_budget::LINKS
                })?;
                copy_heads.insert(
                    node.ino,
                    crate::metadata_budget::MetadataOwner::new(head, charge),
                );
            }
        }
        // Reserve the complete bounded COPY/publication working set before any
        // remote mutation. Credit transfers into canonical owners without a
        // release/reacquire race; unused credit is released on every exit.
        let mut move_credit = self.metadata.charge_with(|| {
            pinned
                .iter()
                .map(|node| {
                    let state = node.state.lock().unwrap();
                    let observed_head = copy_heads
                        .get(&node.ino)
                        .map(|head| crate::metadata_budget::head_bytes(head));
                    let mut head = 2 * observed_head.unwrap_or_else(|| {
                        state.remote.as_ref().map_or(0, |remote| remote.bytes())
                    }) + 1024;
                    if state.version > state.durable {
                        // A preceding flush can replace a small raw HEAD with
                        // percent-encoded local attributes. Include that known
                        // generation before allowing the flush or any COPY.
                        head = head.max(
                            state.metadata.encoding_bytes()
                                + 16 * write::metadata_growth(&self.config)
                                + std::mem::size_of::<ObjectHead>()
                                + 1024,
                        );
                    }
                    let new_len = destination.len() + state.key.len().saturating_sub(old.len());
                    4 * head
                        + 12 * new_len
                        + 2 * state.metadata_bytes()
                        + 4 * crate::metadata_budget::LINKS
                })
                .sum()
        })?;
        let tombstone_credit = self.lock_namespace().reserve_tombstone(&old)?;
        let mut published = false;
        let mut publish_move = |move_credit: &mut crate::metadata_budget::MetadataCharge| {
            if published {
                return Ok(());
            }
            let mut ns = self.lock_namespace();
            let mut state = source.state.lock().unwrap();
            if state.deleted || state.key != old {
                return Err(Error::Fs(libc::ENOENT));
            }
            if state
                .rename_target
                .as_ref()
                .is_some_and(|key| *key != destination)
            {
                return Err(Error::Fs(libc::EBUSY));
            }
            state.admit_metadata_from(move_credit, destination.len())?;
            if let Err(error) = ns.begin_pending_move(&old, &destination, source.ino) {
                state.settle_metadata();
                return Err(error);
            }
            state.rename_target = Some(destination.clone());
            state.settle_metadata();
            published = true;
            Ok(())
        };
        // A failed dirty-flush admission has not started a rename. Publish its
        // persistent intent at the first mutating request instead; the scoped
        // MoveLease already excludes local writers during preparation. Existing
        // retry intent is never removed, and possibly applied requests retain it.
        self.flush_node_locked_before(&source, Some(&mut || publish_move(&mut move_credit)))
            .await?;
        // A dirty flush created a new generation. Keep its complete HTTP
        // properties only in this COPY operation's pre-admitted working set.
        if self.metadata.enabled()
            && source_was_dirty
            && source.state.lock().unwrap().remote.is_some()
        {
            let key = {
                let state = source.state.lock().unwrap();
                let key = state.data_key.as_ref().unwrap_or(&state.key);
                if directory {
                    format!("{key}/")
                } else {
                    key.to_string()
                }
            };
            let head = self.store.head(&key).await?;
            let charge = move_credit.take(crate::metadata_budget::head_bytes(&head))?;
            copy_heads.insert(
                source.ino,
                crate::metadata_budget::MetadataOwner::new(head, charge),
            );
        }
        // A clean/no-directory-object flush sends no request. Keep only the
        // scoped lease during read-only discovery; persistent intent starts at
        // COPY, after its fallible preparation, not before LIST or ETag HEAD.
        if directory {
            // Drain paged source objects incrementally; never materialize a subtree.
            let prefix = format!("{old}/");
            let _index_charge = move_credit.take(
                bindings.len()
                    * (std::mem::size_of::<(&str, Arc<Inode>)>() + crate::metadata_budget::LINKS),
            )?;
            let cached: BTreeMap<_, _> = {
                let ns = self.lock_namespace();
                bindings
                    .iter()
                    .map(|(key, ino)| (key.as_str(), ns.nodes[ino].clone()))
                    .collect()
            };
            let mut last_drained = None;
            loop {
                let page = self.store.list(&prefix, None, None).await?;
                if page.objects.is_empty() {
                    break;
                }
                for object in page.objects {
                    if last_drained
                        .as_ref()
                        .is_some_and(|key: &String| object.key <= *key)
                    {
                        return Err(self.again("rename_listing_repeated"));
                    }
                    let suffix = object
                        .key
                        .strip_prefix(&prefix)
                        .ok_or_else(|| Error::InvalidResponse("rename prefix escape".into()))?;
                    let target = format!("{destination}/{suffix}");
                    if self.metadata.enabled()
                        && cached.contains_key(object.key.trim_end_matches('/'))
                    {
                        let node = &cached[object.key.trim_end_matches('/')];
                        if !copy_heads.contains_key(&node.ino)
                            && !(node.ino == source.ino && source_was_dirty)
                        {
                            return Err(Error::Fs(libc::ESTALE));
                        }
                    }
                    let etag = match object.etag {
                        Some(etag) => etag,
                        None => self.store.head(&object.key).await?.etag,
                    };
                    publish_move(&mut move_credit)?;
                    let copied = self.store.copy(&object.key, &target, &etag).await?;
                    if let Some(node) = cached.get(object.key.trim_end_matches('/')) {
                        let mut head = self.store.head(&target).await?;
                        if head.etag != copied {
                            return Err(Error::Fs(libc::ESTALE));
                        }
                        let mut state = node.state.lock().unwrap();
                        let mut _normalization = None;
                        if self.metadata.enabled() {
                            let expected = copy_heads.get(&node.ino).map(|head| &**head);
                            if expected.is_none_or(|expected| !same_copy_metadata(expected, &head))
                            {
                                return Err(Error::Fs(libc::ESTALE));
                            }
                            _normalization =
                                Some(move_credit.take(
                                    2 * crate::metadata_budget::head_bytes(expected.unwrap()),
                                )?);
                            compact_copy_head(&mut head);
                        }
                        state.admit_metadata_from(
                            &mut move_credit,
                            self.metadata.estimate(|| {
                                crate::metadata_budget::head_bytes(&head) + target.len()
                            }),
                        )?;
                        let remote = state.cache_remote_from(head, &mut move_credit)?;
                        if state.version == state.durable {
                            state.metadata = remote.metadata.clone();
                        }
                        state.remote = Some(remote);
                        state.data_key = Some(crate::metadata_budget::SharedKey::from_admitted(
                            target.trim_end_matches('/').to_owned(),
                            &mut move_credit,
                        )?);
                        state.settle_metadata();
                    }
                    self.store.delete(&object.key).await?;
                    last_drained = Some(object.key);
                }
            }
        } else {
            let needs_copy =
                source.state.lock().unwrap().data_key.as_deref() != Some(destination.as_str());
            if needs_copy {
                let (remote, mut head, _copy_charge) = {
                    let state = source.state.lock().unwrap();
                    let remote = state.remote.clone().ok_or(Error::Fs(libc::EIO))?;
                    let head = copy_heads
                        .get(&source.ino)
                        .map(|head| (**head).clone())
                        .unwrap_or_else(|| remote.read_head());
                    let charge = move_credit.take(
                        self.metadata
                            .estimate(|| crate::metadata_budget::head_bytes(&head)),
                    )?;
                    (remote, head, charge)
                };
                publish_move(&mut move_credit)?;
                head.etag = self.store.copy(&old, &destination, &head.etag).await?;
                // Publish the backing key before DELETE can remove the old
                // source, including cancellation or a lost DELETE response.
                let mut state = source.state.lock().unwrap();
                state.admit_metadata_from(
                    &mut move_credit,
                    self.metadata
                        .estimate(|| crate::metadata_budget::head_bytes(&head) + destination.len()),
                )?;
                let remote = if self.metadata.enabled() {
                    state.cache_remote_from(head, &mut move_credit)?
                } else {
                    remote.with_generation(head.size, head.etag)?
                };
                if state.version == state.durable {
                    state.metadata = remote.metadata.clone();
                }
                state.remote = Some(remote);
                state.data_key = Some(crate::metadata_budget::SharedKey::from_admitted(
                    destination.clone(),
                    &mut move_credit,
                )?);
                state.settle_metadata();
            }
            let mut head = self.store.head(&destination).await?;
            if source
                .state
                .lock()
                .unwrap()
                .remote
                .as_ref()
                .is_none_or(|remote| remote.etag != head.etag)
            {
                return Err(Error::Fs(libc::ESTALE));
            }
            {
                let mut state = source.state.lock().unwrap();
                let mut _normalization = None;
                if self.metadata.enabled()
                    && copy_heads
                        .get(&source.ino)
                        .is_none_or(|expected| !same_copy_metadata(expected, &head))
                {
                    return Err(Error::Fs(libc::ESTALE));
                }
                if self.metadata.enabled() {
                    _normalization = Some(move_credit.take(
                        2 * crate::metadata_budget::head_bytes(
                            copy_heads.get(&source.ino).unwrap(),
                        ),
                    )?);
                    compact_copy_head(&mut head);
                }
                state.admit_metadata_from(
                    &mut move_credit,
                    self.metadata
                        .estimate(|| crate::metadata_budget::head_bytes(&head)),
                )?;
                let remote = state.cache_remote_from(head, &mut move_credit)?;
                if state.version == state.durable {
                    state.metadata = remote.metadata.clone();
                }
                state.remote = Some(remote);
                state.settle_metadata();
            }
            self.store.delete(&old).await?;
        }
        // Copy can change ETags, including for cached descendants. Refresh those
        // generations while their per-inode commit gates remain held.
        if directory {
            for node in &pinned {
                let info = {
                    let state = node.state.lock().unwrap();
                    (
                        state.key.clone(),
                        state.remote.is_some(),
                        file_type(state.mode) == FileType::Directory,
                    )
                };
                if info.0.starts_with(&format!("{old}/")) && info.1 {
                    let new = format!(
                        "{destination}{}{}",
                        &info.0[old.len()..],
                        if info.2 { "/" } else { "" }
                    );
                    let mut head = self.store.head(&new).await?;
                    let mut state = node.state.lock().unwrap();
                    let mut _normalization = None;
                    if self.metadata.enabled()
                        && copy_heads
                            .get(&node.ino)
                            .is_none_or(|expected| !same_copy_metadata(expected, &head))
                    {
                        return Err(Error::Fs(libc::ESTALE));
                    }
                    if self.metadata.enabled() {
                        _normalization = Some(move_credit.take(
                            2 * crate::metadata_budget::head_bytes(
                                copy_heads.get(&node.ino).unwrap(),
                            ),
                        )?);
                        compact_copy_head(&mut head);
                    }
                    state.admit_metadata_from(
                        &mut move_credit,
                        self.metadata
                            .estimate(|| crate::metadata_budget::head_bytes(&head)),
                    )?;
                    let remote = state.cache_remote_from(head, &mut move_credit)?;
                    if state.version == state.durable {
                        state.metadata = remote.metadata.clone();
                    }
                    state.remote = Some(remote);
                    state.settle_metadata();
                }
            }
        }
        // Reserve the complete publication before changing any name binding.
        // Failed admission leaves the retryable move markers and old live names.
        let _new_keys_charge = move_credit.take(self.metadata.estimate(|| {
            bindings.len() * std::mem::size_of::<crate::metadata_budget::SharedKey>()
        }))?;
        let mut ns = self.lock_namespace();
        let new_keys = bindings
            .iter()
            .map(|(key, ino)| {
                let new = format!("{destination}{}", &key[old.len()..]);
                if let Some(node) = ns.nodes.get(ino) {
                    let state = node.state.lock().unwrap();
                    if let Some(backing) = &state.data_key
                        && *backing == new
                    {
                        return Ok(backing.clone());
                    }
                }
                crate::metadata_budget::SharedKey::from_admitted(new, &mut move_credit)
            })
            .collect::<Result<Vec<_>>>()?;
        // Empty/pure-local directories reach this synchronous namespace commit
        // without remote mutation: the scoped lease suffices, so no persistent
        // retry intent is needed. There is no await during this publication.
        ns.tombstone_reserved(&old, tombstone_credit)?;
        ns.remove_tombstone(&destination);
        if let Some(ino) = ns.names.remove(destination.as_str())
            && let Some(target) = ns.nodes.get(&ino)
        {
            let mut state = target.state.lock().unwrap();
            state.deleted = true;
            state.extents.clear();
            state.durable = state.version;
        }
        ns.forget_dirty(&destination);
        for ((key, &ino), new) in bindings.iter().map(|(key, ino)| (key, ino)).zip(new_keys) {
            ns.names.remove(key.as_str());
            ns.names.insert(new.clone(), ino);
            ns.forget_dirty(key);
            if let Ok(node) = ns.get(ino) {
                let mut state = node.state.lock().unwrap();
                state.key = new;
                state.data_key = None;
                state.negative_lookups.clear();
                if ino == source.ino {
                    state.rename_target = None;
                }
                ns.track_dirty(ino, &state);
                state.settle_metadata();
            }
        }
        ns.changed(&destination);
        ns.remove_pending_move(&old);
        ns.remove_pending_move(&destination);
        ns.get(newparent)?
            .state
            .lock()
            .unwrap()
            .negative_lookups
            .remove(newname);
        Ok(())
    }

    pub fn setattr(
        &self,
        ino: u64,
        mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        mtime: Option<SystemTime>,
    ) -> Result<FileAttr> {
        self.check_write()?;
        if ino == 1 {
            return Err(Error::Fs(libc::EOPNOTSUPP));
        }
        let node = self.node(ino)?;
        {
            let mut ns = self.lock_namespace();
            let mut state = node.state.lock().unwrap();
            if ns.is_moving(&state.key) || ns.is_deleting(&state.key) {
                return Err(self.again("setattr_binding_busy"));
            }
            if state.deleted {
                return Err(Error::Fs(libc::ENOENT));
            }
            let previous = (state.mode, state.uid, state.gid, state.mtime);
            if self.config.enable_perms {
                if let Some(mode) = mode {
                    state.mode = (state.mode & libc::S_IFMT) | (mode & 0o7777);
                }
                if let Some(uid) = uid {
                    state.uid = uid;
                }
                if let Some(gid) = gid {
                    state.gid = gid;
                }
            }
            if let Some(mtime) = mtime {
                state.mtime = mtime;
            }
            if let Err(error) = self.prepare_metadata(&mut state) {
                (state.mode, state.uid, state.gid, state.mtime) = previous;
                return Err(error);
            }
            state.version += 1;
            ns.track_dirty(node.ino, &state);
        }
        Ok(node.attr())
    }

    pub fn xattrs(&self, ino: u64) -> Result<crate::metadata_budget::Xattrs> {
        if self.config.disable_xattr {
            return Err(Error::Fs(libc::ENOSYS));
        }
        let node = self.node(ino)?;
        let state = node.state.lock().unwrap();
        let charge = self.metadata.charge_with(|| {
            2 * state.metadata.map_bytes()
                + state.metadata.len() * 5
                + state
                    .remote
                    .as_ref()
                    .map_or(0, |head| head.etag.len() + 128)
        })?;
        let mut attrs: BTreeMap<_, _> = state
            .metadata
            .iter()
            .map(|(k, v)| (format!("user.{k}"), v.to_vec()))
            .collect();
        if let Some(remote) = &state.remote {
            attrs.insert("s3.etag".into(), remote.etag.as_bytes().to_vec());
        }
        Ok(crate::metadata_budget::Xattrs::new(attrs, charge))
    }

    pub fn setxattr(&self, ino: u64, name: &str, value: Option<&[u8]>, flags: i32) -> Result<()> {
        self.check_write()?;
        if self.config.disable_xattr {
            return Err(Error::Fs(libc::ENOSYS));
        }
        let node = self.node(ino)?;
        if ino == 1 {
            return Err(Error::Fs(libc::EOPNOTSUPP));
        }
        let mut ns = self.lock_namespace();
        let mut state = node.state.lock().unwrap();
        if ns.is_moving(&state.key) || ns.is_deleting(&state.key) {
            return Err(self.again("setxattr_binding_busy"));
        }
        if state.deleted {
            return Err(Error::Fs(libc::ENOENT));
        }
        let Some(name) = name.strip_prefix("user.") else {
            return Ok(());
        };
        if name.eq_ignore_ascii_case(crate::backend::metadata::UPLOAD_RECEIPT) {
            return Err(Error::Fs(libc::EPERM));
        }
        if name == self.config.symlink_attr {
            return Ok(());
        }
        let exists = state.metadata.contains_key(name);
        if flags & libc::XATTR_CREATE != 0 && exists {
            return Err(Error::Fs(libc::EEXIST));
        }
        if !exists && (flags & libc::XATTR_REPLACE != 0 || value.is_none()) {
            return Err(Error::Fs(libc::ENODATA));
        }
        let size: usize = state
            .metadata
            .iter()
            .filter(|(k, _)| *k != name)
            .map(|(k, v)| k.len() + v.len())
            .sum();
        if size + name.len() + value.map_or(0, <[u8]>::len) > 65536 {
            return Err(Error::Fs(libc::E2BIG));
        }
        if let Some(value) = value {
            state.metadata.try_insert(name, value)?;
        } else {
            state.metadata.try_remove(name)?;
        }
        state.version += 1;
        state.settle_metadata();
        ns.track_dirty(node.ino, &state);
        Ok(())
    }

    pub async fn shutdown(&self) -> Result<()> {
        self.closing.store(true, Ordering::Release);
        let (cache, startup) = {
            let mut disk = self.disk.lock().unwrap();
            disk.closing = true;
            if let Some(deferred) = disk.deferred.take() {
                deferred.close();
            }
            (disk.cache.clone(), disk.startup.clone())
        };
        let deadline = self.config.request_timeout;
        // Teardown is independently owned before the first writer await. A
        // cancelled/failed flush cannot strand the optional directory lock.
        let close = (cache.is_some() || startup.is_some()).then(|| {
            tokio::spawn(async move {
                if let Some(cache) = cache {
                    cache.close(deadline).await?;
                }
                if let Some(startup) = startup {
                    tokio::time::timeout(Duration::from_secs(120) + deadline, startup)
                        .await
                        .map_err(|_| Error::Timeout("L2 startup shutdown"))?
                        .map_err(|error| Error::Config(error.to_string()))?;
                }
                Ok::<_, Error>(())
            })
        });
        let result = self.flush_all().await;
        if let Some(close) = close {
            match close.await {
                Ok(Ok(())) => (),
                Ok(Err(error)) => eprintln!("disk cache close failed: {error}"),
                Err(error) => eprintln!("disk cache close task failed: {error}"),
            }
        }
        // Cancelled multipart tasks retain their admission token through abort.
        // Do not tear down the runtime while that bounded cleanup is in flight.
        let _idle = self
            .flushers
            .acquire_many(self.config.max_flushers as u32)
            .await
            .map_err(|_| Error::Cancelled)?;
        result?;
        if !self.lock_namespace().pending_moves.is_empty() {
            return Err(Error::Backend {
                operation: "shutdown",
                code: "PendingRename".into(),
                message: "a failed rename must be retried before successful shutdown".into(),
            });
        }
        Ok(())
    }
    pub fn is_closing(&self) -> bool {
        self.closing.load(Ordering::Acquire)
    }
}
