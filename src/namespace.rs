use crate::{
    error::{Error, Result},
    inode::{Inode, State},
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
    dirty: BTreeMap<String, u64>,
    pub names: BTreeMap<String, u64>,
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
    limit: usize,
}

impl Namespace {
    pub fn new(root: Inode, limit: usize) -> Self {
        let root = Arc::new(root);
        let inodes = Arc::new(InodeIndex::new());
        inodes.pin().insert(1, Arc::downgrade(&root));
        Self {
            nodes: BTreeMap::from([(1, root)]),
            inodes,
            dirty: BTreeMap::new(),
            names: BTreeMap::from([("".into(), 1)]),
            tombstones: BTreeMap::new(),
            epoch: 0,
            moving: BTreeSet::new(),
            pending_moves: BTreeMap::new(),
            pending_deletes: BTreeMap::new(),
            evictions: 0,
            capacity_failures: 0,
            next_ino: 2,
            limit,
        }
    }

    pub fn get(&self, ino: u64) -> Result<Arc<Inode>> {
        self.nodes.get(&ino).cloned().ok_or(Error::Fs(libc::ENOENT))
    }

    pub fn named(&self, key: &str) -> Option<Arc<Inode>> {
        self.names.get(key).and_then(|i| self.nodes.get(i)).cloned()
    }

    pub(crate) fn bindings(&self, key: &str, descendants: bool) -> Vec<(String, u64)> {
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
                    .range(prefix.clone()..)
                    .take_while(|(name, _)| name.starts_with(&prefix))
                    .map(|(name, ino)| (name.clone(), *ino)),
            );
        }
        bindings
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
            .range(prefix.clone()..)
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
                .range(prefix.clone()..)
                .next()
                .is_some_and(|(name, _)| name.starts_with(&prefix))
    }

    /// Caller holds namespace then inode state, including after a commit.
    pub(crate) fn track_dirty(&mut self, ino: u64, state: &State) {
        if !state.deleted && state.version > state.durable {
            self.dirty.insert(state.key.clone(), ino);
        } else if self.dirty.get(&state.key) == Some(&ino) {
            self.dirty.remove(&state.key);
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
        self.tombstones.retain(|_, until| *until > now);
        self.tombstones.contains_key(key)
    }

    pub fn tombstone(&mut self, key: &str) -> Result<()> {
        self.hidden(key);
        if self.tombstones.len() + self.moving.len() >= self.limit
            && !self.tombstones.contains_key(key)
            && !self.moving.contains(key)
        {
            return Err(Error::Fs(libc::ENOMEM));
        }
        self.tombstones
            .insert(key.to_owned(), Instant::now() + Duration::from_secs(60));
        self.changed(key);
        Ok(())
    }

    pub fn is_moving(&self, key: &str) -> bool {
        self.moving
            .iter()
            .any(|prefix| key == prefix || key.starts_with(&format!("{prefix}/")))
    }
    pub fn is_deleting(&self, key: &str) -> bool {
        self.pending_deletes
            .keys()
            .any(|prefix| key == prefix || key.starts_with(&format!("{prefix}/")))
    }

    pub fn install(&mut self, key: &str, make: impl FnOnce(u64) -> Inode) -> Result<Arc<Inode>> {
        if let Some(node) = self.named(key) {
            return Ok(node);
        }
        if self.nodes.len() >= self.limit {
            let victim = self.nodes.iter().find_map(|(&ino, node)| {
                if ino == 1 || Arc::strong_count(node) > 1 {
                    return None;
                }
                // A weak resolver can acquire state after the strong-count
                // check. Eviction must not wait for it under the global mutex.
                let s = node.state.try_lock().ok()?;
                (s.lookups == 0
                    && s.opens == 0
                    && s.version == s.durable
                    && s.rename_target.is_none())
                .then_some(ino)
                .filter(|ino| !self.pending_deletes.values().any(|pending| pending == ino))
            });
            if let Some(ino) = victim {
                let owner = self.nodes.remove(&ino).unwrap();
                match Arc::try_unwrap(owner) {
                    Ok(node) => {
                        self.evictions += 1;
                        self.inodes.pin().remove(&ino);
                        let key = node.state.into_inner().unwrap().key;
                        if self.names.get(&key) == Some(&ino) {
                            self.names.remove(&key);
                        }
                    }
                    Err(owner) => {
                        // A lock-free lookup upgraded its weak identity after
                        // the eligibility check. Preserve that live inode.
                        self.nodes.insert(ino, owner);
                        self.capacity_failures += 1;
                        return Err(Error::Fs(libc::ENOMEM));
                    }
                }
            } else {
                self.capacity_failures += 1;
                return Err(Error::Fs(libc::ENOMEM));
            }
        }
        let ino = self.next_ino;
        self.next_ino = self
            .next_ino
            .checked_add(1)
            .ok_or(Error::Fs(libc::EOVERFLOW))?;
        let node = Arc::new(make(ino));
        self.nodes.insert(ino, node.clone());
        self.names.insert(key.into(), ino);
        self.inodes.pin().insert(ino, Arc::downgrade(&node));
        Ok(node)
    }
}

/// A bounded namespace marker, not a network-held namespace mutex. Drop wakes
/// epoch-based listings on success, error and cancellation alike.
pub struct MoveLease<'a> {
    namespace: &'a Mutex<Namespace>,
    keys: Vec<String>,
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
        ns.moving.extend(keys.iter().cloned());
        for key in &keys {
            ns.changed(key);
        }
        Ok(Self { namespace, keys })
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
