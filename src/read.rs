use crate::{
    backend::{Store, s3::ObjectHead},
    budget::{Block, Budget, Reservation},
    error::{Error, Result},
    inode::Inode,
    write::{Snapshot, materialize},
};
use std::{collections::HashMap, ops::Range, sync::Arc};

/// Logical payload unit shared by L1/L2 and streamed S3 delivery.
pub const READ_BLOCK_BYTES: usize = 128 * 1024;
const _: () = assert!(READ_BLOCK_BYTES == crate::disk_cache::MAX_PAYLOAD_BYTES);

pub(crate) struct Window {
    pub snapshot: Arc<Snapshot>,
    pub range: Range<u64>,
    pub charge: Reservation,
    pub single_buffer: bool,
    pub cache_key: Option<(u64, u64)>,
    pub fill: FillGuard,
}

pub(crate) fn generation(version: u64, remote: Option<&ObjectHead>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    remote.map(|head| &head.etag).hash(&mut hash);
    remote.map(|head| head.size).hash(&mut hash);
    version.hash(&mut hash);
    hash.finish()
}

pub(crate) fn refill_state(node: &Inode, consumed: u64) -> Option<(u64, u64, u64)> {
    let state = node.state.lock().unwrap();
    if state.deleted
        || state.version != state.durable
        || state.remote.is_none()
        || !state.extents.is_empty()
        || state.visible_remote_size < state.size
        || consumed >= state.size
    {
        return None;
    }
    Some((
        state.size,
        generation(state.version, state.remote.as_ref()),
        state.version,
    ))
}

pub(crate) struct Fill {
    end: u64,
    done: tokio::sync::watch::Sender<()>,
}

pub(crate) enum FillAdmission {
    Leader(FillGuard),
    Wait(tokio::sync::watch::Receiver<()>),
}

pub(crate) struct FillGuard {
    node: Arc<Inode>,
    key: (u64, u64),
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
            return FillAdmission::Wait(fill.done.subscribe());
        }
    }
    let key = (generation, start);
    let (done, _) = tokio::sync::watch::channel(());
    fills.insert(key, Fill { end, done });
    FillAdmission::Leader(FillGuard {
        node: node.clone(),
        key,
    })
}

pub(crate) fn fill_covers(node: &Inode, generation: u64, offset: u64) -> bool {
    node.read_fills
        .lock()
        .unwrap()
        .range((generation, 0)..=(generation, offset))
        .next_back()
        .is_some_and(|(_, fill)| offset < fill.end)
}

pub(crate) fn unfilled(node: &Inode, generation: u64, range: Range<u64>) -> Option<Range<u64>> {
    let fills = node.read_fills.lock().unwrap();
    let covered = fills
        .range((generation, 0)..(generation, range.end))
        .map(|(&(_, start), fill)| start..fill.end);
    crate::cache::first_gap(range, covered)
}

impl FillGuard {
    pub(crate) fn node(&self) -> &Arc<Inode> {
        &self.node
    }
    pub(crate) fn published(&self) {
        if let Some(fill) = self.node.read_fills.lock().unwrap().get(&self.key) {
            fill.done.send_replace(());
        }
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

pub(crate) struct DiskRange {
    pub disk: DiskRead,
    pub node: Arc<Inode>,
    pub generation: u64,
    pub admission: Reservation,
    pub deadline: std::time::Duration,
}

/// Disk output transfers the existing fill reservation. The separate, already
/// reserved source output remains available on a timed-out disk read: its owned
/// I/O may still hold the first reservation. Missing adjacent blocks share a GET.
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
    let mut next = start;
    let mut missing = start;
    while next < end {
        let full_start = next / READ_BLOCK_BYTES as u64 * READ_BLOCK_BYTES as u64;
        let full_end = block_end(next, snapshot.size);
        let segment_end = full_end.min(end);
        let full_len = (full_end - full_start) as usize;
        let full = crate::disk_cache::DiskKey::new(
            &range.disk.namespace,
            snapshot.key.as_bytes(),
            remote.etag.as_bytes(),
            snapshot.size,
            full_start,
            full_len,
        )
        .ok();
        let key = full
            .filter(|key| range.disk.cache.touch_validated(key))
            .or_else(|| {
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
        let mut block_admission = range.admission.split(full_len);
        let block = if current(&range.node, range.generation) {
            if let Some(key) = &key {
                range
                    .disk
                    .cache
                    .read_block_reserved(
                        key,
                        block_admission.split(key.covered_len()),
                        range.deadline,
                    )
                    .await
            } else {
                None
            }
        } else {
            None
        };
        if let Some(block) = block.filter(|_| current(&range.node, range.generation)) {
            let relative = (next - key.as_ref().unwrap().offset()) as usize;
            // Identity storage belongs to the bounded disk lookup, not to a
            // possibly blocked source GET or the retained L1 entry.
            drop(key);
            if missing < next {
                store
                    .read_range_chunks(
                        &snapshot.key,
                        missing,
                        (next - missing) as usize,
                        remote,
                        local,
                        output,
                    )
                    .await?;
            }
            sender
                .send((
                    next,
                    block
                        .with_disk_origin()
                        .slice(relative..relative + (segment_end - next) as usize),
                ))
                .await
                .map_err(|_| Error::Cancelled)?;
            missing = segment_end;
        }
        next = segment_end;
    }
    if missing < end {
        store
            .read_range_chunks(
                &snapshot.key,
                missing,
                (end - missing) as usize,
                remote,
                local,
                output,
            )
            .await?;
    }
    Ok(())
}

impl Drop for FillGuard {
    fn drop(&mut self) {
        self.node.read_fills.lock().unwrap().remove(&self.key);
    }
}

#[derive(Default)]
pub struct Pattern {
    end: u64,
    sequential: u64,
    /// Completed contiguous runs, not individual kernel/FUSE read sizes.
    recent: std::collections::VecDeque<u64>,
}
impl Pattern {
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
        metadata: HashMap::new(),
    };
    snapshot.settle_metadata()?;
    Ok(snapshot)
}
