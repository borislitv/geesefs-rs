use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::{
    backend::{Store, s3::ObjectHead},
    budget::{Block, Budget},
    config::Config,
    error::{Error, Result},
    inode::{Inode, State, file_type},
    namespace::{MoveLease, Namespace, child},
    read,
    write::{self, MAX_IO, PART_SIZE},
};
use fuser::{FileAttr, FileType};
use futures_util::{StreamExt, stream::FuturesUnordered};
use tokio::sync::{Notify, RwLock, Semaphore};

pub struct Filesystem {
    pub config: Arc<Config>,
    pub store: Arc<dyn Store>,
    pub namespace: Mutex<Namespace>,
    pub ingress: Budget,
    data: Budget,
    cache: Mutex<crate::cache::ReadCache>,
    cache_published: Notify,
    flush_lane: Budget,
    flush_activity: Arc<RwLock<()>>,
    flushers: Arc<Semaphore>,
    upload_admission: write::UploadAdmission,
    metadata_slots: Arc<Semaphore>,
    directory_pages: moka::sync::Cache<(u64, u64, String, usize), Arc<DirectoryPage>>,
    preloaded_directories: moka::sync::Cache<u64, std::time::Instant>,
    closing: AtomicBool,
}

struct DirectoryPage {
    fetched: std::time::Instant,
    entries: Vec<DirEntry>,
}

#[derive(Clone, Debug)]
pub struct DirEntry {
    pub ino: u64,
    pub name: String,
    pub kind: FileType,
}

/// Effective owner budgets, not RSS or allocator measurements. The cache shares
/// the data lane and can borrow revocable, idle flush capacity for clean owners.
#[derive(Debug, serde::Serialize)]
pub struct EffectiveBudgets {
    pub payload_limit_bytes: usize,
    pub data_shared_bytes: usize,
    pub read_cache_policy_bytes: usize,
    pub read_cache_loaned_bytes: usize,
    pub ingress_bytes: usize,
    pub flush_bytes: usize,
    pub data_used_bytes: usize,
    pub ingress_used_bytes: usize,
    pub flush_used_bytes: usize,
    pub requested_read_ahead_bytes: usize,
    pub requested_small_window_bytes: usize,
    pub requested_large_window_bytes: usize,
    pub max_effective_window_bytes: usize,
    pub max_overlay_window_bytes: usize,
    pub read_parallel_chunk_bytes: usize,
    pub upload_chunk_bytes: usize,
    pub max_read_tasks_per_window: usize,
    pub max_request_tasks: usize,
    pub max_flush_operations: usize,
    pub cache_entry_limit: u64,
    pub single_part_limit_bytes: u64,
    pub max_object_bytes: u64,
    pub negative_entries_per_directory: usize,
    pub max_parallel_parts_per_file: usize,
    pub max_upload_requests: usize,
    pub max_copy_requests: usize,
    pub max_queued_upload_jobs: usize,
    pub max_metadata_resolution_jobs: usize,
    pub metadata_resolution_jobs_in_use: usize,
}

pub fn not_found(error: &Error) -> bool {
    matches!(error, Error::Fs(code) if *code == libc::ENOENT)
        || matches!(error, Error::Backend { code,.. } if matches!(code.as_str(), "NotFound"|"NoSuchKey"|"404"))
}

impl Filesystem {
    pub fn effective_budgets(&self) -> EffectiveBudgets {
        let window = self
            .data
            .limit()
            .saturating_sub(MAX_IO)
            .min(u32::MAX as usize - MAX_IO);
        EffectiveBudgets {
            payload_limit_bytes: self.ingress.limit() + self.data.limit() + self.flush_lane.limit(),
            data_shared_bytes: self.data.limit(),
            read_cache_policy_bytes: self.data.limit() + self.flush_lane.limit(),
            read_cache_loaned_bytes: self.cache.lock().unwrap().loaned_bytes(),
            ingress_bytes: self.ingress.limit(),
            flush_bytes: self.flush_lane.limit(),
            data_used_bytes: self.data.used(),
            ingress_used_bytes: self.ingress.used(),
            flush_used_bytes: self.flush_lane.used(),
            requested_read_ahead_bytes: self.config.read_ahead * 1024,
            requested_small_window_bytes: self.config.read_ahead_small * 1024,
            requested_large_window_bytes: self.config.read_ahead_large * 1024,
            max_effective_window_bytes: window,
            max_overlay_window_bytes: window / 2,
            read_parallel_chunk_bytes: (self.config.read_ahead_parallel * 1024).min(window),
            upload_chunk_bytes: crate::upload_body::CHUNK_SIZE,
            max_read_tasks_per_window: self.config.max_requests,
            max_request_tasks: self.config.max_requests,
            max_flush_operations: self.config.max_flushers,
            cache_entry_limit: self.config.entry_limit,
            single_part_limit_bytes: self.config.single_part.max(5) * 1024 * 1024,
            max_object_bytes: self.config.part_sizes.max_size(),
            negative_entries_per_directory: self.config.negative_lookup_cache_size,
            max_parallel_parts_per_file: self.config.max_parallel_parts,
            max_upload_requests: self.config.max_flushers,
            max_copy_requests: self.config.max_parallel_copy,
            max_queued_upload_jobs: self.config.max_requests,
            max_metadata_resolution_jobs: self.config.max_requests,
            metadata_resolution_jobs_in_use: self.config.max_requests
                - self.metadata_slots.available_permits(),
        }
    }
    // A failed/cancelled directory move may already have drained source keys.
    // Keep mutators from resurrecting them until that bounded move is retried.
    fn pending_rename(ns: &Namespace, key: &str, except: Option<u64>) -> bool {
        key.match_indices('/')
            .map(|(end, _)| end)
            .chain(std::iter::once(key.len()))
            .any(|end| {
                ns.pending_moves
                    .get(&key[..end])
                    .is_some_and(|ino| Some(*ino) != except)
            })
    }
    pub fn new(config: Config, store: Arc<dyn Store>) -> Result<Arc<Self>> {
        config.validate()?;
        let total = config
            .memory_limit
            .checked_mul(1024 * 1024)
            .ok_or(Error::Fs(libc::EINVAL))?;
        // All lanes are carved out of the requested total, not hidden additions.
        let flush_bytes = 2 * PART_SIZE;
        let ingress_bytes = (total / 16).max(MAX_IO);
        let data_bytes = total
            .checked_sub(flush_bytes + ingress_bytes)
            .filter(|n| *n >= 4 * MAX_IO)
            .ok_or_else(|| {
                Error::Config(
                    "memory-limit too small for bounded read/write/flush lanes (minimum 16 MiB)"
                        .into(),
                )
            })?;
        let config = Arc::new(config);
        let root = Inode::new(1, "".into(), true, None, &config);
        root.state.lock().unwrap().lookups = 1;
        Ok(Arc::new(Self {
            namespace: Mutex::new(Namespace::new(root, config.entry_limit as usize)),
            ingress: Budget::new(ingress_bytes)?,
            data: Budget::new(data_bytes)?,
            cache: Mutex::new(crate::cache::ReadCache::with_entry_limit(
                (data_bytes + flush_bytes) as u64,
                config.entry_limit as usize,
            )),
            cache_published: Notify::new(),
            flush_lane: Budget::new(flush_bytes)?,
            flush_activity: Arc::new(RwLock::new(())),
            flushers: Arc::new(Semaphore::new(config.max_flushers)),
            upload_admission: write::UploadAdmission::new(&config),
            metadata_slots: Arc::new(Semaphore::new(config.max_requests)),
            directory_pages: moka::sync::Cache::builder()
                .max_capacity(config.entry_limit)
                .weigher(|_, page: &Arc<DirectoryPage>| page.entries.len().max(1) as u32)
                .build(),
            preloaded_directories: moka::sync::Cache::new(config.entry_limit),
            closing: AtomicBool::new(false),
            config,
            store,
        }))
    }

    pub fn node(&self, ino: u64) -> Result<Arc<Inode>> {
        self.namespace.lock().unwrap().get(ino)
    }

    pub fn check_write(&self) -> Result<()> {
        if self.config.mount_options.iter().any(|v| v == "ro") {
            return Err(Error::Fs(libc::EROFS));
        }
        if self.closing.load(Ordering::Acquire) {
            return Err(Error::Cancelled);
        }
        Ok(())
    }

    fn child_key(&self, parent: u64, name: &str) -> Result<String> {
        let parent = self.node(parent)?;
        let state = parent.state.lock().unwrap();
        if state.deleted {
            return Err(Error::Fs(libc::ENOENT));
        }
        if file_type(state.mode) != FileType::Directory {
            return Err(Error::Fs(libc::ENOTDIR));
        }
        child(&state.key, name)
    }

