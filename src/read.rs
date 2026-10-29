use crate::{
    backend::{Store, s3::ObjectHead},
    budget::{Block, Budget, Reservation},
    error::{Error, Result},
    inode::Inode,
    write::{Snapshot, materialize},
};
use std::{collections::HashMap, ops::Range, sync::Arc};

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
    pub(crate) fn published(&self) {
        if let Some(fill) = self.node.read_fills.lock().unwrap().get(&self.key) {
            fill.done.send_replace(());
        }
    }
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
    Ok(Snapshot {
        key: state.data_key.as_ref().unwrap_or(&state.key).clone(),
        size: state.size,
        version: state.version,
        remote: state.remote.clone(),
        visible: state.visible_remote_size,
        extents: state.extents.clone(),
        metadata: HashMap::new(),
    })
}
