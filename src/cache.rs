use moka::sync::Cache;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use crate::budget::{Block, Reservation};

/// Best-effort retained policy. Blocks remain charged to their allocation budget
/// after eviction while any reader pins them. Cache admission never reserves bytes.
type Key = (u64, u64, u64);
#[derive(Clone)]
struct Entry {
    block: Block,
    stamp: u64,
}
#[derive(Clone)]
pub struct ReadCache {
    entries: Cache<Key, Entry>,
    index: Arc<Mutex<BTreeMap<Key, u64>>>,
    sequence: Arc<AtomicU64>,
    entry_limit: usize,
}

impl ReadCache {
    pub fn new(capacity: u64) -> Self {
        Self::with_entry_limit(capacity, 100000)
    }
    pub fn with_entry_limit(capacity: u64, entry_limit: usize) -> Self {
        let index = Arc::new(Mutex::new(BTreeMap::<Key, u64>::new()));
        let evicted = index.clone();
        Self {
            index,
            sequence: Arc::new(AtomicU64::new(1)),
            entry_limit: entry_limit.max(1),
            entries: Cache::builder()
                .max_capacity(capacity)
                .weigher(|_, value: &Entry| {
                    u32::try_from(value.block.as_slice().len()).unwrap_or(u32::MAX)
                })
                .eviction_listener(move |key: Arc<Key>, value: Entry, _| {
                    let mut index = evicted.lock().unwrap();
                    if index.get(key.as_ref()) == Some(&value.stamp) {
                        index.remove(key.as_ref());
                    }
                })
                .build(),
        }
    }

    pub fn get(&self, inode: u64, generation: u64, offset: u64) -> Option<Block> {
        self.entries
            .get(&(inode, generation, offset))
            .map(|value| value.block)
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
                .index
                .lock()
                .unwrap()
                .range((inode, generation, 0)..=upper)
                .next_back()
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
        if let Some((block, relative)) = self.covering(inode, generation, offset, length) {
            return Some(block.slice(relative..relative + length));
        }
        let (mut block, mut relative) = self.covering(inode, generation, offset, 1)?;
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
        let stamp = self.sequence.fetch_add(1, Ordering::Relaxed);
        let victim = {
            let mut index = self.index.lock().unwrap();
            index.insert(key, stamp);
            if index.len() > self.entry_limit {
                index.pop_first().map(|(key, _)| key)
            } else {
                None
            }
        };
        if let Some(victim) = victim {
            self.entries.invalidate(&victim);
            if victim == key {
                return;
            }
        }
        self.entries.insert(key, Entry { block, stamp });
        self.entries.run_pending_tasks();
    }

    pub fn clear(&self) {
        self.entries.invalidate_all();
        self.entries.run_pending_tasks();
        self.index.lock().unwrap().clear();
    }
}