    pub async fn lookup(&self, parent: u64, name: &str) -> Result<Arc<Inode>> {
        let started = std::time::Instant::now();
        let key = self.child_key(parent, name)?;
        if self.config.effective_list_type() == crate::config::ListType::Extended
            && !self.config.stat_cache_ttl.is_zero()
        {
            let preload = {
                let mut ns = self.namespace.lock().unwrap();
                let negative = ns
                    .get(parent)?
                    .state
                    .lock()
                    .unwrap()
                    .negative_lookups
                    .get(name)
                    .is_some_and(|fetched| fetched.elapsed() < self.config.stat_cache_ttl);
                !ns.is_moving(&key)
                    && !ns.is_deleting(&key)
                    && !ns.hidden(&key)
                    && ns.named(&key).is_none()
                    && !negative
            };
            if preload {
                // Speculation is optional. An absent/failed/standard-metadata
                // page never replaces the normal authoritative lookup below.
                let _ =
                    tokio::time::timeout(self.config.http_timeout, self.preload_directory(parent))
                        .await;
            }
        }
        let (epoch, observed) = {
            let mut ns = self.namespace.lock().unwrap();
            if ns.is_moving(&key) {
                return Err(Error::Fs(libc::EAGAIN));
            }
            if ns.is_deleting(&key) {
                return Err(Error::Fs(libc::EAGAIN));
            }
            if ns.hidden(&key) {
                return Err(Error::Fs(libc::ENOENT));
            }
            if let Some(node) = ns.named(&key) {
                let state = node.state.lock().unwrap();
                if state.rename_target.is_some()
                    || state.data_key.is_some()
                    || state.version > state.durable
                    || state.metadata_known && state.fetched.elapsed() < self.config.stat_cache_ttl
                    || file_type(state.mode) == FileType::Directory
                        && ns.has_dirty_descendants(&key)
                {
                    drop(state);
                    return Ok(node);
                }
            }
            if self.config.negative_lookup_cache_size > 0 {
                let parent = ns.get(parent)?;
                let mut state = parent.state.lock().unwrap();
                if state
                    .negative_lookups
                    .get(name)
                    .is_some_and(|when| when.elapsed() < self.config.stat_cache_ttl)
                {
                    return Err(Error::Fs(libc::ENOENT));
                }
                state.negative_lookups.remove(name);
            }
            (
                ns.epoch,
                ns.named(&key).map(|node| {
                    let s = node.state.lock().unwrap();
                    (
                        node.ino,
                        s.version,
                        s.remote.as_ref().map(|h| h.etag.clone()),
                    )
                }),
            )
        };
        let (directory, head) = match self.store.head(&key).await {
            Ok(head) => (false, Some(head)),
            Err(error) if not_found(&error) => {
                let marker = if self.config.no_dir_object {
                    None
                } else {
                    match self.store.head(&format!("{key}/")).await {
                        Ok(head) => Some(head),
                        Err(error) if not_found(&error) => None,
                        Err(error) => return Err(error),
                    }
                };
                if let Some(head) = marker {
                    (true, Some(head))
                } else {
                    if self.config.no_implicit_dir {
                        return Err(self.missing(parent, name, &key, epoch, started, &observed)?);
                    }
                    let page = self.store.list(&format!("{key}/"), Some("/"), None).await?;
                    if page.objects.is_empty() && page.common_prefixes.is_empty() {
                        return Err(self.missing(parent, name, &key, epoch, started, &observed)?);
                    }
                    (true, None)
                }
            }
            Err(error) => return Err(error),
        };
        let mut ns = self.namespace.lock().unwrap();
        {
            let parent = ns.get(parent)?;
            let state = parent.state.lock().unwrap();
            if state.deleted || child(&state.key, name)? != key {
                return Err(Error::Fs(libc::EAGAIN));
            }
        }
        if ns.is_moving(&key) || ns.is_deleting(&key) || ns.hidden(&key) {
            return Err(Error::Fs(libc::EAGAIN));
        }
        if let Some(node) = ns.named(&key) {
            let fresh = Inode::new(node.ino, key.clone(), directory, head, &self.config);
            let mut state = node.state.lock().unwrap();
            if observed.as_ref()
                != Some(&(
                    node.ino,
                    state.version,
                    state.remote.as_ref().map(|h| h.etag.clone()),
                ))
            {
                return Err(Error::Fs(libc::EAGAIN));
            }
            if state.version == state.durable
                && state.rename_target.is_none()
                && state.data_key.is_none()
            {
                let mut replacement = fresh.state.into_inner().unwrap();
                replacement.lookups = state.lookups;
                replacement.opens = state.opens;
                replacement.version = state.version;
                replacement.durable = state.durable;
                replacement.negative_lookups = std::mem::take(&mut state.negative_lookups);
                *state = replacement;
            }
            drop(state);
            return Ok(node);
        }
        if observed.is_some() {
            return Err(Error::Fs(libc::EAGAIN));
        }
        ns.install(&key, |ino| {
            Inode::new(ino, key.clone(), directory, head, &self.config)
        })
    }

    async fn preload_directory(&self, parent: u64) -> Result<()> {
        let node = self.node(parent)?;
        let _gate = node.preload.lock().await;
        let (prefix, epoch) = {
            let ns = self.namespace.lock().unwrap();
            if self
                .preloaded_directories
                .get(&parent)
                .is_some_and(|fetched| fetched.elapsed() < self.config.stat_cache_ttl)
            {
                return Ok(());
            }
            let state = node.state.lock().unwrap();
            if state.deleted || ns.is_moving(&state.key) || ns.is_deleting(&state.key) {
                return Err(Error::Fs(libc::EAGAIN));
            }
            let prefix = if state.key.is_empty() {
                String::new()
            } else {
                format!("{}/", state.key)
            };
            (prefix, ns.epoch)
        };
        let fetched = std::time::Instant::now();
        let page = self.store.list(&prefix, Some("/"), None).await;
        let mut ns = self.namespace.lock().unwrap();
        if ns.epoch != epoch {
            return Err(Error::Fs(libc::EAGAIN));
        }
        // This records an attempt, not an authoritative negative listing.
        self.preloaded_directories.insert(parent, fetched);
        self.preloaded_directories.run_pending_tasks();
        ns.hidden("");
        for object in page?
            .objects
            .into_iter()
            .take(crate::backend::extended_list::MAX_PAGE_ENTRIES)
        {
            let Some(name) = object.key.strip_prefix(&prefix) else {
                return Err(Error::InvalidResponse("preload prefix escape".into()));
            };
            if name.is_empty()
                || name.contains('/')
                || ns.tombstones.contains_key(&object.key)
                || ns.is_moving(&object.key)
                || ns.is_deleting(&object.key)
                || ns.named(&object.key).is_some()
            {
                continue;
            }
            child(prefix.trim_end_matches('/'), name)?;
            let (Some(metadata), Some(etag)) =
                (object.metadata, object.etag.filter(|etag| !etag.is_empty()))
            else {
                continue;
            };
            let head = ObjectHead {
                size: object.size,
                etag,
                metadata: crate::backend::metadata::encode_metadata(&metadata)?
                    .into_iter()
                    .collect(),
                last_modified: object
                    .last_modified
                    .and_then(|date| SystemTime::try_from(date).ok()),
                ..Default::default()
            };
            ns.install(&object.key, |ino| {
                let node = Inode::new(ino, object.key.clone(), false, Some(head), &self.config);
                node.state.lock().unwrap().fetched = fetched;
                node
            })?;
        }
        Ok(())
    }

    pub fn forget(&self, ino: u64, count: u64) {
        if let Ok(node) = self.node(ino) {
            let mut s = node.state.lock().unwrap();
            s.lookups = s.lookups.saturating_sub(count);
        }
    }

