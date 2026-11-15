use crate::{
    backend::{Store, metadata::CachedRemote},
    budget::{Block, Budget, Reservation},
    error::{Error, Result},
    inode::Inode,
    write::{Snapshot, materialize},
};
use std::{
    ops::Range,
    sync::{Arc, Mutex},
};

#[cfg(test)]
#[path = "../tests/support/disk_cache_timeout_pipeline.rs"]
mod timeout_pipeline;

/// Logical payload unit shared by L2 and streamed S3 delivery.
pub const READ_BLOCK_BYTES: usize = 128 * 1024;
const _: () = assert!(READ_BLOCK_BYTES == crate::disk_cache::MAX_PAYLOAD_BYTES);

// Per active range producer: the one-entry Tokio channel (including its internal
// allocation block), three scalar run descriptors (active/queued/discovered), and
// the extra borrowed-future/channel state. Payload and disk jobs retain their
// separate existing reservations; this credit never funds another GET.
pub(crate) const GAP_PIPELINE_BYTES: usize = 4096;

/// Window eligibility decisions, separate from adapter/backend read outcomes.
#[derive(Clone, Copy, Debug)]
pub enum DiskDecision {
    Window,
    Closing,
    Unavailable,
    NamespaceIdentity,
    NamespaceContended,
    InodeContended,
    LockPoisoned,
    Deleted,
    Dirty,
    Pending,
    NotRegular,
    RemoteIdentity,
    Generation,
    Excluded,
    Eligible,
    Admission,
}
impl DiskDecision {
    pub const ALL: [Self; 16] = [
        Self::Window,
        Self::Closing,
        Self::Unavailable,
        Self::NamespaceIdentity,
        Self::NamespaceContended,
        Self::InodeContended,
        Self::LockPoisoned,
        Self::Deleted,
        Self::Dirty,
        Self::Pending,
        Self::NotRegular,
        Self::RemoteIdentity,
        Self::Generation,
        Self::Excluded,
        Self::Eligible,
        Self::Admission,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Window => "uncacheable_window",
            Self::Closing => "closing",
            Self::Unavailable => "unavailable",
            Self::NamespaceIdentity => "namespace_identity",
            Self::NamespaceContended => "namespace_contended",
            Self::InodeContended => "inode_contended",
            Self::LockPoisoned => "lock_poisoned",
            Self::Deleted => "deleted",
            Self::Dirty => "dirty",
            Self::Pending => "pending",
            Self::NotRegular => "not_regular",
            Self::RemoteIdentity => "remote_identity",
            Self::Generation => "generation",
            Self::Excluded => "excluded",
            Self::Eligible => "eligible",
            Self::Admission => "admission",
        }
    }
}

pub(crate) struct Window {
    pub snapshot: Arc<Snapshot>,
    pub range: Range<u64>,
    pub charge: Reservation,
    pub single_buffer: bool,
    pub cache_key: Option<(u64, u64)>,
    pub fill: FillGuard,
}

pub(crate) fn generation(version: u64, remote: Option<&CachedRemote>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    remote.map(|head| &head.etag).hash(&mut hash);
    remote.map(|head| head.size).hash(&mut hash);
    version.hash(&mut hash);
    hash.finish()
}

