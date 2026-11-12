use crate::{
    error::{Error, Result},
    inode::{Inode, State},
    metadata_budget::{LINKS, MetadataCharge},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex, Weak, atomic::Ordering},
    time::{Duration, Instant},
};

pub(crate) type InodeIndex = papaya::HashMap<u64, Weak<Inode>>;

pub struct Namespace {
    pub nodes: BTreeMap<u64, Arc<Inode>>,
    pub(crate) inodes: Arc<InodeIndex>,
    dirty: BTreeMap<crate::metadata_budget::SharedKey, u64>,
    pub names: BTreeMap<crate::metadata_budget::SharedKey, u64>,
    pub tombstones: BTreeMap<String, Instant>,
    pub epoch: u64,
    pub moving: BTreeSet<String>,
    /// Two prefixes per pending live inode, retained across failed move attempts.
    pub pending_moves: BTreeMap<String, u64>,
    /// Ambiguous/cancelled DELETEs retain their old name until an explicit retry.
    pub pending_deletes: BTreeMap<String, u64>,
    pub evictions: u64,
    pub capacity_failures: u64,
    next_ino: u64,
    reclaim_cursor: u64,
    limit: usize,
    operation_charge: MetadataCharge,
}

/// Admission-local continuation: concurrent admissions must not advance each
/// other's scan. Each admission starts after the last successful victim, wraps
/// once, and retains its own high-water mark while other callers install nodes.
#[derive(Default)]
pub(crate) struct ReclaimScan {
    cursor: u64,
    end: Option<u64>,
    wrap_end: u64,
}

pub(crate) enum Reclaim {
    Removed,
    More,
    Exhausted,
}

impl Namespace {
    pub fn new(root: Inode, limit: usize) -> Self {
        let operation_charge = root
            .state
            .lock()
            .unwrap()
            .metadata_budget()
            .charge(0)
            .expect("empty namespace operation charge");
        let root_key = root.state.lock().unwrap().key.clone();
        let root = Arc::new(root);
        let inodes = Arc::new(InodeIndex::new());
        inodes.pin().insert(1, Arc::downgrade(&root));
        Self {
            nodes: BTreeMap::from([(1, root)]),
            inodes,
            dirty: BTreeMap::new(),
            names: BTreeMap::from([(root_key, 1)]),
            tombstones: BTreeMap::new(),
            epoch: 0,
            moving: BTreeSet::new(),
            pending_moves: BTreeMap::new(),
            pending_deletes: BTreeMap::new(),
            evictions: 0,
            capacity_failures: 0,
            next_ino: 2,
            reclaim_cursor: 0,
            limit,
            operation_charge,
        }
    }

    pub fn get(&self, ino: u64) -> Result<Arc<Inode>> {
        self.nodes.get(&ino).cloned().ok_or(Error::Fs(libc::ENOENT))
    }

    pub fn named(&self, key: &str) -> Option<Arc<Inode>> {
        self.names.get(key).and_then(|i| self.nodes.get(i)).cloned()
    }

    pub(crate) fn bindings(
        &self,
        key: &str,
        descendants: bool,
    ) -> Result<crate::metadata_budget::MetadataOwner<Vec<(String, u64)>>> {
        let charge = self.operation_charge.budget().charge_with(|| {
            let prefix = format!("{key}/");
            let root = self.names.get(key).map_or(0, |_| {
                2 * (key.len() + std::mem::size_of::<(String, u64)>())
            });
            let children: usize = if descendants {
                self.names
                    .range::<str, _>((
                        std::ops::Bound::Included(prefix.as_str()),
                        std::ops::Bound::Unbounded,
                    ))
                    .take_while(|(name, _)| name.starts_with(&prefix))
                    .map(|(name, _)| 2 * (name.len() + std::mem::size_of::<(String, u64)>()))
                    .sum()
            } else {
                0
            };
            4 * std::mem::size_of::<(String, u64)>() + root + children
        })?;
        let mut bindings: Vec<_> = self
            .names
            .get(key)
            .map(|ino| (key.to_owned(), *ino))
            .into_iter()
            .collect();
        if descendants {
            let prefix = format!("{key}/");
            bindings.extend(
                self.names
                    .range::<str, _>((
                        std::ops::Bound::Included(prefix.as_str()),
                        std::ops::Bound::Unbounded,
                    ))
                    .take_while(|(name, _)| name.starts_with(&prefix))
                    .map(|(name, ino)| (name.as_str().to_owned(), *ino)),
            );
        }
        Ok(crate::metadata_budget::MetadataOwner::new(bindings, charge))
    }