    /// Refresh getattr/fstat by stable inode identity, never by a possibly
    /// replaced name. Root is synthetic; implicit directories are checked by
    /// LIST, not by assuming a marker exists. Dirty attributes remain local.
    pub async fn refresh_inode_if_expired(&self, ino: u64) -> Result<Arc<Inode>> {
        let node = self.node(ino)?;
        let (key, directory, remote, version, epoch) = {
            let ns = self.namespace.lock().unwrap();
            let s = node.state.lock().unwrap();
            if s.deleted {
                return Err(Error::Fs(libc::ENOENT));
            }
            if ino == 1 {
                return Ok(node.clone());
            }
            if ns.is_moving(&s.key) || ns.is_deleting(&s.key) {
                return Err(Error::Fs(libc::EAGAIN));
            }
            if s.version > s.durable
                || s.rename_target.is_some()
                || s.data_key.is_some()
                || s.metadata_known && s.fetched.elapsed() < self.config.stat_cache_ttl
            {
                return Ok(node.clone());
            }
            let directory = file_type(s.mode) == FileType::Directory;
            if directory && ns.has_dirty_descendants(&s.key) {
                return Ok(node.clone());
            }
            (
                s.key.clone(),
                directory,
                s.remote.clone(),
                s.version,
                ns.epoch,
            )
        };
        let fetched = if directory && remote.is_none() {
            let page = self.store.list(&format!("{key}/"), Some("/"), None).await;
            match page {
                Ok(page) if !page.objects.is_empty() || !page.common_prefixes.is_empty() => {
                    Ok(None)
                }
                Ok(_) => Err(Error::Fs(libc::ENOENT)),
                Err(e) => Err(e),
            }
        } else {
            match self
                .store
                .head(&if directory {
                    format!("{key}/")
                } else {
                    key.clone()
                })
                .await
            {
                Ok(head) => Ok(Some(head)),
                Err(error) if directory && not_found(&error) && !self.config.no_implicit_dir => {
                    match self.store.list(&format!("{key}/"), Some("/"), None).await {
                        Ok(page)
                            if !page.objects.is_empty() || !page.common_prefixes.is_empty() =>
                        {
                            Ok(None)
                        }
                        Ok(_) => Err(Error::Fs(libc::ENOENT)),
                        Err(e) => Err(e),
                    }
                }
                Err(e) => Err(e),
            }
        };
        let mut ns = self.namespace.lock().unwrap();
        let mut s = node.state.lock().unwrap();
        if s.deleted
            || s.key != key
            || ns.names.get(&key) != Some(&ino)
            || ns.is_moving(&key)
            || ns.is_deleting(&key)
        {
            return Err(Error::Fs(libc::EAGAIN));
        }
        if s.version != version
            || s.remote.as_ref().map(|h| &h.etag) != remote.as_ref().map(|h| &h.etag)
            || s.version > s.durable
            || s.rename_target.is_some()
            || s.data_key.is_some()
            || directory && ns.has_dirty_descendants(&key)
        {
            return Ok(node.clone());
        }
        if directory && ns.epoch != epoch && fetched.as_ref().is_err_and(not_found) {
            return Err(Error::Fs(libc::EAGAIN));
        }
        match fetched {
            Ok(head) => {
                let mut fresh = Inode::new(ino, key, directory, head, &self.config)
                    .state
                    .into_inner()
                    .unwrap();
                fresh.lookups = s.lookups;
                fresh.opens = s.opens;
                fresh.version = s.version;
                fresh.durable = s.durable;
                fresh.negative_lookups = std::mem::take(&mut s.negative_lookups);
                *s = fresh;
            }
            Err(error) if not_found(&error) => {
                s.deleted = true;
                ns.names.remove(&key);
                ns.changed(&key);
                return Err(error);
            }
            Err(error) => return Err(error),
        }
        drop(s);
        Ok(node)
    }
    fn missing(
        &self,
        parent: u64,
        name: &str,
        key: &str,
        epoch: u64,
        started: std::time::Instant,
        observed: &Option<(u64, u64, Option<String>)>,
    ) -> Result<Error> {
        let mut ns = self.namespace.lock().unwrap();
        if ns.epoch != epoch {
            return Ok(Error::Fs(libc::EAGAIN));
        }
        if let Some(node) = ns.named(key) {
            let mut state = node.state.lock().unwrap();
            if state.version > state.durable
                || file_type(state.mode) == FileType::Directory && ns.has_dirty_descendants(key)
                || observed.as_ref()
                    != Some(&(
                        node.ino,
                        state.version,
                        state.remote.as_ref().map(|h| h.etag.clone()),
                    ))
            {
                return Ok(Error::Fs(libc::EAGAIN));
            }
            state.deleted = true;
            drop(state);
            ns.names.remove(key);
            ns.changed(key);
        }
        if self.config.negative_lookup_cache_size > 0
            && started.elapsed() < self.config.stat_cache_ttl
        {
            let node = ns.get(parent)?;
            let mut state = node.state.lock().unwrap();
            state
                .negative_lookups
                .retain(|_, when| when.elapsed() < self.config.stat_cache_ttl);
            if state.negative_lookups.len() >= self.config.negative_lookup_cache_size
                && !state.negative_lookups.contains_key(name)
                && let Some(oldest) = state
                    .negative_lookups
                    .iter()
                    .min_by_key(|(_, when)| *when)
                    .map(|(name, _)| name.clone())
            {
                state.negative_lookups.remove(&oldest);
            }
            state.negative_lookups.insert(name.into(), started);
        }
        Ok(Error::Fs(libc::ENOENT))
    }

    pub fn reference(&self, node: &Inode) {
        node.state.lock().unwrap().lookups += 1;
    }

    pub async fn link(&self, ino: u64, parent: u64, name: &str) -> Result<Arc<Inode>> {
        if !self.config.emulate_hardlinks_as_symlinks {
            return Err(Error::Fs(libc::EOPNOTSUPP));
        }
        let source = self.node(ino)?;
        if source.attr().kind == FileType::Directory {
            return Err(Error::Fs(libc::EPERM));
        }
        let target = source.state.lock().unwrap().key.clone();
        let parent = self.node(parent)?;
        let base = parent.state.lock().unwrap().key.clone();
        let from: Vec<_> = base.split('/').filter(|s| !s.is_empty()).collect();
        let to: Vec<_> = target.split('/').filter(|s| !s.is_empty()).collect();
        let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
        let relative = format!(
            "{}{}",
            "../".repeat(from.len() - common),
            to[common..].join("/")
        );
        self.create(
            parent.ino,
            name,
            libc::S_IFLNK | 0o777,
            Some(relative.as_bytes()),
            0,
        )
        .await
    }

    pub fn parent_ino(&self, ino: u64) -> u64 {
        let Ok(node) = self.node(ino) else {
            return 1;
        };
        let key = node.state.lock().unwrap().key.clone();
        let parent = key.rsplit_once('/').map_or("", |(parent, _)| parent);
        self.namespace
            .lock()
            .unwrap()
            .named(parent)
            .map_or(1, |node| node.ino)
    }

    pub fn open(&self, ino: u64, writable: bool) -> Result<()> {
        if writable {
            self.check_write()?;
        }
        let node = self.node(ino)?;
        let mut state = node.state.lock().unwrap();
        if state.deleted {
            return Err(Error::Fs(libc::ENOENT));
        }
        if state.opens == 0 {
            *node.pattern.lock().unwrap() = read::Pattern::default();
        }
        state.opens += 1;
        Ok(())
    }

    pub fn release(&self, ino: u64) {
        if let Ok(node) = self.node(ino) {
            let mut state = node.state.lock().unwrap();
            state.opens = state.opens.saturating_sub(1);
        }
    }

    pub async fn create(
        &self,
        parent: u64,
        name: &str,
        mode: u32,
        target: Option<&[u8]>,
        rdev: u32,
    ) -> Result<Arc<Inode>> {
        self.check_write()?;
        let key = self.child_key(parent, name)?;
        match self.lookup(parent, name).await {
            Ok(_) => return Err(Error::Fs(libc::EEXIST)),
            Err(error) if not_found(&error) => {}
            Err(error) => return Err(error),
        }
        let directory = mode & libc::S_IFMT == libc::S_IFDIR;
        let mut ns = self.namespace.lock().unwrap();
        {
            let p = ns.get(parent)?;
            let s = p.state.lock().unwrap();
            if s.deleted
                || ns.names.get(&s.key) != Some(&parent)
                || s.rename_target.is_some()
                || child(&s.key, name)? != key
            {
                return Err(Error::Fs(libc::ENOENT));
            }
        }
        if ns.is_moving(&key) || ns.is_deleting(&key) || Self::pending_rename(&ns, &key, None) {
            return Err(Error::Fs(libc::EAGAIN));
        }
        if ns.named(&key).is_some() {
            return Err(Error::Fs(libc::EEXIST));
        }
        ns.tombstones.remove(&key);
        ns.changed(&key);
        let node = ns.install(&key, |ino| {
            Inode::new(ino, key.clone(), directory, None, &self.config)
        })?;
        ns.get(parent)?
            .state
            .lock()
            .unwrap()
            .negative_lookups
            .remove(name);
        {
            let mut state = node.state.lock().unwrap();
            state.mode = mode;
            state.rdev = rdev;
            state.version = 1;
            if let Some(target) = target {
                state
                    .metadata
                    .insert(self.config.symlink_attr.clone(), target.to_vec());
                state.size = target.len() as u64;
            }
            self.prepare_metadata(&mut state);
        }
        Ok(node)
    }

    /// FUSE creation passes the request identity only with enable-perms, as the
    /// reference's CreateFile/MkDir/CreateSymlink SetAttributes calls do. There is
    /// no userspace setgid-parent inheritance in that contract.
    pub async fn create_with_identity(
        &self,
        parent: u64,
        name: &str,
        mode: u32,
        target: Option<&[u8]>,
        rdev: u32,
        identity: (u32, u32),
    ) -> Result<Arc<Inode>> {
        let node = self.create(parent, name, mode, target, rdev).await?;
        if self.config.enable_perms {
            self.setattr(node.ino, None, Some(identity.0), Some(identity.1), None)?;
        }
        Ok(node)
    }

    /// Publish the requested result while retaining the caller-owned load future.
    /// A returned error belongs to read-ahead after a successful publication.
    pub async fn read_reply(
        &self,
        ino: u64,
        offset: u64,
        length: usize,
        ready: impl FnOnce(Result<Block>) + Send,
    ) -> Option<Error> {
        self.read_reply_inner(ino, offset, length, false, ready)
            .await
    }

