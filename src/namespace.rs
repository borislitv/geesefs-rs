use crate::{
    error::{Error, Result},
    inode::Inode,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub struct Namespace {
    pub nodes: BTreeMap<u64, Arc<Inode>>,
    pub names: BTreeMap<String, u64>,
    pub tombstones: BTreeMap<String, Instant>,
    pub epoch: u64,
    pub moving: BTreeSet<String>,
    /// Two prefixes per pending live inode, retained across failed move attempts.
    pub pending_moves: BTreeMap<String, u64>,
    /// Ambiguous/cancelled DELETEs retain their old name until an explicit retry.
    pub pending_deletes: BTreeMap<String, u64>,
    next_ino: u64,
    limit: usize,
}

impl Namespace {
    pub fn new(root: Inode, limit: usize) -> Self {
        Self {
            nodes: BTreeMap::from([(1, Arc::new(root))]),
            names: BTreeMap::from([("".into(), 1)]),
            tombstones: BTreeMap::new(),
            epoch: 0,
            moving: BTreeSet::new(),
            pending_moves: BTreeMap::new(),
            pending_deletes: BTreeMap::new(),
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
        self.epoch += 1;
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
                let s = node.state.lock().unwrap();
                (s.lookups == 0
                    && s.opens == 0
                    && s.version == s.durable
                    && s.rename_target.is_none())
                .then_some(ino)
                .filter(|ino| !self.pending_deletes.values().any(|pending| pending == ino))
            });
            if let Some(ino) = victim {
                let node = self.nodes.remove(&ino).unwrap();
                let key = node.state.lock().unwrap().key.clone();
                if self.names.get(&key) == Some(&ino) {
                    self.names.remove(&key);
                }
            } else {
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
        ns.epoch += 1;
        Ok(Self { namespace, keys })
    }
}
impl Drop for MoveLease<'_> {
    fn drop(&mut self) {
        let mut ns = self.namespace.lock().unwrap();
        for key in &self.keys {
            ns.moving.remove(key);
        }
        ns.epoch += 1;
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
