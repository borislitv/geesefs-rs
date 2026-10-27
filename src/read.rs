use crate::{
    backend::Store,
    budget::{Block, Budget},
    error::{Error, Result},
    inode::Inode,
    write::{Snapshot, materialize},
};
use std::{collections::HashMap, sync::Arc};

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
    pub fn prefetch(
        &mut self,
        offset: u64,
        length: usize,
        config: &crate::config::Config,
    ) -> usize {
        // Public Go getReadAhead/trackRead chooses from the preceding history,
        // then tracks this nonempty request regardless of cache hit or miss.
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
        length.max(kb.saturating_mul(1024))
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