    /// Send bytes synchronously without allowing a reply to pin its GET owner.
    pub async fn read_reply_data(
        &self,
        ino: u64,
        offset: u64,
        length: usize,
        ready: impl FnOnce(Result<&[u8]>) + Send,
    ) -> Option<Error> {
        self.read_reply_inner(ino, offset, length, true, move |result| match result {
            Ok(block) => ready(Ok(block.as_slice())),
            Err(error) => ready(Err(error)),
        })
        .await
    }

    async fn read_reply_inner(
        &self,
        ino: u64,
        offset: u64,
        length: usize,
        borrow_reply: bool,
        ready: impl FnOnce(Result<Block>) + Send,
    ) -> Option<Error> {
        let mut ready = Some(ready);
        let result = self
            .read_inner(ino, offset, length, borrow_reply, &mut ready)
            .await;
        if let Some(ready) = ready {
            let cached_length = result
                .as_ref()
                .ok()
                .and_then(Option::as_ref)
                .map_or(0, |block| block.as_slice().len());
            ready(result.map(|block| block.expect("unpublished read owns its result")));
            if cached_length == 0 {
                None
            } else {
                self.refill_after_hit(ino, offset + cached_length as u64)
                    .await
                    .err()
            }
        } else {
            result.err()
        }
    }

    /// Keep optional refill in the same admitted, cancellable reply future. Ready
    /// bytes have already been published and no reply owner is held across admission.
    async fn refill_after_hit(&self, ino: u64, consumed: u64) -> Result<()> {
        let node = self.node(ino)?;
        let Some((size, generation, version)) = read::refill_state(&node, consumed) else {
            return Ok(());
        };
        let budgets = self.effective_budgets();
        let window = node
            .pattern
            .lock()
            .unwrap()
            .window(&self.config)
            .min(budgets.max_effective_window_bytes);
        if window == 0 {
            return Ok(());
        }
        let Some(range) = self.refill_range(&node, generation, consumed, size, window) else {
            return Ok(());
        };
        let length = (range.end - range.start) as usize;
        let read::FillAdmission::Leader(fill) =
            read::start_fill(&node, generation, range.start, range.end)
        else {
            return Ok(());
        };
        // A concurrent fill may have published and relinquished its interval
        // between planning and ownership. Retry speculation on a later read.
        if self
            .cache
            .lock()
            .unwrap()
            .gap(ino, generation, range.clone())
            != Some(range.clone())
        {
            return Ok(());
        }
        let Some(charge) = self.try_refill_admission(ino, generation, consumed, length) else {
            return Ok(());
        };
        let snapshot = read::snapshot(&node)?;
        if snapshot.version != version
            || read::generation(snapshot.version, snapshot.remote.as_ref()) != generation
            || !write::remote_only(&snapshot, range.start, length)
        {
            return Ok(());
        }
        let start = range.start;
        self.load_window(
            read::Window {
                snapshot: Arc::new(snapshot),
                range,
                charge,
                single_buffer: true,
                cache_key: Some((ino, generation)),
                fill,
            },
            start,
            0,
            false,
            &mut None::<fn(Result<Block>)>,
        )
        .await?;
        Ok(())
    }

    fn refill_range(
        &self,
        node: &Inode,
        generation: u64,
        consumed: u64,
        size: u64,
        window: usize,
    ) -> Option<std::ops::Range<u64>> {
        let trigger = consumed.saturating_add((window / 2) as u64).min(size - 1);
        if read::fill_covers(node, generation, trigger)
            || self
                .cache
                .lock()
                .unwrap()
                .contains(node.ino, generation, trigger)
        {
            return None;
        }
        let mut cursor = consumed;
        loop {
            let gap = self
                .cache
                .lock()
                .unwrap()
                .gap(node.ino, generation, cursor..size)?;
            cursor = gap.end;
            if let Some(range) = read::unfilled(node, generation, gap) {
                return Some(range.start..range.end.min(range.start.saturating_add(window as u64)));
            }
            if cursor == size {
                return None;
            }
        }
    }

    fn try_refill_admission(
        &self,
        ino: u64,
        generation: u64,
        consumed: u64,
        length: usize,
    ) -> Option<crate::budget::Reservation> {
        // No waiting, flushing, or bypassing FIFO for speculative bytes. Reclaim
        // old owners only while the still-needed current backing stays intact.
        let cache = self.cache.lock().unwrap();
        loop {
            if let Ok(charge) = self.data.try_reserve(length) {
                return Some(charge);
            }
            if !cache.evict_consumed_lru(ino, generation, consumed) {
                return None;
            }
        }
    }

    pub async fn read(&self, ino: u64, offset: u64, length: usize) -> Result<Block> {
        Ok(self
            .read_inner(ino, offset, length, false, &mut None::<fn(Result<Block>)>)
            .await?
            .expect("a read without early publication retains its result"))
    }

    async fn read_inner(
        &self,
        ino: u64,
        offset: u64,
        length: usize,
        borrow_reply: bool,
        ready: &mut Option<impl FnOnce(Result<Block>) + Send>,
    ) -> Result<Option<Block>> {
        if length > MAX_IO {
            return Err(Error::Fs(libc::EINVAL));
        }
        let node = self.node(ino)?;
        let (size, clean, generation, version, etag, mut single_buffer) = {
            let state = node.state.lock().unwrap();
            (
                state.size,
                state.version == state.durable,
                read::generation(state.version, state.remote.as_ref()),
                state.version,
                state.remote.as_ref().map(|remote| remote.etag.clone()),
                state.version == state.durable
                    && state.extents.is_empty()
                    && state.remote.is_some()
                    && state.visible_remote_size >= state.size,
            )
        };
        let length = length.min(size.saturating_sub(offset) as usize);
        // Track every nonempty caller read, including cache hits. Backend misses
        // are not the access stream: skipping hits turns contiguous kernel reads
        // into apparent seeks at the next read-ahead boundary.
        let window = if length > 0 {
            node.pattern
                .lock()
                .unwrap()
                .prefetch(offset, length, &self.config)
        } else {
            0
        };
        // Participate in pressure reclamation before borrowing a cache block:
        // an earlier FIFO waiter may need bytes published after its eviction.
        // One-block hits share their allocation; fragment spans use this output.
        let reply = self.admit(length).await?;
        if length == 0 {
            return reply.copy(&[]).map(Some);
        }
        if let Some(block) = self
            .cache
            .lock()
            .unwrap()
            .read(ino, generation, offset, length, reply)
        {
            return Ok(Some(block));
        }
        // Miss admission reserves output and scratch atomically below.
        let budgets = self.effective_budgets();
        let mut start = if clean {
            offset / (128 * 1024) * (128 * 1024)
        } else {
            offset
        };
        let window_limit = if single_buffer {
            budgets.max_effective_window_bytes
        } else {
            budgets.max_overlay_window_bytes
        };
        if offset - start + length as u64 > window_limit as u64 {
            start = offset;
        }
        let relative = (offset - start) as usize;
        let mut prefetch = if clean {
            window
                .max(relative + length)
                .min(window_limit)
                .min(size.saturating_sub(start) as usize)
        } else {
            length
        };
        let chunk = budgets.read_parallel_chunk_bytes.max(1);
        // A budget-limited large window should end on a configured parallel
        // chunk boundary, not add another partial GET at every refill. EOF keeps
        // its exact tail, and the caller's requested range always fits.
        if clean && prefetch < window && start + (prefetch as u64) < size && prefetch >= chunk {
            prefetch = (prefetch / chunk * chunk).max(relative + length);
        }
        // FUSE may issue neighboring reads before the first window completes.
        // Waiting readers must not reserve another whole window and evict the
        // very fill they can reuse. Recheck after wakeup and after taking ownership.
        let fill = loop {
            let admission = read::start_fill(&node, generation, start, start + prefetch as u64);
            let reply = self.admit(length).await?;
            let cached = self
                .cache
                .lock()
                .unwrap()
                .read(ino, generation, offset, length, reply);
            if let Some(block) = cached {
                return Ok(Some(block));
            }
            match admission {
                read::FillAdmission::Leader(owner) => break owner,
                read::FillAdmission::Wait(mut done) => {
                    // Subscribe before the cache check so a completed chunk
                    // cannot be missed between lookup and waiting. Closing also
                    // wakes followers after error or cancellation; retry normally.
                    let _ = tokio::time::timeout(self.config.request_timeout, done.changed())
                        .await
                        .map_err(|_| Error::Timeout("overlapping read fill"))?;
                }
            }
        };
        // Atomically admit all payload/GET/output buffers before the first
        // allocation; otherwise competing readers could each pin half a lane.
        let (charge, snapshot) = loop {
            let charge = self
                .admit((if single_buffer { 1 } else { 2 }) * prefetch + length)
                .await?;
            // Capture payload owners only after admission. Pinning dirty extents
            // while waiting for their own permits would deadlock a pressured read.
            let snapshot = read::snapshot(&node)?;
            let available = (snapshot.size.saturating_sub(start)).min(prefetch as u64) as usize;
            if single_buffer && !write::remote_only(&snapshot, start, available) {
                drop(snapshot);
                drop(charge);
                single_buffer = false;
                prefetch = prefetch.min(budgets.max_overlay_window_bytes);
                if prefetch < window && start + (prefetch as u64) < size && prefetch >= chunk {
                    prefetch = (prefetch / chunk * chunk).max(relative + length);
                }
                continue;
            }
            break (charge, Arc::new(snapshot));
        };
        let cacheable = clean
            && snapshot.version == version
            && snapshot.remote.as_ref().map(|head| head.etag.clone()) == etag;
        self.load_window(
            read::Window {
                snapshot,
                range: start..start + prefetch as u64,
                charge,
                single_buffer,
                cache_key: cacheable.then_some((ino, generation)),
                fill,
            },
            offset,
            length,
            borrow_reply,
            ready,
        )
        .await
    }