pub(crate) struct Fill {
    end: u64,
    chunks: Arc<Mutex<Vec<(u64, Block)>>>,
    progress: tokio::sync::watch::Sender<FillProgress>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum FillProgress {
    Admitting,
    Reading,
    Done,
}

pub(crate) enum FillAdmission {
    Leader(FillGuard),
    Wait(FillWait),
}

pub(crate) struct FillGuard {
    node: Arc<Inode>,
    key: (u64, u64),
    chunks: Arc<Mutex<Vec<(u64, Block)>>>,
    progress: tokio::sync::watch::Sender<FillProgress>,
}

/// Only subscribed active consumers can retain a completed producer's chunks.
/// No entry survives in the inode registry after its producer finishes.
pub(crate) struct FillWait {
    chunks: Arc<Mutex<Vec<(u64, Block)>>>,
    pub progress: tokio::sync::watch::Receiver<FillProgress>,
}

impl FillWait {
    pub(crate) fn read(
        &self,
        offset: u64,
        length: usize,
        reply: &mut Reservation,
    ) -> Option<Block> {
        let chunks = self.chunks.lock().unwrap();
        let find = |at| {
            chunks
                .iter()
                .find(|(start, block)| *start <= at && at - *start < block.as_slice().len() as u64)
        };
        // Verify complete coverage before allocating or exposing any bytes.
        let end = offset + length as u64;
        let mut at = offset;
        while at < end {
            let (start, block) = find(at)?;
            at = end.min(*start + block.as_slice().len() as u64);
        }
        let (start, block) = find(offset)?;
        let relative = (offset - *start) as usize;
        if block.as_slice().len() - relative >= length {
            return Some(block.slice(relative..relative + length));
        }
        let mut output = reply.split(length).zeroed();
        let mut copied = 0;
        while copied < length {
            let (start, block) = find(offset + copied as u64)?;
            let relative = (offset + copied as u64 - *start) as usize;
            let amount = (length - copied).min(block.as_slice().len() - relative);
            output.as_mut_slice()[copied..copied + amount]
                .copy_from_slice(&block.as_slice()[relative..relative + amount]);
            copied += amount;
        }
        Some(output)
    }
}

/// Coalesce overlapping windows of one generation, without serializing disjoint
/// readers. Completed chunks notify followers; the owned interval closes its
/// channel on success, error or cancellation.
pub(crate) fn start_fill(
    node: &Arc<Inode>,
    generation: u64,
    start: u64,
    end: u64,
) -> FillAdmission {
    let mut fills = node.read_fills.lock().unwrap();
    for (&(_, at), fill) in fills.range((generation, 0)..=(generation, u64::MAX)) {
        if at < end && start < fill.end {
            return FillAdmission::Wait(FillWait {
                chunks: fill.chunks.clone(),
                progress: fill.progress.subscribe(),
            });
        }
    }
    let key = (generation, start);
    let (progress, _) = tokio::sync::watch::channel(FillProgress::Admitting);
    let chunks = Arc::new(Mutex::new(Vec::new()));
    fills.insert(
        key,
        Fill {
            end,
            chunks: chunks.clone(),
            progress: progress.clone(),
        },
    );
    FillAdmission::Leader(FillGuard {
        node: node.clone(),
        key,
        chunks,
        progress,
    })
}

impl FillGuard {
    pub(crate) fn node(&self) -> &Arc<Inode> {
        &self.node
    }
    pub(crate) fn admitted(&self) {
        self.progress.send_replace(FillProgress::Reading);
    }
    pub(crate) fn publish(&self, at: u64, block: Block) {
        self.chunks.lock().unwrap().push((at, block));
        self.progress.send_replace(FillProgress::Reading);
    }
}

#[derive(Clone)]
pub(crate) struct DiskRead {
    pub cache: crate::disk_cache::DiskCache,
    pub namespace: [u8; 32],
}

pub(crate) fn current(node: &Inode, expected: u64) -> bool {
    let state = node.state.lock().unwrap();
    !state.deleted && generation(state.version, state.remote.as_ref()) == expected
}

/// Exact logical block coverage, including the short EOF block. No source GET
/// is extended to fill a disk record.
pub(crate) fn block_end(at: u64, size: u64) -> u64 {
    (at / READ_BLOCK_BYTES as u64 + 1)
        .saturating_mul(READ_BLOCK_BYTES as u64)
        .min(size)
}

pub(crate) fn disk_coverage(start: u64, end: u64, size: u64) -> usize {
    if end <= start {
        return 0;
    }
    (block_end(end - 1, size) - start / READ_BLOCK_BYTES as u64 * READ_BLOCK_BYTES as u64) as usize
}

pub(crate) fn disk_window_admission(start: u64, end: u64, size: u64) -> usize {
    let bytes = disk_coverage(start, end, size);
    bytes + bytes.div_ceil(READ_BLOCK_BYTES) * crate::disk_cache::READ_WAIT_BYTES
}

pub(crate) struct DiskRange {
    pub disk: DiskRead,
    pub node: Arc<Inode>,
    pub generation: u64,
    pub admission: Reservation,
    pub deadline: std::time::Duration,
}

pub(crate) fn debug_disk_fallback(
    cache: &crate::disk_cache::DiskCache,
    reason: crate::disk_cache::ReadFallback,
) {
    if let Some(trace) = crate::metrics::context() {
        use crate::disk_cache::ReadFallback;
        if matches!(
            reason,
            ReadFallback::Poisoned
                | ReadFallback::Join
                | ReadFallback::BackendError
                | ReadFallback::Identity
                | ReadFallback::OutputCopy
                | ReadFallback::OutputCoverage
        ) {
            trace.cache_warning(reason.label());
        }
        trace.debug("l2_read_fallback", || {
            let stats = cache.stats();
            serde_json::json!({"reason": reason.label(), "state": format!("{:?}", stats.state),
                "active_jobs": stats.active_jobs, "active_read_jobs": stats.active_read_jobs,
                "active_spill_jobs": stats.active_spill_jobs, "queued_reads": stats.queued_reads})
        });
    }
}

/// Disk output transfers the existing fill reservation. The separate, already
/// reserved source output remains available on a timed-out disk read: its owned
/// I/O may still hold the first reservation. Missing adjacent blocks share a GET.
/// Discovery and one sequential source consumer overlap in this same task; only
/// one additional run may queue, without per-hole tasks or retained payloads.
pub(crate) async fn load_disk_range(
    store: &Arc<dyn Store>,
    snapshot: &Snapshot,
    span: Range<u64>,
    local: &Budget,
    output: &crate::backend::RangeSink,
    sender: &tokio::sync::mpsc::Sender<(u64, Block)>,
    mut range: DiskRange,
) -> Result<()> {
    let (start, end) = (span.start, span.end);
    let remote = snapshot.remote.as_ref().expect("qualified clean range");
    let (gaps, mut missing_ranges) = tokio::sync::mpsc::channel::<Range<u64>>(1);
    let discover = async move {
        let mut next = start;
        let mut missing = start;
        while next < end {
            let full_start = next / READ_BLOCK_BYTES as u64 * READ_BLOCK_BYTES as u64;
            let full_end = block_end(next, snapshot.size);
            let segment_end = full_end.min(end);
            let full_len = (full_end - full_start) as usize;
            let mut block_admission = range
                .admission
                .split(full_len + crate::disk_cache::READ_WAIT_BYTES);
            // The queued state holds only already-funded window/canonical owners,
            // never a newly allocated DiskKey. Active read-work credit bounds keys.
            let slot = if current(&range.node, range.generation) {
                match range
                    .disk
                    .cache
                    .read_admission(
                        block_admission.split(crate::disk_cache::READ_WAIT_BYTES),
                        range.deadline,
                    )
                    .await
                {
                    Ok(slot) if current(&range.node, range.generation) => Some(slot),
                    Ok(_) => {
                        range.disk.cache.record_read_fallback(
                            crate::disk_cache::ReadFallback::GenerationBefore,
                        );
                        debug_disk_fallback(
                            &range.disk.cache,
                            crate::disk_cache::ReadFallback::GenerationBefore,
                        );
                        None
                    }
                    Err(reason) => {
                        debug_disk_fallback(&range.disk.cache, reason);
                        if matches!(
                            reason,
                            crate::disk_cache::ReadFallback::Timeout
                                | crate::disk_cache::ReadFallback::Jobs
                        ) {
                            // Optional-tier saturation must not renew a full
                            // wait for every remaining block before source GET.
                            break;
                        }
                        None
                    }
                }
            } else {
                range
                    .disk
                    .cache
                    .record_read_fallback(crate::disk_cache::ReadFallback::GenerationBefore);
                debug_disk_fallback(
                    &range.disk.cache,
                    crate::disk_cache::ReadFallback::GenerationBefore,
                );
                None
            };
            let key = slot.as_ref().and_then(|_| {
                let full = crate::disk_cache::DiskKey::new(
                    &range.disk.namespace,
                    snapshot.key.as_bytes(),
                    remote.etag.as_bytes(),
                    snapshot.size,
                    full_start,
                    full_len,
                )
                .ok();
                if let Some(full) = full
                    && (range.disk.cache.touch_validated(&full)
                        || (full_start == next && full_len == (segment_end - next) as usize))
                {
                    return Some(full);
                }
                crate::disk_cache::DiskKey::new(
                    &range.disk.namespace,
                    snapshot.key.as_bytes(),
                    remote.etag.as_bytes(),
                    snapshot.size,
                    next,
                    (segment_end - next) as usize,
                )
                .ok()
            });
            if slot.is_some() && key.is_none() {
                range
                    .disk
                    .cache
                    .record_read_fallback(crate::disk_cache::ReadFallback::Identity);
                debug_disk_fallback(&range.disk.cache, crate::disk_cache::ReadFallback::Identity);
            }
            let relative = key.as_ref().map_or(0, |key| (next - key.offset()) as usize);
            let block = if let (Some(slot), Some(key)) = (slot, key) {
                let coverage = key.covered_len();
                let result = slot.read_owned(key, block_admission.split(coverage)).await;
                if let Err(reason) = result.as_ref() {
                    debug_disk_fallback(&range.disk.cache, *reason);
                    if matches!(reason, crate::disk_cache::ReadFallback::Timeout) {
                        break; // The actual I/O retains its slot and funding.
                    }
                }
                result.ok()
            } else {
                None
            };
            let block = block.filter(|_| {
                if current(&range.node, range.generation) {
                    true
                } else {
                    range
                        .disk
                        .cache
                        .record_read_fallback(crate::disk_cache::ReadFallback::GenerationAfter);
                    debug_disk_fallback(
                        &range.disk.cache,
                        crate::disk_cache::ReadFallback::GenerationAfter,
                    );
                    false
                }
            });
            if let Some(block) = block {
                // Even a full descriptor queue must not hold an already
                // verified hit behind an unrelated source body.
                sender
                    .send((
                        next,
                        block
                            .with_disk_origin()
                            .slice(relative..relative + (segment_end - next) as usize),
                    ))
                    .await
                    .map_err(|_| Error::Cancelled)?;
                if missing < next {
                    gaps.send(missing..next)
                        .await
                        .map_err(|_| Error::Cancelled)?;
                }
                missing = segment_end;
            }
            next = segment_end;
        }
        if missing < end {
            gaps.send(missing..end)
                .await
                .map_err(|_| Error::Cancelled)?;
        }
        drop(gaps);
        Ok::<_, Error>(())
    };
    let fetch = async {
        while let Some(missing) = missing_ranges.recv().await {
            store
                .read_range_chunks(
                    &snapshot.key,
                    missing.start,
                    (missing.end - missing.start) as usize,
                    &remote.read_head(),
                    local,
                    output,
                )
                .await?;
        }
        Ok::<_, Error>(())
    };
    // The first observed error drops the other borrowed future, including its
    // queued descriptors or active source body. Cancellation drops both too.
    tokio::try_join!(discover, fetch)?;
    Ok(())
}

impl Drop for FillGuard {
    fn drop(&mut self) {
        self.node.read_fills.lock().unwrap().remove(&self.key);
        self.progress.send_replace(FillProgress::Done);
    }
}

#[derive(Default)]
pub struct Pattern {
    end: u64,
    sequential: u64,
    planned: Option<(u64, u64, u64)>,
    /// Completed contiguous runs, not individual kernel/FUSE read sizes.
    recent: std::collections::VecDeque<u64>,
}
impl Pattern {
    /// A bounded interval hint, not evidence that bytes are cached. Required
    /// bytes always consult Foyer/source. Avoid sliding a new speculative window
    /// forward on every small read after a producer has already planned its tail.
    pub(crate) fn persistent_window(
        &mut self,
        generation: u64,
        start: u64,
        required: usize,
        planned: usize,
    ) -> usize {
        if self
            .planned
            .is_some_and(|(old, begin, end)| old == generation && begin <= start && start < end)
        {
            return required;
        }
        self.planned = Some((generation, start, start.saturating_add(planned as u64)));
        planned
    }
    pub(crate) fn window(&self, config: &crate::config::Config) -> usize {
        let average = (u128::from(self.sequential)
            + self.recent.iter().copied().map(u128::from).sum::<u128>())
            / (1 + self.recent.len() as u128);
        let kb = if self.sequential >= config.large_read_cutoff * 1024 {
            config.read_ahead_large
        } else if !self.recent.is_empty() && average <= config.small_read_cutoff as u128 * 1024 {
            config.read_ahead_small
        } else {
            config.read_ahead
        };
        kb.saturating_mul(1024)
    }

