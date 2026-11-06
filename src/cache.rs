use lru::LruCache;
use std::{
    collections::{BTreeMap, BTreeSet},
    num::NonZeroUsize,
    ops::Range,
    sync::{Arc, Mutex},
};

use crate::budget::{Block, Budget, Reservation};

/// Best-effort retained policy. Blocks remain charged to their allocation budget
/// after eviction while any reader pins them. Insertion never reserves bytes;
/// idle-lane loans only transfer an exclusively owned allocation's existing charge.
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

struct Span {
    length: usize,
    consumed: usize,
}

struct State {
    entries: LruCache<Key, Block>,
    index: BTreeMap<Key, Span>,
    weight: u64,
    loaned: usize,
    loans: BTreeSet<Key>,
    evictions: u64,
    evicted_bytes: u64,
    insertions: u64,
    replacements: u64,
    oversized: u64,
}

/// Retained visible spans; byte permits may outlive eviction in pinned readers.
#[derive(Clone, Debug)]
pub struct CacheStats {
    pub entries: usize,
    pub bytes: u64,
    pub capacity: u64,
    pub entry_limit: usize,
    pub loaned_bytes: usize,
    pub evictions: u64,
    pub evicted_bytes: u64,
    pub insertions: u64,
    pub replacements: u64,
    pub oversized: u64,
}

impl State {
    fn covering(
        &mut self,
        inode: u64,
        generation: u64,
        offset: u64,
        length: usize,
    ) -> Option<(Block, usize)> {
        let exact = (inode, generation, offset);
        let key = if self
            .entries
            .peek(&exact)
            .is_some_and(|block| block.as_slice().len() >= length)
        {
            exact
        } else {
            *self
                .index
                .range((inode, generation, 0)..=exact)
                .rev()
                .find(|&(&(_, _, start), span)| {
                    span.length
                        .checked_sub((offset - start) as usize)
                        .is_some_and(|available| available >= length)
                })?
                .0
        };
        let relative = (offset - key.2) as usize;
        Some((
            self.entries
                .get(&key)
                .expect("range index owns a cache entry")
                .clone(),
            relative,
        ))
    }

    fn consumed(&mut self, key: Key, relative: usize, length: usize) {
        if let Some(span) = self.index.get_mut(&key)
            && relative <= span.consumed
        {
            span.consumed = span
                .consumed
                .max(relative.saturating_add(length).min(span.length));
        }
    }

    fn removed(&mut self, key: Key, block: Block) {
        self.index.remove(&key);
        self.weight -= block.as_slice().len() as u64;
        if block.is_loaned() {
            self.loaned -= block.as_slice().len();
            self.loans.remove(&key);
        }
    }

    fn evict_lru(&mut self) -> bool {
        if let Some((key, block)) = self.entries.pop_lru() {
            self.evicted(key, block);
            true
        } else {
            false
        }
    }