    async fn load_window(
        &self,
        window: read::Window,
        offset: u64,
        length: usize,
        borrow_reply: bool,
        ready: &mut Option<impl FnOnce(Result<Block>) + Send>,
    ) -> Result<Option<Block>> {
        let read::Window {
            snapshot,
            range,
            mut charge,
            single_buffer,
            cache_key,
            fill,
        } = window;
        let chunk = self.effective_budgets().read_parallel_chunk_bytes.max(1);
        let mut output = charge.split(length);
        // The snapshot may have shrunk while admission flushed another generation.
        let available = length.min(snapshot.size.saturating_sub(offset) as usize);
        let mut reply = Some(output.split(available));
        drop(output);
        let mut result = if available == 0 {
            Some(reply.take().expect("empty reply admission").zeroed())
        } else {
            None
        };
        let mut remaining = available;
        if remaining == 0
            && let Some(ready) = ready.take()
        {
            ready(Ok(result.take().expect("empty requested result")));
        }
        let end = range.end.min(snapshot.size);
        let mut tasks = tokio::task::JoinSet::new();
        let (sender, mut chunks) = tokio::sync::mpsc::channel(self.config.max_requests);
        let mut next = range.start;
        let mut prefetch_error = None;
        while next < end || !tasks.is_empty() || !chunks.is_empty() {
            while next < end && tasks.len() < self.config.max_requests {
                let at = next;
                let amount = (end - next).min(chunk as u64) as usize;
                next += amount as u64;
                let mut owner = charge.split((if single_buffer { 1 } else { 2 }) * amount);
                let (store, snapshot) = (self.store.clone(), snapshot.clone());
                let sender = sender.clone();
                tasks.spawn(async move {
                    let result = async {
                        let local = Budget::new(2 * amount)?;
                        let output = crate::backend::RangeSink::new(sender, owner.split(amount));
                        if single_buffer {
                            store
                                .read_range_chunks(
                                    &snapshot.key,
                                    at,
                                    amount,
                                    snapshot.remote.as_ref().expect("clean remote snapshot"),
                                    &local,
                                    &output,
                                )
                                .await
                        } else {
                            let block =
                                write::materialize(&store, &snapshot, at, amount, &local).await?;
                            output.send(at, block).await
                        }
                    }
                    .await;
                    (at, result)
                });
            }
            let completed = tokio::select! {
                // A producer can finish with a late body error after enqueueing
                // valid requested bytes. Process those bytes before its result.
                biased;
                value = chunks.recv(), if !chunks.is_empty() || !tasks.is_empty() => {
                    Some(value.expect("window retains its delivery sender"))
                }
                completed = tasks.join_next(), if !tasks.is_empty() => {
                    let (at, outcome) = match completed.expect("nonempty read task set") {
                        Ok(value) => value,
                        Err(_) => {
                            tasks.abort_all();
                            while tasks.join_next().await.is_some() {}
                            return Err(Error::Cancelled);
                        }
                    };
                    if let Err(error) = outcome {
                        let chunk_end = (at + chunk as u64).min(end);
                        if at >= offset + available as u64 || chunk_end <= offset {
                            // Optional read-ahead cannot cancel a healthy requested
                            // dependency, irrespective of completion order.
                            prefetch_error.get_or_insert(error);
                            continue;
                        }
                        tasks.abort_all();
                        while tasks.join_next().await.is_some() {}
                        return Err(error);
                    }
                    None
                }
            };
            let Some((at, block)) = completed else {
                continue;
            };
            let left = at.max(offset);
            let right = (at + block.as_slice().len() as u64).min(offset + available as u64);
            if right > left {
                if borrow_reply && remaining == available && right - left == available as u64 {
                    // The delivered immutable owner already covers the reply.
                    // Retain its complete backing charge instead of allocating a copy.
                    result = Some(block.slice((left - at) as usize..(right - at) as usize));
                    drop(reply.take());
                } else {
                    result
                        .get_or_insert_with(|| {
                            reply.take().expect("composed reply admission").zeroed()
                        })
                        .as_mut_slice()
                        [(left - offset) as usize..(right - offset) as usize]
                        .copy_from_slice(
                            &block.as_slice()[(left - at) as usize..(right - at) as usize],
                        );
                }
                remaining -= (right - left) as usize;
            }
            if remaining == 0
                && let Some(ready) = ready.take()
            {
                ready(Ok(result.take().expect("completed requested result")));
            }
            if let Some((ino, generation)) = cache_key {
                // Publish into the cache followers inspect, including after
                // pressure reclamation while the GET was in flight.
                let cache = self.cache.lock().unwrap();
                cache.insert(ino, generation, at, block);
                if right > left {
                    cache.consumed(
                        ino,
                        generation,
                        at,
                        (left - at) as usize,
                        (right - left) as usize,
                    );
                }
                fill.published();
                self.cache_published.notify_waiters();
            }
        }
        if let Some(error) = prefetch_error {
            return Err(error);
        }
        Ok(result)
    }

    async fn admit(&self, size: usize) -> Result<crate::budget::Reservation> {
        match self.data.try_reserve(size) {
            Ok(reservation) => Ok(reservation),
            Err(Error::Fs(code)) if code == libc::ENOMEM => {
                // Reclaim only as much old cache ownership as admission needs.
                // Pinned borrowers retain their permits; FIFO waiters stay ahead.
                {
                    let cache = self.cache.lock().unwrap();
                    while self.try_cache_loan(&cache) || cache.evict_lru() {
                        if let Ok(reservation) = self.data.try_reserve(size) {
                            return Ok(reservation);
                        }
                    }
                }
                if let Ok(reservation) = self.data.try_reserve(size) {
                    return Ok(reservation);
                }
                if self.config.use_enomem {
                    return self.data.try_reserve(size);
                }
                self.flush_all().await?;
                tokio::time::timeout(Duration::from_secs(30), async {
                    // Retain the same FIFO reservation while reclaiming owners
                    // published by fills that were in flight at the first sweep.
                    let reservation = self.data.reserve(size);
                    tokio::pin!(reservation);
                    loop {
                        let published = self.cache_published.notified();
                        // notify_waiters covers futures created before the
                        // cache check, including ones not yet polled.
                        tokio::select! {
                            biased;
                            result = &mut reservation => return result,
                            _ = async {
                                let reclaimed = {
                                    let cache = self.cache.lock().unwrap();
                                    self.try_cache_loan(&cache) || cache.evict_lru()
                                };
                                if !reclaimed {
                                    published.await;
                                }
                            } => {}
                        }
                    }
                })
                .await
                .map_err(|_| Error::Timeout("payload admission"))?
            }
            Err(error) => Err(error),
        }
    }

    fn try_cache_loan(&self, cache: &crate::cache::ReadCache) -> bool {
        // Never wait for optional capacity or lend while an upload/body owns it.
        let Ok(_activity) = self.flush_activity.try_write() else {
            return false;
        };
        cache.loan_lru(&self.flush_lane)
    }

    pub async fn write(&self, ino: u64, offset: u64, bytes: &[u8]) -> Result<usize> {
        self.check_write()?;
        if bytes.len() > MAX_IO {
            return Err(Error::Fs(libc::EINVAL));
        }
        if offset
            .checked_add(bytes.len() as u64)
            .is_none_or(|end| end > self.config.part_sizes.max_size())
        {
            return Err(Error::Fs(libc::EFBIG));
        }
        let node = self.node(ino)?;
        if file_type(node.state.lock().unwrap().mode) != FileType::RegularFile {
            return Err(Error::Fs(libc::EINVAL));
        }
        let reservation = self.admit(bytes.len()).await?;
        let ns = self.namespace.lock().unwrap();
        let mut state = node.state.lock().unwrap();
        if ns.is_moving(&state.key) || ns.is_deleting(&state.key) {
            return Err(Error::Fs(libc::EAGAIN));
        }
        write::insert_extent(&mut state, offset, reservation.copy(bytes)?)?;
        Ok(bytes.len())
    }

    pub fn truncate(&self, ino: u64, size: u64) -> Result<()> {
        self.check_write()?;
        if size > self.config.part_sizes.max_size() {
            return Err(Error::Fs(libc::EFBIG));
        }
        let node = self.node(ino)?;
        let ns = self.namespace.lock().unwrap();
        let mut state = node.state.lock().unwrap();
        if ns.is_moving(&state.key) || ns.is_deleting(&state.key) {
            return Err(Error::Fs(libc::EAGAIN));
        }
        if file_type(state.mode) != FileType::RegularFile {
            return Err(Error::Fs(libc::EINVAL));
        }
        write::truncate(&mut state, size)
    }

