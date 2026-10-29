use lru::LruCache;
use std::{
    collections::BTreeMap,
    num::NonZeroUsize,
    ops::Range,
    sync::{Arc, Mutex},
};

use crate::budget::{Block, Reservation};

/// Best-effort retained policy. Blocks remain charged to their allocation budget
/// after eviction while any reader pins them. Cache admission never reserves bytes.
type Key = (u64, u64, u64);

pub(crate) fn first_gap(
    mut range: Range<u64>,
    covered: impl Iterator<Item = Range<u64>>,
) -> Option<Range<u64>> {
    for span in covered {
        if span.start > range.start {
            return Some(range.start..span.start);
        }
        range.start = range.start.max(span.end);
        if range.start >= range.end {
            return None;
        }
    }
    Some(range)
}

struct State {
    entries: LruCache<Key, Block>,
    index: BTreeMap<Key, usize>,
    weight: u64,
}

impl State {
    fn removed(&mut self, key: Key, block: Block) {
        self.index.remove(&key);
        self.weight -= block.as_slice().len() as u64;
    }

    fn evict_lru(&mut self) -> bool {
        if let Some((key, block)) = self.entries.pop_lru() {
            self.removed(key, block);
            true
        } else {
            false
        }
    }
}

#[derive(Clone)]
pub struct ReadCache {
    state: Arc<Mutex<State>>,
    capacity: u64,
}

impl ReadCache {
    pub fn new(capacity: u64) -> Self {
        Self::with_entry_limit(capacity, 100000)
    }
    pub fn with_entry_limit(capacity: u64, entry_limit: usize) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                entries: LruCache::sparse(NonZeroUsize::new(entry_limit.max(1)).unwrap()),
                index: BTreeMap::new(),
                weight: 0,
            })),
            capacity,
        }
    }

    pub fn get(&self, inode: u64, generation: u64, offset: u64) -> Option<Block> {
        self.state
            .lock()
            .unwrap()
            .entries
            .get(&(inode, generation, offset))
            .cloned()
    }

    pub fn covering(
        &self,
        inode: u64,
        generation: u64,
        offset: u64,
        length: usize,
    ) -> Option<(Block, usize)> {
        if let Some(block) = self.get(inode, generation, offset)
            && block.as_slice().len() >= length
        {
            return Some((block, 0));
        }
        let mut upper = (inode, generation, offset);
        loop {
            let candidate = self
                .state
                .lock()
                .unwrap()
                .index
                .range((inode, generation, 0)..=upper)
                .rev()
                // Reject unusable spans without updating recency or cloning
                // payload owners. An older overlapping span may still cover it.
                .find(|&(&(_, _, start), span_length)| {
                    span_length
                        .checked_sub((offset - start) as usize)
                        .is_some_and(|available| available >= length)
                })
                .map(|(&key, _)| key);
            let Some((_, _, start)) = candidate else {
                break;
            };
            if let Some(block) = self.get(inode, generation, start) {
                let relative = (offset - start) as usize;
                if relative
                    .checked_add(length)
                    .is_some_and(|end| end <= block.as_slice().len())
                {
                    return Some((block, relative));
                }
            }
            if start == 0 {
                break;
            }
            upper = (inode, generation, start - 1);
        }
        None
    }

    /// Reuse one immutable view, or join adjacent fragments into admitted output.
    /// A gap remains a miss; bytes from different generations are never combined.
    pub(crate) fn read(
        &self,
        inode: u64,
        generation: u64,
        offset: u64,
        length: usize,
        reply: Reservation,
    ) -> Option<Block> {
        let (mut block, mut relative) = self.covering(inode, generation, offset, 1)?;
        if block.as_slice().len() - relative >= length {
            return Some(block.slice(relative..relative + length));
        }
        let mut result = reply.zeroed();
        let mut filled = 0;
        loop {
            let amount = (block.as_slice().len() - relative).min(length - filled);
            result.as_mut_slice()[filled..filled + amount]
                .copy_from_slice(&block.as_slice()[relative..relative + amount]);
            filled += amount;
            if filled == length {
                return Some(result);
            }
            (block, relative) = self.covering(inode, generation, offset + filled as u64, 1)?;
        }
    }

    pub fn insert(&self, inode: u64, generation: u64, offset: u64, block: Block) {
        if block.as_slice().is_empty() {
            return;
        }
        let key = (inode, generation, offset);
        let length = block.as_slice().len();
        let weight = length as u64;
        let mut state = self.state.lock().unwrap();
        if let Some(old) = state.entries.pop(&key) {
            state.removed(key, old);
        }
        if weight > self.capacity {
            return;
        }
        while state.weight > self.capacity - weight {
            assert!(state.evict_lru(), "positive cache weight owns entries");
        }
        if let Some((key, victim)) = state.entries.push(key, block) {
            state.removed(key, victim);
        }
        state.weight += weight;
        state.index.insert(key, length);
    }

    /// Release one cache owner; caller views retain their complete backing charge.
    pub(crate) fn evict_lru(&self) -> bool {
        self.state.lock().unwrap().evict_lru()
    }

    /// Speculation cannot fund itself by evicting this reader's unconsumed data.
    pub(crate) fn evict_consumed_lru(&self, inode: u64, generation: u64, consumed: u64) -> bool {
        let mut state = self.state.lock().unwrap();
        if state
            .entries
            .peek_lru()
            .is_some_and(|(&(ino, entry_generation, start), block)| {
                ino == inode
                    && entry_generation == generation
                    && start.saturating_add(block.as_slice().len() as u64) > consumed
            })
        {
            return false;
        }
        state.evict_lru()
    }

    pub(crate) fn contains(&self, inode: u64, generation: u64, offset: u64) -> bool {
        self.state
            .lock()
            .unwrap()
            .index
            .range((inode, generation, 0)..=(inode, generation, offset))
            .rev()
            .any(|(&(_, _, start), &length)| offset - start < length as u64)
    }

    /// Locate only the first missing interval without cloning or touching owners.
    pub(crate) fn gap(&self, inode: u64, generation: u64, range: Range<u64>) -> Option<Range<u64>> {
        let state = self.state.lock().unwrap();
        let covered = state
            .index
            .range((inode, generation, 0)..(inode, generation, range.end))
            .map(|(&(_, _, start), &length)| start..start.saturating_add(length as u64));
        first_gap(range, covered)
    }

    pub fn clear(&self) {
        let mut state = self.state.lock().unwrap();
        state.entries.clear();
        state.index.clear();
        state.weight = 0;
    }
}