    pub fn prefetch(
        &mut self,
        offset: u64,
        length: usize,
        config: &crate::config::Config,
    ) -> usize {
        // Public Go getReadAhead/trackRead chooses from the preceding history,
        // then tracks this nonempty request regardless of cache hit or miss.
        let window = self.window(config);
        if length > 0 {
            if offset == self.end {
                self.sequential = self.sequential.saturating_add(length as u64);
            } else {
                if config.small_read_count > 1 {
                    self.recent.push_back(self.sequential);
                    while self.recent.len() >= config.small_read_count {
                        self.recent.pop_front();
                    }
                }
                self.sequential = length as u64;
            }
            self.end = offset.saturating_add(length as u64);
        }
        length.max(window)
    }
}

pub async fn read(
    store: &Arc<dyn Store>,
    node: &Inode,
    offset: u64,
    length: usize,
    budget: &Budget,
) -> Result<Block> {
    let snapshot = snapshot(node)?;
    let length = (snapshot.size.saturating_sub(offset)).min(length as u64) as usize;
    materialize(store, &snapshot, offset, length, budget).await
}

pub fn snapshot(node: &Inode) -> Result<Snapshot> {
    let state = node.state.lock().unwrap();
    if state.deleted {
        return Err(Error::Fs(libc::ENOENT));
    }
    let charge = state
        .metadata_budget()
        .charge_with(|| 4 * state.metadata_bytes())?;
    let mut snapshot = Snapshot {
        metadata_charge: charge,
        key: state.data_key.as_ref().unwrap_or(&state.key).clone(),
        size: state.size,
        version: state.version,
        remote: state.remote.clone(),
        visible: state.visible_remote_size,
        extents: state.extents.clone(),
        metadata: state.metadata.clone(),
    };
    snapshot.settle_metadata()?;
    Ok(snapshot)
}