    fn prepare_metadata(&self, state: &mut State) {
        if self.config.enable_perms {
            state.metadata.insert(
                self.config.uid_attr.clone(),
                state.uid.to_string().into_bytes(),
            );
            state.metadata.insert(
                self.config.gid_attr.clone(),
                state.gid.to_string().into_bytes(),
            );
            state.metadata.insert(
                self.config.mode_attr.clone(),
                state.mode.to_string().into_bytes(),
            );
        }
        if self.config.specials_enabled() || state.mode & libc::S_IFMT != libc::S_IFREG {
            state.metadata.insert(
                self.config.mode_attr.clone(),
                state.mode.to_string().into_bytes(),
            );
            if state.rdev != 0 {
                state.metadata.insert(
                    self.config.rdev_attr.clone(),
                    state.rdev.to_string().into_bytes(),
                );
            }
        }
        if self.config.enable_mtime {
            state.metadata.insert(
                self.config.mtime_attr.clone(),
                state
                    .mtime
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
                    .to_string()
                    .into_bytes(),
            );
        }
    }

    pub async fn fsync(&self, ino: u64) -> Result<()> {
        let node = self.node(ino)?;
        let _gate = node.flush.lock().await;
        let key = node.state.lock().unwrap().key.clone();
        let busy = {
            let ns = self.namespace.lock().unwrap();
            ns.is_moving(&key) || ns.is_deleting(&key) || Self::pending_rename(&ns, &key, None)
        };
        if busy {
            return Err(Error::Fs(libc::EAGAIN));
        }
        self.flush_node_locked(&node).await
    }

