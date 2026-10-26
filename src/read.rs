use crate::{
    backend::Store,
    budget::{Block, Budget},
    error::{Error, Result},
    inode::Inode,
    write::{Snapshot, materialize},
};
use std::{collections::HashMap, sync::Arc};

#[derive(Default)]
pub struct Pattern {
    end: u64,
    sequential: u64,
    recent: std::collections::VecDeque<usize>,
}
impl Pattern {
    pub fn prefetch(
        &mut self,
        offset: u64,
        length: usize,
        config: &crate::config::Config,
    ) -> usize {
        self.sequential = if offset == self.end {
            self.sequential.saturating_add(length as u64)
        } else {
            length as u64
        };
        self.end = offset.saturating_add(length as u64);
        self.recent.push_back(length);
        while self.recent.len() > config.small_read_count {
            self.recent.pop_front();
        }
        let average = self.recent.iter().sum::<usize>() / self.recent.len().max(1);
        let kb = if self.sequential >= config.large_read_cutoff * 1024 {
            config.read_ahead_large
        } else if self.recent.len() == config.small_read_count
            && average <= config.small_read_cutoff * 1024
        {
            config.read_ahead_small
        } else {
            config.read_ahead
        };
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
