//! Bounded second-chance hints using Foyer's supported region/reinsertion hooks.
use foyer::{
    EvictionInfo, EvictionPicker, FifoPicker, Statistics, StorageFilterCondition,
    StorageFilterResult,
};
use lru::LruCache;
use std::{
    num::NonZeroUsize,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

#[derive(Clone, Debug)]
pub(crate) struct Recent(Arc<RecentState>);
#[derive(Debug)]
struct RecentState {
    references: Mutex<References>,
    region_reclaims_started: AtomicU64,
}
#[derive(Debug)]
struct References {
    hints: LruCache<u64, ()>,
    remaining: usize,
}
impl Recent {
    pub(crate) fn new(capacity: usize) -> Self {
        Self(Arc::new(RecentState {
            references: Mutex::new(References {
                hints: LruCache::new(NonZeroUsize::new(capacity).unwrap()),
                remaining: 0,
            }),
            region_reclaims_started: AtomicU64::new(0),
        }))
    }
    pub(crate) fn touch(&self, hash: u64) {
        self.0.references.lock().unwrap().hints.put(hash, ());
    }
    pub(crate) fn len(&self) -> usize {
        self.0.references.lock().unwrap().hints.len()
    }
    pub(crate) fn region_reclaims_started(&self) -> u64 {
        self.0.region_reclaims_started.load(Ordering::Relaxed)
    }
    pub(crate) fn picker(&self) -> impl EvictionPicker {
        CopyBoundedFifo {
            fifo: FifoPicker::default(),
            recent: self.clone(),
            copy_limit: 0,
        }
    }
}
impl StorageFilterCondition for Recent {
    fn filter(&self, _: &Arc<Statistics>, hash: u64, size: usize) -> StorageFilterResult {
        if size == 0 || size > super::key::RECORD_CAP {
            return StorageFilterResult::Reject;
        }
        let mut state = self.0.references.lock().unwrap();
        if state.hints.pop(&hash).is_none() {
            return StorageFilterResult::Reject;
        }
        // The reclaimer reads a page-rounded backing even for a tiny record.
        // Bound actual copy I/O, not just useful bytes in a partial payload.
        match state.remaining.checked_sub(super::funding::rounded(size)) {
            Some(left) => {
                state.remaining = left;
                StorageFilterResult::Admit
            }
            None => StorageFilterResult::Reject,
        }
    }
}
#[derive(Debug)]
struct CopyBoundedFifo {
    fifo: FifoPicker,
    recent: Recent,
    copy_limit: usize,
}
impl EvictionPicker for CopyBoundedFifo {
    fn init(&mut self, blocks: &[u32], block_size: usize) {
        self.copy_limit = block_size / 4;
        self.fifo.init(blocks, block_size);
    }
    fn pick(&mut self, info: EvictionInfo<'_>) -> Option<u32> {
        self.fifo.pick(info)
    }
    fn on_block_evictable(&mut self, info: EvictionInfo<'_>, block: u32) {
        self.fifo.on_block_evictable(info, block);
    }
    fn on_block_evict(&mut self, info: EvictionInfo<'_>, block: u32) {
        self.recent
            .0
            .region_reclaims_started
            .fetch_add(1, Ordering::Relaxed);
        self.fifo.on_block_evict(info, block);
        // Exactly one engine reclaimer makes this a per-region quota. The shared
        // SubmissionLimits additionally bounds live raw-copy bytes/items.
        self.recent.0.references.lock().unwrap().remaining = self.copy_limit;
    }
}