    async fn flush_node_locked(&self, node: &Inode) -> Result<()> {
        {
            let mut state = node.state.lock().unwrap();
            self.prepare_metadata(&mut state);
            if self.config.no_dir_object && file_type(state.mode) == FileType::Directory {
                state.durable = state.version;
                return Ok(());
            }
        }
        let flusher = self
            .flushers
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| Error::Cancelled)?;
        // Shared guards preserve independent flush concurrency. The budget carries
        // activity into cloned SDK bodies and jobs, including cancellation cleanup.
        let lane = self
            .flush_lane
            .with_activity(self.flush_activity.clone().read_owned().await);
        self.cache.lock().unwrap().reclaim_loans();
        write::flush_locked(
            &self.store,
            node,
            &lane,
            flusher,
            &self.config,
            &self.upload_admission,
        )
        .await
    }

    pub async fn flush_all(&self) -> Result<()> {
        self.reconcile_pending_deletes().await?;
        // Snapshot only bounded live inode IDs, not a backend directory tree.
        let nodes: Vec<_> = self
            .namespace
            .lock()
            .unwrap()
            .nodes
            .values()
            .filter(|n| {
                let s = n.state.lock().unwrap();
                !s.deleted && s.version > s.durable
            })
            .cloned()
            .collect();
        for node in nodes {
            self.fsync(node.ino).await?;
        }
        Ok(())
    }

    pub async fn directory_chunk(
        &self,
        ino: u64,
        after: &str,
        limit: usize,
    ) -> Result<Vec<DirEntry>> {
        self.directory_chunk_inner(ino, after, limit, true).await
    }

    async fn directory_chunk_inner(
        &self,
        ino: u64,
        after: &str,
        limit: usize,
        resolve_types: bool,
    ) -> Result<Vec<DirEntry>> {
        let fetched = std::time::Instant::now();
        let (node, key, epoch) = {
            let ns = self.namespace.lock().unwrap();
            let node = ns.get(ino)?;
            let state = node.state.lock().unwrap();
            if file_type(state.mode) != FileType::Directory {
                return Err(Error::Fs(libc::ENOTDIR));
            }
            if state.deleted {
                return Err(Error::Fs(libc::ENOENT));
            }
            let key = state.key.clone();
            if resolve_types && (ns.is_moving(&key) || ns.is_deleting(&key)) {
                return Err(Error::Fs(libc::EAGAIN));
            }
            let epoch = node.directory_epoch.load(Ordering::Relaxed);
            if resolve_types
                && let Some(page) = self.directory_pages.get(&(ino, epoch, after.into(), limit))
                && page.fetched.elapsed() < self.config.stat_cache_ttl
                // READDIR does not retain lookup references. Namespace pressure
                // may have evicted an inode represented by this cached page.
                && page.entries.iter().all(|entry| ns.nodes.contains_key(&entry.ino))
            {
                return Ok(page.entries.clone());
            }
            drop(state);
            (node, key, epoch)
        };
        let prefix = if key.is_empty() {
            key
        } else {
            format!("{key}/")
        };
        let mut selected: BTreeMap<String, (bool, Option<ObjectHead>, bool)> = BTreeMap::new();
        {
            let mut ns = self.namespace.lock().unwrap();
            let hidden: Vec<_> = ns.tombstones.keys().cloned().collect();
            for (name, _) in ns.names.range(prefix.clone()..) {
                if !name.starts_with(&prefix) {
                    break;
                }
                let relative = &name[prefix.len()..];
                if relative.is_empty()
                    || relative.contains('/')
                    || relative <= after
                    || hidden.contains(name)
                {
                    continue;
                }
                let child = ns.named(name).unwrap();
                let state = child.state.lock().unwrap();
                if !state.deleted
                    && (state.version > state.durable
                        || state.fetched.elapsed() < self.config.stat_cache_ttl
                        || file_type(state.mode) == FileType::Directory && {
                            let prefix = format!("{name}/");
                            ns.names
                                .range(prefix.clone()..)
                                .take_while(|(name, _)| name.starts_with(&prefix))
                                .any(|(_, ino)| {
                                    ns.nodes.get(ino).is_some_and(|child| {
                                        let state = child.state.lock().unwrap();
                                        !state.deleted && state.version > state.durable
                                    })
                                })
                        })
                {
                    selected.insert(
                        relative.into(),
                        (
                            file_type(state.mode) == FileType::Directory,
                            state.remote.clone(),
                            state.metadata_known,
                        ),
                    );
                }
                if selected.len() > limit {
                    selected.pop_last();
                }
            }
            ns.hidden("");
        }
        let mut token = None;
        loop {
            let page = self
                .store
                .list(&prefix, Some("/"), token.as_deref())
                .await?;
            let mut page_last = String::new();
            for (name, directory, head, known) in page
                .objects
                .into_iter()
                .filter(|o| o.key != prefix)
                .map(|o| {
                    let known = o.metadata.is_some();
                    let metadata = o
                        .metadata
                        .and_then(|metadata| {
                            crate::backend::metadata::encode_metadata(&metadata).ok()
                        })
                        .unwrap_or_default()
                        .into_iter()
                        .collect();
                    let head = ObjectHead {
                        size: o.size,
                        etag: o.etag.unwrap_or_default(),
                        metadata,
                        last_modified: o.last_modified.and_then(|d| SystemTime::try_from(d).ok()),
                        ..Default::default()
                    };
                    let directory = o.key.ends_with('/');
                    (o.key, directory, Some(head), known)
                })
                .chain(
                    page.common_prefixes
                        .into_iter()
                        .map(|p| (p, true, None, false)),
                )
            {
                let Some(name) = name.strip_prefix(&prefix) else {
                    return Err(Error::InvalidResponse("directory prefix escape".into()));
                };
                let name = name.trim_end_matches('/');
                if name.is_empty() || name.contains('/') {
                    continue;
                }
                page_last = page_last.max(name.into());
                if name <= after {
                    continue;
                }
                let full = child(prefix.trim_end_matches('/'), name)?;
                if self.namespace.lock().unwrap().hidden(&full) {
                    continue;
                }
                selected
                    .entry(name.into())
                    .or_insert((directory, head, known));
                if selected.len() > limit {
                    selected.pop_last();
                }
            }
            if page.next_token.is_none()
                || selected.len() == limit
                    && selected
                        .last_key_value()
                        .is_some_and(|(name, _)| *name <= page_last)
            {
                break;
            }
            token = page.next_token;
        }
        // Safe fuser exposes no DT_UNKNOWN variant. Standard LIST has no user
        // metadata, so resolve only this bounded selected page before claiming a
        // concrete file type. Extended authoritative metadata avoids these HEADs.
        if resolve_types {
            let mut jobs = FuturesUnordered::new();
            let mut pending = selected
                .iter_mut()
                .filter(|(_, entry)| !entry.0 && !entry.2);
            let mut next = pending.next();
            while next.is_some() || !jobs.is_empty() {
                tokio::select! {
                    completed = jobs.next(), if !jobs.is_empty() => {
                        completed.expect("admitted metadata job")?;
                    }
                    admitted = tokio::time::timeout(
                        self.config.http_timeout,
                        self.metadata_slots.clone().acquire_owned(),
                    ), if next.is_some() => {
                        let permit = admitted
                            .map_err(|_| Error::Timeout("directory metadata admission"))?
                            .map_err(|_| Error::Cancelled)?;
                        let (name, (directory, head, known)) = next.take().expect("selected unknown type");
                        // Admission precedes creation of each future: concurrent
                        // pages cannot accumulate per-entry semaphore waiters.
                        // Borrowed futures drop synchronously on error/cancel.
                        jobs.push(async move {
                            let _permit = permit;
                            let resolved = tokio::time::timeout(
                                self.config.http_timeout,
                                self.lookup(ino, name),
                            ).await.map_err(|_| Error::Timeout("directory metadata lookup"))??;
                            let state = resolved.state.lock().unwrap();
                            *directory = file_type(state.mode) == FileType::Directory;
                            *head = state.remote.clone();
                            *known = state.metadata_known;
                            Ok::<(), Error>(())
                        });
                        next = pending.next();
                    }
                }
            }
        }
        let mut ns = self.namespace.lock().unwrap();
        if node.directory_epoch.load(Ordering::Relaxed) != epoch
            || ns.names.get(prefix.trim_end_matches('/')) != Some(&ino)
        {
            return Err(Error::Fs(libc::EAGAIN));
        }
        let mut entries = Vec::with_capacity(selected.len());
        for (name, (directory, head, known)) in selected {
            let full = child(prefix.trim_end_matches('/'), &name)?;
            let node = ns.install(&full, |ino| {
                let node = Inode::new(ino, full.clone(), directory, head, &self.config);
                node.state.lock().unwrap().metadata_known = known;
                node
            })?;
            entries.push(DirEntry {
                ino: node.ino,
                name,
                kind: node.attr().kind,
            });
        }
        if resolve_types {
            self.directory_pages.insert(
                (ino, epoch, after.into(), limit),
                Arc::new(DirectoryPage {
                    fetched,
                    entries: entries.clone(),
                }),
            );
            self.directory_pages.run_pending_tasks();
        }
        Ok(entries)
    }

    pub async fn unlink(&self, parent: u64, name: &str, directory: bool) -> Result<()> {
        self.check_write()?;
        let key = self.child_key(parent, name)?;
        let pending = {
            let ns = self.namespace.lock().unwrap();
            ns.pending_deletes
                .get(&key)
                .and_then(|ino| ns.nodes.get(ino))
                .cloned()
        };
        let node = match pending {
            Some(node) => node,
            None => self.lookup(parent, name).await?,
        };
        if (node.attr().kind == FileType::Directory) != directory {
            return Err(Error::Fs(if directory {
                libc::ENOTDIR
            } else {
                libc::EISDIR
            }));
        }
        self.unlink_node(node, key, directory, Some((parent, name)))
            .await
    }

    /// Resolve owned ambiguous DELETEs before shutdown/retry. The retained name
    /// binding excludes recreation; only the original inode can complete it.
    pub async fn reconcile_pending_deletes(&self) -> Result<()> {
        let pending: Vec<_> = {
            let ns = self.namespace.lock().unwrap();
            ns.pending_deletes
                .iter()
                .map(|(key, ino)| Ok((key.clone(), ns.get(*ino)?)))
                .collect::<Result<_>>()?
        };
        for (key, node) in pending {
            let directory = node.attr().kind == FileType::Directory;
            self.unlink_node(node, key, directory, None).await?;
        }
        Ok(())
    }

    async fn unlink_node(
        &self,
        node: Arc<Inode>,
        key: String,
        directory: bool,
        parent: Option<(u64, &str)>,
    ) -> Result<()> {
        let _gate = node.flush.lock().await;
        {
            let ns = self.namespace.lock().unwrap();
            let state = node.state.lock().unwrap();
            if state.deleted
                || state.key != key
                || ns.names.get(&key) != Some(&node.ino)
                || Self::pending_rename(&ns, &key, None)
            {
                return Err(Error::Fs(libc::ENOENT));
            }
        }
        let _reservation = MoveLease::begin_delete(&self.namespace, key.clone(), node.ino)?;
        {
            let ns = self.namespace.lock().unwrap();
            let state = node.state.lock().unwrap();
            if state.deleted || state.key != key || ns.names.get(&key) != Some(&node.ino) {
                return Err(Error::Fs(libc::ENOENT));
            }
            if let Some((parent, name)) = parent {
                let parent = ns.get(parent)?;
                let parent = parent.state.lock().unwrap();
                if parent.deleted || child(&parent.key, name)? != key {
                    return Err(Error::Fs(libc::ENOENT));
                }
            }
        }
        // An emptiness probe publishes no dirent types and holds this prefix's
        // reservation; it must not recursively look up children through that lease.
        if directory
            && !self
                .directory_chunk_inner(node.ino, "", 1, false)
                .await?
                .is_empty()
        {
            return Err(Error::Fs(libc::ENOTEMPTY));
        }
        {
            let mut ns = self.namespace.lock().unwrap();
            ns.tombstone(&key)?;
            ns.pending_deletes.insert(key.clone(), node.ino);
        }
        let remote_key = if directory {
            format!("{key}/")
        } else {
            key.clone()
        };
        // Cancellation and ambiguous errors leave a bounded pending record, not
        // an irreversibly hidden/deleted dirty inode. Retrying unlink reconciles
        // the idempotent DELETE while recreation and conflicting moves stay out.
        if let Err(error) = self.store.delete(&remote_key).await {
            // A modeled rejection is not an ambiguous applied DELETE. Restore
            // ordinary lookup on authorization/validation failures only.
            if matches!(&error,Error::Backend{code,..} if matches!(code.as_str(),"AccessDenied"|"InvalidAccessKeyId"|"SignatureDoesNotMatch"|"InvalidArgument"|"InvalidRequest"))
            {
                let mut ns = self.namespace.lock().unwrap();
                ns.pending_deletes.remove(&key);
                ns.tombstones.remove(&key);
                ns.changed(&key);
            }
            return Err(error);
        }
        let mut ns = self.namespace.lock().unwrap();
        ns.pending_deletes.remove(&key);
        ns.names.remove(&key);
        ns.changed(&key);
        let mut state = node.state.lock().unwrap();
        state.deleted = true;
        state.extents.clear();
        state.durable = state.version;
        Ok(())
    }

    pub async fn rename(
        &self,
        parent: u64,
        name: &str,
        newparent: u64,
        newname: &str,
    ) -> Result<()> {
        self.check_write()?;
        let source = self.lookup(parent, name).await?;
        let destination = self.child_key(newparent, newname)?;
        let old = source.state.lock().unwrap().key.clone();
        {
            let ns = self.namespace.lock().unwrap();
            if Self::pending_rename(&ns, &old, Some(source.ino))
                || Self::pending_rename(&ns, &destination, Some(source.ino))
                || [&old, &destination].into_iter().any(|key| {
                    let prefix = format!("{key}/");
                    ns.pending_moves
                        .range(prefix.clone()..)
                        .take_while(|(key, _)| key.starts_with(&prefix))
                        .any(|(_, ino)| *ino != source.ino)
                })
            {
                return Err(Error::Fs(libc::EBUSY));
            }
        }
        if destination == old {
            return Ok(());
        }
        if destination.starts_with(&format!("{old}/")) {
            return Err(Error::Fs(libc::EINVAL));
        }
        let directory = source.attr().kind == FileType::Directory;
        let retry =
            source.state.lock().unwrap().rename_target.as_deref() == Some(destination.as_str());
        let target = match self.lookup(newparent, newname).await {
            Ok(target) => {
                if (target.attr().kind == FileType::Directory) != directory {
                    return Err(Error::Fs(if directory {
                        libc::ENOTDIR
                    } else {
                        libc::EISDIR
                    }));
                }
                if directory && !retry && !self.directory_chunk(target.ino, "", 1).await?.is_empty()
                {
                    return Err(Error::Fs(libc::ENOTEMPTY));
                }
                Some(target)
            }
            Err(error) if not_found(&error) => None,
            Err(error) => return Err(error),
        };
        let _moving = MoveLease::begin(&self.namespace, vec![old.clone(), destination.clone()])?;
        let mut pinned: Vec<_> = {
            let ns = self.namespace.lock().unwrap();
            ns.nodes
                .values()
                .filter(|node| {
                    let state = node.state.lock().unwrap();
                    state.key == old || directory && state.key.starts_with(&format!("{old}/"))
                })
                .cloned()
                .collect()
        };
        if let Some(target) = &target {
            pinned.push(target.clone());
        }
        pinned.sort_by_key(|node| node.ino);
        pinned.dedup_by_key(|node| node.ino);
        let mut gates = Vec::with_capacity(pinned.len());
        for node in &pinned {
            gates.push(node.flush.clone().lock_owned().await);
        }
        {
            let mut state = source.state.lock().unwrap();
            if state.deleted || state.key != old {
                return Err(Error::Fs(libc::ENOENT));
            }
            if state
                .rename_target
                .as_ref()
                .is_some_and(|key| *key != destination)
            {
                return Err(Error::Fs(libc::EBUSY));
            }
            state.rename_target = Some(destination.clone());
        }
        {
            let mut ns = self.namespace.lock().unwrap();
            ns.pending_moves.insert(old.clone(), source.ino);
            ns.pending_moves.insert(destination.clone(), source.ino);
        }
        self.flush_node_locked(&source).await?;
        let mut renamed_head = None;
        if directory {
            // Drain paged source objects incrementally; never materialize a subtree.
            let prefix = format!("{old}/");
            let cached: BTreeMap<_, _> = pinned
                .iter()
                .map(|node| (node.state.lock().unwrap().key.clone(), node.clone()))
                .collect();
            let mut last_drained = None;
            loop {
                let page = self.store.list(&prefix, None, None).await?;
                if page.objects.is_empty() {
                    break;
                }
                for object in page.objects {
                    if last_drained
                        .as_ref()
                        .is_some_and(|key: &String| object.key <= *key)
                    {
                        return Err(Error::Fs(libc::EAGAIN));
                    }
                    let suffix = object
                        .key
                        .strip_prefix(&prefix)
                        .ok_or_else(|| Error::InvalidResponse("rename prefix escape".into()))?;
                    let target = format!("{destination}/{suffix}");
                    let etag = match object.etag {
                        Some(etag) => etag,
                        None => self.store.head(&object.key).await?.etag,
                    };
                    let copied = self.store.copy(&object.key, &target, &etag).await?;
                    if let Some(node) = cached.get(object.key.trim_end_matches('/')) {
                        let head = self.store.head(&target).await?;
                        if head.etag != copied {
                            return Err(Error::Fs(libc::ESTALE));
                        }
                        let mut state = node.state.lock().unwrap();
                        state.remote = Some(head);
                        state.data_key = Some(target.trim_end_matches('/').into());
                    }
                    self.store.delete(&object.key).await?;
                    last_drained = Some(object.key);
                }
            }
        } else {
            let needs_copy =
                source.state.lock().unwrap().data_key.as_deref() != Some(destination.as_str());
            if needs_copy {
                let mut head = source
                    .state
                    .lock()
                    .unwrap()
                    .remote
                    .clone()
                    .ok_or(Error::Fs(libc::EIO))?;
                head.etag = self.store.copy(&old, &destination, &head.etag).await?;
                // Publish the backing key before DELETE can remove the old
                // source, including cancellation or a lost DELETE response.
                let mut state = source.state.lock().unwrap();
                state.remote = Some(head);
                state.data_key = Some(destination.clone());
            }
            let head = self.store.head(&destination).await?;
            if source
                .state
                .lock()
                .unwrap()
                .remote
                .as_ref()
                .is_none_or(|remote| remote.etag != head.etag)
            {
                return Err(Error::Fs(libc::ESTALE));
            }
            renamed_head = Some(head);
            self.store.delete(&old).await?;
        }
        // Copy can change ETags, including for cached descendants. Refresh those
        // generations while their per-inode commit gates remain held.
        let mut refreshed = BTreeMap::new();
        if directory {
            for node in &pinned {
                let info = {
                    let state = node.state.lock().unwrap();
                    (
                        state.key.clone(),
                        state.remote.is_some(),
                        file_type(state.mode) == FileType::Directory,
                    )
                };
                if info.0.starts_with(&format!("{old}/")) && info.1 {
                    let new = format!(
                        "{destination}{}{}",
                        &info.0[old.len()..],
                        if info.2 { "/" } else { "" }
                    );
                    refreshed.insert(node.ino, self.store.head(&new).await?);
                }
            }
        }
        let mut ns = self.namespace.lock().unwrap();
        ns.tombstone(&old)?;
        ns.tombstones.remove(&destination);
        if let Some(ino) = ns.names.remove(&destination)
            && let Some(target) = ns.nodes.get(&ino)
        {
            let mut state = target.state.lock().unwrap();
            state.deleted = true;
            state.extents.clear();
            state.durable = state.version;
        }
        let changed: Vec<_> = ns
            .names
            .iter()
            .filter(|(key, _)| **key == old || key.starts_with(&format!("{old}/")))
            .map(|(key, &ino)| (key.clone(), ino))
            .collect();
        for (key, ino) in changed {
            let new = format!("{destination}{}", &key[old.len()..]);
            ns.names.remove(&key);
            ns.names.insert(new.clone(), ino);
            if let Some(node) = ns.nodes.get(&ino) {
                let mut state = node.state.lock().unwrap();
                state.key = new;
                state.data_key = None;
                state.negative_lookups.clear();
                if let Some(head) = refreshed.remove(&ino) {
                    state.remote = Some(head);
                }
                if ino == source.ino {
                    state.rename_target = None;
                }
                if ino == source.ino
                    && let Some(head) = renamed_head.take()
                {
                    state.remote = Some(head);
                }
            }
        }
        ns.changed(&destination);
        ns.pending_moves.remove(&old);
        ns.pending_moves.remove(&destination);
        ns.get(newparent)?
            .state
            .lock()
            .unwrap()
            .negative_lookups
            .remove(newname);
        Ok(())
    }

    pub fn setattr(
        &self,
        ino: u64,
        mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        mtime: Option<SystemTime>,
    ) -> Result<FileAttr> {
        self.check_write()?;
        if ino == 1 {
            return Err(Error::Fs(libc::EOPNOTSUPP));
        }
        let node = self.node(ino)?;
        {
            let ns = self.namespace.lock().unwrap();
            let mut state = node.state.lock().unwrap();
            if ns.is_moving(&state.key) || ns.is_deleting(&state.key) {
                return Err(Error::Fs(libc::EAGAIN));
            }
            if state.deleted {
                return Err(Error::Fs(libc::ENOENT));
            }
            if self.config.enable_perms {
                if let Some(mode) = mode {
                    state.mode = (state.mode & libc::S_IFMT) | (mode & 0o7777);
                }
                if let Some(uid) = uid {
                    state.uid = uid;
                }
                if let Some(gid) = gid {
                    state.gid = gid;
                }
            }
            if let Some(mtime) = mtime {
                state.mtime = mtime;
            }
            state.version += 1;
            self.prepare_metadata(&mut state);
        }
        Ok(node.attr())
    }

    pub fn xattrs(&self, ino: u64) -> Result<BTreeMap<String, Vec<u8>>> {
        if self.config.disable_xattr {
            return Err(Error::Fs(libc::ENOSYS));
        }
        let node = self.node(ino)?;
        let state = node.state.lock().unwrap();
        let mut attrs: BTreeMap<_, _> = state
            .metadata
            .iter()
            .map(|(k, v)| (format!("user.{k}"), v.clone()))
            .collect();
        if let Some(remote) = &state.remote {
            attrs.insert("s3.etag".into(), remote.etag.as_bytes().to_vec());
        }
        Ok(attrs)
    }

    pub fn setxattr(&self, ino: u64, name: &str, value: Option<&[u8]>, flags: i32) -> Result<()> {
        self.check_write()?;
        if self.config.disable_xattr {
            return Err(Error::Fs(libc::ENOSYS));
        }
        let node = self.node(ino)?;
        if ino == 1 {
            return Err(Error::Fs(libc::EOPNOTSUPP));
        }
        let ns = self.namespace.lock().unwrap();
        let mut state = node.state.lock().unwrap();
        if ns.is_moving(&state.key) || ns.is_deleting(&state.key) {
            return Err(Error::Fs(libc::EAGAIN));
        }
        if state.deleted {
            return Err(Error::Fs(libc::ENOENT));
        }
        let Some(name) = name.strip_prefix("user.") else {
            return Ok(());
        };
        if name == self.config.symlink_attr {
            return Ok(());
        }
        let exists = state.metadata.contains_key(name);
        if flags & libc::XATTR_CREATE != 0 && exists {
            return Err(Error::Fs(libc::EEXIST));
        }
        if flags & libc::XATTR_REPLACE != 0 && !exists || value.is_none() && !exists {
            return Err(Error::Fs(libc::ENODATA));
        }
        let size: usize = state
            .metadata
            .iter()
            .filter(|(k, _)| k.as_str() != name)
            .map(|(k, v)| k.len() + v.len())
            .sum();
        if size + name.len() + value.map_or(0, <[u8]>::len) > 65536 {
            return Err(Error::Fs(libc::E2BIG));
        }
        if let Some(value) = value {
            state.metadata.insert(name.into(), value.to_vec());
        } else {
            state.metadata.remove(name);
        }
        state.version += 1;
        Ok(())
    }

    pub async fn shutdown(&self) -> Result<()> {
        self.closing.store(true, Ordering::Release);
        let result = self.flush_all().await;
        // Cancelled multipart tasks retain their admission token through abort.
        // Do not tear down the runtime while that bounded cleanup is in flight.
        let _idle = self
            .flushers
            .acquire_many(self.config.max_flushers as u32)
            .await
            .map_err(|_| Error::Cancelled)?;
        result?;
        if self
            .namespace
            .lock()
            .unwrap()
            .nodes
            .values()
            .any(|node| node.state.lock().unwrap().rename_target.is_some())
        {
            return Err(Error::Backend {
                operation: "shutdown",
                code: "PendingRename".into(),
                message: "a failed rename must be retried before successful shutdown".into(),
            });
        }
        Ok(())
    }
    pub fn is_closing(&self) -> bool {
        self.closing.load(Ordering::Acquire)
    }
}