    fn evicted(&mut self, key: Key, block: Block) {
        self.evictions += 1;
        self.evicted_bytes += block.as_slice().len() as u64;
        self.removed(key, block);
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
                loaned: 0,
                loans: BTreeSet::new(),
                evictions: 0,
                evicted_bytes: 0,
                insertions: 0,
                replacements: 0,
                oversized: 0,
            })),
            capacity,
        }
    }

    /// Scrapes never wait behind a cache operation or change recency.
    pub fn try_stats(&self) -> Option<CacheStats> {
        let state = self.state.try_lock().ok()?;
        Some(CacheStats {
            entries: state.entries.len(),
            bytes: state.weight,
            capacity: self.capacity,
            entry_limit: state.entries.cap().get(),
            loaned_bytes: state.loaned,
            evictions: state.evictions,
            evicted_bytes: state.evicted_bytes,
            insertions: state.insertions,
            replacements: state.replacements,
            oversized: state.oversized,
        })
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
        self.state
            .lock()
            .unwrap()
            .covering(inode, generation, offset, length)
    }

    fn read_span(
        &self,
        inode: u64,
        generation: u64,
        offset: u64,
        length: usize,
    ) -> Option<(Block, usize)> {
        let mut state = self.state.lock().unwrap();
        let (block, relative) = state.covering(inode, generation, offset, 1)?;
        let amount = (block.as_slice().len() - relative).min(length);
        state.consumed(
            (inode, generation, offset - relative as u64),
            relative,
            amount,
        );
        Some((block, relative))
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
        let (mut block, mut relative) = self.read_span(inode, generation, offset, length)?;
        if block.as_slice().len() - relative >= length && !block.is_loaned() {
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
            (block, relative) =
                self.read_span(inode, generation, offset + filled as u64, length - filled)?;
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
            state.replacements += 1;
            state.removed(key, old);
        }
        if weight > self.capacity {
            state.oversized += 1;
            return;
        }
        while state.weight > self.capacity - weight {
            assert!(state.evict_lru(), "positive cache weight owns entries");
        }
        if let Some((key, victim)) = state.entries.push(key, block) {
            state.evicted(key, victim);
        }
        state.insertions += 1;
        state.weight += weight;
        state.index.insert(
            key,
            Span {
                length,
                consumed: 0,
            },
        );
    }

    /// A bounded eviction hint, not a pin: disjoint tail reads cannot advance an
    /// unread prefix, and required admission may still reclaim any LRU owner.
    pub(crate) fn consumed(
        &self,
        inode: u64,
        generation: u64,
        start: u64,
        relative: usize,
        length: usize,
    ) {
        self.state
            .lock()
            .unwrap()
            .consumed((inode, generation, start), relative, length);
    }

    /// Release one cache owner; caller views retain their complete backing charge.
    pub(crate) fn evict_lru(&self) -> bool {
        self.state.lock().unwrap().evict_lru()
    }

    /// Keep LRU bytes by moving only their exclusive backing's existing charge.
    pub(crate) fn loan_lru(&self, lane: &Budget) -> bool {
        let mut state = self.state.lock().unwrap();
        let loaned = state
            .entries
            .iter_mut()
            .rev()
            .find(|(_, block)| !block.is_loaned())
            .and_then(|(key, block)| {
                block
                    .try_loan(lane)
                    .then_some((*key, block.as_slice().len()))
            });
        if let Some((key, length)) = loaned {
            state.loaned += length;
            state.loans.insert(key);
            true
        } else {
            false
        }
    }

    pub(crate) fn loaned_bytes(&self) -> usize {
        self.state.lock().unwrap().loaned
    }

    /// Upload activity excludes new loans. A concurrent synchronous cache copy
    /// may briefly retain a loaned backing; byte admission waits for its release.
    /// Cache reads never export a loaned owner or await while retaining one.
    pub(crate) fn reclaim_loans(&self) {
        let mut state = self.state.lock().unwrap();
        if state.loaned == 0 {
            return;
        }
        while let Some(key) = state.loans.first().copied() {
            let block = state.entries.pop(&key).expect("loaned cache owner");
            state.evicted(key, block);
        }
    }

    /// Speculation cannot fund itself with another reader's unconsumed data.
    pub(crate) fn evict_consumed_lru(&self, inode: u64, generation: u64, consumed: u64) -> bool {
        let mut state = self.state.lock().unwrap();
        if state
            .entries
            .peek_lru()
            .is_some_and(|(&(ino, entry_generation, start), block)| {
                if ino == inode && entry_generation == generation {
                    start.saturating_add(block.as_slice().len() as u64) > consumed
                } else {
                    let span = &state.index[&(ino, entry_generation, start)];
                    span.consumed < span.length
                }
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
            .any(|(&(_, _, start), span)| offset - start < span.length as u64)
    }

    /// Locate only the first missing interval without cloning or touching owners.
    pub(crate) fn gap(&self, inode: u64, generation: u64, range: Range<u64>) -> Option<Range<u64>> {
        let state = self.state.lock().unwrap();
        let covered = state
            .index
            .range((inode, generation, 0)..(inode, generation, range.end))
            .map(|(&(_, _, start), span)| start..start.saturating_add(span.length as u64));
        first_gap(range, covered)
    }

    pub fn clear(&self) {
        let mut state = self.state.lock().unwrap();
        state.entries.clear();
        state.index.clear();
        state.weight = 0;
        state.loaned = 0;
        state.loans.clear();
    }
}
