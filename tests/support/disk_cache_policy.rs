//! Development-only qualification adapter for Foyer's public policy hooks.
use foyer::{
    EvictionInfo, EvictionPicker, FifoPicker, Statistics, StorageFilterCondition,
    StorageFilterResult,
};
use lru::LruCache;
use std::{
    num::NonZeroUsize,
    sync::{Arc, Mutex},
};

#[derive(Debug)]
struct State {
    references: LruCache<u64, ()>,
    copy_remaining: usize,
}

#[derive(Clone, Debug)]
pub struct Recent {
    state: Arc<Mutex<State>>,
}

impl Recent {
    pub fn new(capacity: NonZeroUsize) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                references: LruCache::new(capacity),
                copy_remaining: 0,
            })),
        }
    }

    pub fn touch(&self, hash: u64) {
        self.state.lock().unwrap().references.put(hash, ());
    }

    pub fn picker(&self) -> impl EvictionPicker {
        CopyBoundedFifo {
            fifo: FifoPicker::default(),
            recent: self.clone(),
            copy_limit: 0,
        }
    }
}

impl StorageFilterCondition for Recent {
    fn filter(&self, _: &Arc<Statistics>, hash: u64, estimated_size: usize) -> StorageFilterResult {
        let mut state = self.state.lock().unwrap();
        if state.references.pop(&hash).is_none() {
            return StorageFilterResult::Reject;
        }
        if let Some(remaining) = state.copy_remaining.checked_sub(estimated_size) {
            state.copy_remaining = remaining;
            StorageFilterResult::Admit
        } else {
            StorageFilterResult::Reject
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
        self.fifo.on_block_evict(info, block);
        // This allowance is per reclaimed region, not an aggregate RAM bound.
        // The qualification fixture deliberately uses exactly one reclaimer.
        self.recent.state.lock().unwrap().copy_remaining = self.copy_limit;
    }
}