    pub(crate) fn reserve_tombstone(&mut self, key: &str) -> Result<MetadataCharge> {
        self.hidden(key);
        if self.tombstones.len() + self.moving.len() >= self.limit
            && !self.tombstones.contains_key(key)
            && !self.moving.contains(key)
        {
            return Err(Error::Fs(libc::ENOMEM));
        }
        self.operation_charge
            .budget()
            .charge(Self::operation_bytes(key))
    }
    pub(crate) fn tombstone_reserved(
        &mut self,
        key: &str,
        mut credit: MetadataCharge,
    ) -> Result<()> {
        if !self.tombstones.contains_key(key) {
            self.operation_charge.ensure_from(
                self.operation_charge.bytes() + Self::operation_bytes(key),
                &mut credit,
            )?;
        }
        self.tombstones
            .insert(key.to_owned(), Instant::now() + Duration::from_secs(60));
        self.changed(key);
        Ok(())
    }

    pub(crate) fn changed(&mut self, key: &str) {
        self.epoch += 1;
        let parent = key.rsplit_once('/').map_or("", |(parent, _)| parent);
        // Mutations affect the parent page and any moved/deleted subtree, not
        // fresh pages elsewhere. All epoch access is under the namespace lock.
        for name in [parent, key] {
            if let Some(node) = self.named(name) {
                node.directory_epoch.fetch_add(1, Ordering::Relaxed);
            }
        }
        let prefix = format!("{key}/");
        for (_, ino) in self
            .names
            .range::<str, _>((
                std::ops::Bound::Included(prefix.as_str()),
                std::ops::Bound::Unbounded,
            ))
            .take_while(|(name, _)| name.starts_with(&prefix))
        {
            if let Some(node) = self.nodes.get(ino) {
                node.directory_epoch.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub(crate) fn has_dirty_descendants(&self, key: &str) -> bool {
        let prefix = if key.is_empty() {
            String::new()
        } else {
            format!("{key}/")
        };
        // Both ends of an unfinished rename retain their ancestor paths, even
        // when a COPY/DELETE already moved all durable objects out of a parent.
        self.pending_moves
            .range(prefix.clone()..)
            .next()
            .is_some_and(|(name, _)| name.starts_with(&prefix))
            || self
                .dirty
                .range::<str, _>((
                    std::ops::Bound::Included(prefix.as_str()),
                    std::ops::Bound::Unbounded,
                ))
                .next()
                .is_some_and(|(name, _)| name.starts_with(&prefix))
    }

    /// Caller holds namespace then inode state, including after a commit.
    pub(crate) fn track_dirty(&mut self, ino: u64, state: &State) {
        if !state.deleted && state.version > state.durable {
            self.dirty.insert(state.key.clone(), ino);
        } else if self.dirty.get(state.key.as_str()) == Some(&ino) {
            self.dirty.remove(state.key.as_str());
        }
    }

    pub(crate) fn forget_dirty(&mut self, key: &str) {
        self.dirty.remove(key);
    }

    pub(crate) fn dirty_nodes(&self) -> Vec<Arc<Inode>> {
        self.dirty
            .values()
            .map(|ino| self.nodes[ino].clone())
            .collect()
    }

    pub fn hidden(&mut self, key: &str) -> bool {
        let now = Instant::now();
        let mut released = 0;
        self.tombstones.retain(|key, until| {
            if *until > now {
                true
            } else {
                released += Self::operation_bytes(key);
                false
            }
        });
        self.release_operations(released);
        self.tombstones.contains_key(key)
    }

    fn operation_bytes(key: &str) -> usize {
        key.len() + std::mem::size_of::<(String, Instant)>() + LINKS
    }

    fn reserve_operations(&mut self, growth: usize) -> Result<()> {
        self.operation_charge
            .resize(self.operation_charge.bytes().saturating_add(growth))
    }

    fn release_operations(&mut self, bytes: usize) {
        self.operation_charge
            .resize(self.operation_charge.bytes().saturating_sub(bytes))
            .expect("releasing namespace metadata");
    }

    pub(crate) fn remove_tombstone(&mut self, key: &str) {
        if self.tombstones.remove(key).is_some() {
            self.release_operations(Self::operation_bytes(key));
        }
    }

    pub(crate) fn begin_pending_delete(&mut self, key: &str, ino: u64) -> Result<()> {
        self.hidden(key);
        if self.tombstones.len() + self.moving.len() >= self.limit
            && !self.tombstones.contains_key(key)
            && !self.moving.contains(key)
        {
            return Err(Error::Fs(libc::ENOMEM));
        }
        let slots = usize::from(!self.tombstones.contains_key(key))
            + usize::from(!self.pending_deletes.contains_key(key));
        self.reserve_operations(slots * Self::operation_bytes(key))?;
        self.tombstones
            .insert(key.into(), Instant::now() + Duration::from_secs(60));
        self.pending_deletes.insert(key.into(), ino);
        self.changed(key);
        Ok(())
    }

    pub(crate) fn remove_pending_delete(&mut self, key: &str) {
        if self.pending_deletes.remove(key).is_some() {
            self.release_operations(Self::operation_bytes(key));
        }
    }

    pub(crate) fn begin_pending_move(&mut self, old: &str, new: &str, ino: u64) -> Result<()> {
        let growth = [old, new]
            .into_iter()
            .filter(|key| !self.pending_moves.contains_key(*key))
            .map(Self::operation_bytes)
            .sum();
        self.reserve_operations(growth)?;
        self.pending_moves.insert(old.into(), ino);
        self.pending_moves.insert(new.into(), ino);
        Ok(())
    }

    pub(crate) fn remove_pending_move(&mut self, key: &str) {
        if self.pending_moves.remove(key).is_some() {
            self.release_operations(Self::operation_bytes(key));
        }
    }

    pub fn tombstone(&mut self, key: &str) -> Result<()> {
        self.hidden(key);
        if self.tombstones.len() + self.moving.len() >= self.limit
            && !self.tombstones.contains_key(key)
            && !self.moving.contains(key)
        {
            return Err(Error::Fs(libc::ENOMEM));
        }
        if !self.tombstones.contains_key(key) {
            self.reserve_operations(Self::operation_bytes(key))?;
        }
        self.tombstones
            .insert(key.to_owned(), Instant::now() + Duration::from_secs(60));
        self.changed(key);
        Ok(())
    }

    pub fn is_moving(&self, key: &str) -> bool {
        Self::ancestors(key).any(|prefix| self.moving.contains(prefix))
    }
    pub fn is_deleting(&self, key: &str) -> bool {
        Self::ancestors(key).any(|prefix| self.pending_deletes.contains_key(prefix))
    }

    fn ancestors(key: &str) -> impl Iterator<Item = &str> {
        std::iter::successors(Some(key), |key| {
            key.rsplit_once('/').map(|(parent, _)| parent)
        })
    }

    pub fn install(&mut self, key: &str, make: impl FnOnce(u64) -> Inode) -> Result<Arc<Inode>> {
        self.try_install_reclaiming(key, |ino| Ok(make(ino)))
    }

    pub(crate) fn count_full(&self) -> bool {
        self.nodes.len() >= self.limit
    }

    fn clean_unowned(&self, state: &State) -> bool {
        state.opens == 0
            && state.version == state.durable
            && state.rename_target.is_none()
            && state.data_key.is_none()
            && state.pending_upload.is_none()
            && !self.is_moving(&state.key)
            && !self.is_deleting(&state.key)
            && !Self::ancestors(&state.key).any(|key| self.pending_moves.contains_key(key))
    }

    /// Inspect one bounded range, retaining names but never inode owners. Kernel
    /// lookup counts remain unchanged until the real FORGET callback arrives.
    pub(crate) fn kernel_retirement_batch(
        &self,
        scan: &mut ReclaimScan,
    ) -> Vec<(u64, crate::metadata_budget::SharedKey)> {
        use std::ops::Bound::{Excluded, Included};
        let end = *scan.end.get_or_insert(self.next_ino - 1);
        let mut entries = Vec::new();
        for _ in 0..128 {
            let Some((&ino, node)) = self
                .nodes
                .range((Excluded(scan.cursor), Included(end)))
                .next()
            else {
                *scan = ReclaimScan::default();
                break;
            };
            scan.cursor = ino;
            if ino == 1 || Arc::strong_count(node) != 1 {
                continue;
            }
            let Ok(state) = node.state.try_lock() else {
                continue;
            };
            if state.lookups == 0
                || state.deleted
                || self.names.get(state.key.as_str()) != Some(&ino)
                || !self.clean_unowned(&state)
            {
                continue;
            }
            let parent = state.key.rsplit_once('/').map_or("", |(parent, _)| parent);
            if let Some(parent) = self.names.get(parent) {
                entries.push((*parent, state.key.clone()));
            }
        }
        entries
    }

    /// Examine at most 128 inodes. The caller retains its own scan across yields;
    /// a shared successful-victim hint avoids rescanning a long pinned prefix.
    /// Never wait on an inode or retain a resolver pin between batches.
    pub(crate) fn reclaim_batch(&mut self, scan: &mut ReclaimScan) -> Reclaim {
        for _ in 0..128 {
            let Some((ino, node)) = self.next_reclaim_inode(scan) else {
                return Reclaim::Exhausted;
            };
            if ino == 1 || Arc::strong_count(node) != 1 {
                continue;
            }
            let Some(state) = node.state.try_lock().ok() else {
                continue;
            };
            let eligible = state.lookups == 0 && self.clean_unowned(&state);
            drop(state);
            if !eligible {
                continue;
            }
            let owner = self.nodes.remove(&ino).expect("candidate exists");
            match Arc::try_unwrap(owner) {
                Ok(node) => {
                    self.evictions += 1;
                    self.reclaim_cursor = ino;
                    self.inodes.pin().remove(&ino);
                    let key = node.state.into_inner().unwrap().key;
                    if self.names.get(&key) == Some(&ino) {
                        self.names.remove(&key);
                    }
                    return Reclaim::Removed;
                }
                Err(owner) => {
                    self.nodes.insert(ino, owner);
                }
            }
        }
        Reclaim::More
    }

    fn next_reclaim_inode(&self, scan: &mut ReclaimScan) -> Option<(u64, &Arc<Inode>)> {
        use std::ops::Bound::{Excluded, Included};
        if scan.end.is_none() {
            scan.cursor = self.reclaim_cursor;
            scan.wrap_end = self.reclaim_cursor;
            scan.end = Some(self.next_ino - 1);
        }
        let Some((&ino, node)) = self
            .nodes
            .range((Excluded(scan.cursor), Included(scan.end.unwrap())))
            .next()
        else {
            if scan.wrap_end == 0 {
                return None;
            }
            scan.cursor = 0;
            scan.end = Some(scan.wrap_end);
            scan.wrap_end = 0;
            // At most one wrap, bounded by this admission's initial high-water
            // mark rather than another caller's moving scan position.
            return self.next_reclaim_inode(scan);
        };
        scan.cursor = ino;
        Some((ino, node))
    }

    /// Synchronous callers have no continuation across EAGAIN. Finish one
    /// finite local scan here, including all batches, before reporting ENOMEM.
    pub(crate) fn try_install_reclaiming(
        &mut self,
        key: &str,
        make: impl FnOnce(u64) -> Result<Inode>,
    ) -> Result<Arc<Inode>> {
        if let Some(node) = self.named(key) {
            return Ok(node);
        }
        if self.count_full() {
            let mut scan = ReclaimScan::default();
            loop {
                match self.reclaim_batch(&mut scan) {
                    Reclaim::Removed => break,
                    Reclaim::More => continue,
                    Reclaim::Exhausted => {
                        self.capacity_failures += 1;
                        return Err(Error::Fs(libc::ENOMEM));
                    }
                }
            }
        }
        self.try_install(key, make)
    }

    /// Async callers already performed yielding admission. Keep this final
    /// publication attempt bounded if another admission consumed the free slot.
    pub(crate) fn try_install(
        &mut self,
        key: &str,
        make: impl FnOnce(u64) -> Result<Inode>,
    ) -> Result<Arc<Inode>> {
        if let Some(node) = self.named(key) {
            return Ok(node);
        }
        if self.nodes.len() >= self.limit {
            match self.reclaim_batch(&mut ReclaimScan::default()) {
                Reclaim::Removed => {}
                Reclaim::More => return Err(Error::Fs(libc::EAGAIN)),
                Reclaim::Exhausted => {
                    self.capacity_failures += 1;
                    return Err(Error::Fs(libc::ENOMEM));
                }
            }
        }
        let ino = self.next_ino;
        self.next_ino = self
            .next_ino
            .checked_add(1)
            .ok_or(Error::Fs(libc::EOVERFLOW))?;
        let node = Arc::new(make(ino)?);
        self.nodes.insert(ino, node.clone());
        self.names
            .insert(node.state.lock().unwrap().key.clone(), ino);
        self.inodes.pin().insert(ino, Arc::downgrade(&node));
        Ok(node)
    }
}

/// A bounded namespace marker, not a network-held namespace mutex. Drop wakes
/// epoch-based listings on success, error and cancellation alike.
pub struct MoveLease<'a> {
    namespace: &'a Mutex<Namespace>,
    keys: Vec<String>,
    _metadata_charge: MetadataCharge,
}
impl<'a> MoveLease<'a> {
    pub fn begin(namespace: &'a Mutex<Namespace>, keys: Vec<String>) -> Result<Self> {
        Self::begin_inner(namespace, keys, None)
    }
    pub fn begin_delete(namespace: &'a Mutex<Namespace>, key: String, ino: u64) -> Result<Self> {
        Self::begin_inner(namespace, vec![key], Some(ino))
    }
    fn begin_inner(
        namespace: &'a Mutex<Namespace>,
        keys: Vec<String>,
        delete_owner: Option<u64>,
    ) -> Result<Self> {
        let mut ns = namespace.lock().unwrap();
        if keys.iter().any(|key| {
            ns.is_moving(key)
                || delete_owner.is_some()
                    && ns.pending_moves.keys().any(|other| {
                        key == other
                            || key.starts_with(&format!("{other}/"))
                            || other.starts_with(&format!("{key}/"))
                    })
                || ns.pending_deletes.iter().any(|(other, ino)| {
                    Some(*ino) != delete_owner
                        && (key == other
                            || key.starts_with(&format!("{other}/"))
                            || other.starts_with(&format!("{key}/")))
                })
                || ns
                    .moving
                    .iter()
                    .any(|other| other.starts_with(&format!("{key}/")))
        }) {
            return Err(Error::Fs(libc::EBUSY));
        }
        ns.hidden("");
        if ns.tombstones.len() + ns.moving.len() + keys.len() > ns.limit {
            return Err(Error::Fs(libc::ENOMEM));
        }
        let charge = ns.operation_charge.budget().charge(
            std::mem::size_of::<Self>()
                + keys.capacity() * std::mem::size_of::<String>()
                + keys
                    .iter()
                    .map(|key| key.capacity() + Self::key_bytes(key))
                    .sum::<usize>(),
        )?;
        ns.moving.extend(keys.iter().cloned());
        for key in &keys {
            ns.changed(key);
        }
        Ok(Self {
            namespace,
            keys,
            _metadata_charge: charge,
        })
    }

    fn key_bytes(key: &str) -> usize {
        key.len() + std::mem::size_of::<String>() + LINKS
    }
}
impl Drop for MoveLease<'_> {
    fn drop(&mut self) {
        let mut ns = self.namespace.lock().unwrap();
        for key in &self.keys {
            ns.moving.remove(key);
            ns.changed(key);
        }
    }
}

pub fn child(parent: &str, name: &str) -> Result<String> {
    if name.is_empty() || matches!(name, "." | "..") || name.contains('/') || name.contains('\0') {
        return Err(Error::Fs(libc::EINVAL));
    }
    Ok(if parent.is_empty() {
        name.into()
    } else {
        format!("{parent}/{name}")
    })
}
