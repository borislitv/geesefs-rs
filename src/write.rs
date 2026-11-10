use crate::{
    backend::{
        Store,
        metadata::{CachedMetadata, CachedRemote, UPLOAD_RECEIPT},
        s3::ObjectHead,
    },
    budget::{Block, Budget},
    config::Config,
    error::{Error, Result},
    inode::{Extent, Inode, State},
};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

pub const PART_SIZE: usize = 5 * 1024 * 1024;
pub const MAX_IO: usize = 1024 * 1024;

#[derive(Clone)]
pub struct UploadAdmission {
    pub(crate) queue: Arc<tokio::sync::Semaphore>,
    pub(crate) parts: Arc<tokio::sync::Semaphore>,
    pub(crate) copies: Arc<tokio::sync::Semaphore>,
}
impl UploadAdmission {
    pub fn new(config: &Config) -> Self {
        Self {
            queue: Arc::new(tokio::sync::Semaphore::new(config.max_requests)),
            parts: Arc::new(tokio::sync::Semaphore::new(config.max_flushers)),
            copies: Arc::new(tokio::sync::Semaphore::new(config.max_parallel_copy)),
        }
    }
}

pub fn insert_extent(state: &mut State, offset: u64, block: Block) -> Result<()> {
    let length = block.as_slice().len();
    if length == 0 {
        return Ok(());
    }
    if state.deleted {
        return Err(Error::Fs(libc::ENOENT));
    }
    let end = offset
        .checked_add(length as u64)
        .ok_or(Error::Fs(libc::EFBIG))?;
    if state.extents.len() >= 65536 {
        return Err(Error::Fs(libc::ENOMEM));
    }
    state.admit_metadata(
        3 * (std::mem::size_of::<(u64, Extent)>() + crate::metadata_budget::LINKS),
    )?;
    let _temporary = state
        .metadata_budget()
        .charge(2 * state.extents.len() * std::mem::size_of::<(u64, Extent)>())?;
    let overlaps: Vec<_> = state
        .extents
        .range(..end)
        .filter(|(_, e)| e.end > offset)
        .map(|(&start, e)| (start, e.clone()))
        .collect();
    for (start, extent) in overlaps {
        state.extents.remove(&start);
        if start < offset {
            state.extents.insert(
                start,
                Extent {
                    end: offset,
                    ..extent.clone()
                },
            );
        }
        if extent.end > end {
            state.extents.insert(
                end,
                Extent {
                    data_offset: extent.data_offset + (end - start) as usize,
                    ..extent
                },
            );
        }
    }
    state.version += 1;
    state.extents.insert(
        offset,
        Extent {
            end,
            data: block,
            data_offset: 0,
            version: state.version,
        },
    );
    state.size = state.size.max(end);
    state.mtime = std::time::SystemTime::now();
    state.settle_metadata();
    Ok(())
}

pub fn truncate(state: &mut State, size: u64) -> Result<()> {
    if state.deleted {
        return Err(Error::Fs(libc::ENOENT));
    }
    if size < state.size && state.cuts.len() >= 65536 {
        return Err(Error::Fs(libc::ENOMEM));
    }
    state.admit_metadata(std::mem::size_of::<(u64, u64)>() + crate::metadata_budget::LINKS)?;
    state.version += 1;
    if size < state.size {
        if state.cuts.len() >= 65536 {
            return Err(Error::Fs(libc::ENOMEM));
        }
        state.truncate_version = state.version;
        state.cuts.insert(state.version, size);
        state.visible_remote_size = state.visible_remote_size.min(size);
        state.extents.retain(|&start, e| {
            e.end = e.end.min(size);
            start < size
        });
    }
    state.size = size;
    state.mtime = std::time::SystemTime::now();
    state.settle_metadata();
    Ok(())
}

pub struct Snapshot {
    pub(crate) metadata_charge: crate::metadata_budget::MetadataCharge,
    pub(crate) key: crate::metadata_budget::SharedKey,
    pub(crate) size: u64,
    pub(crate) version: u64,
    pub(crate) remote: Option<CachedRemote>,
    pub(crate) visible: u64,
    pub(crate) extents: BTreeMap<u64, Extent>,
    pub(crate) metadata: CachedMetadata,
}

impl Snapshot {
    pub fn key(&self) -> &str {
        &self.key
    }
    pub fn size(&self) -> u64 {
        self.size
    }
    pub fn version(&self) -> u64 {
        self.version
    }
    pub fn remote(&self) -> Option<&crate::backend::metadata::RemoteRecord> {
        self.remote.as_deref()
    }
    fn set_remote(
        &mut self,
        remote: CachedRemote,
        credit: &mut crate::metadata_budget::MetadataCharge,
    ) -> Result<()> {
        let previous = self.remote.replace(remote);
        let result = self
            .metadata_charge
            .ensure_from(self.metadata_bytes(), credit);
        if result.is_err() {
            self.remote = previous;
        }
        result?;
        self.settle_metadata()
    }
    fn metadata_bytes(&self) -> usize {
        use crate::metadata_budget::LINKS;
        std::mem::size_of::<Self>()
            // One transient conditional-read DTO per snapshot, not another
            // retained metadata map or another charge for the shared baseline.
            + self.remote.as_ref().map_or(0, |remote| std::mem::size_of::<ObjectHead>() + remote.etag.len())
            + self.extents.len() * (std::mem::size_of::<(u64, Extent)>() + LINKS)
    }
    pub(crate) fn settle_metadata(&mut self) -> Result<()> {
        if !self.metadata_charge.budget().enabled() {
            return Ok(());
        }
        let bytes = self.metadata_bytes();
        if bytes > self.metadata_charge.bytes() {
            return Err(Error::Fs(libc::ENOMEM));
        }
        // Settlement never attempts late admission after a copy was built.
        self.metadata_charge.resize(bytes)
    }
    pub(crate) fn try_clone(&self) -> Result<Self> {
        let charge = self
            .metadata_charge
            .budget()
            .charge_with(|| 2 * self.metadata_bytes())?;
        let mut snapshot = Self {
            metadata_charge: charge,
            key: self.key.clone(),
            size: self.size,
            version: self.version,
            remote: self.remote.clone(),
            visible: self.visible,
            extents: self.extents.clone(),
            metadata: self.metadata.clone(),
        };
        snapshot.settle_metadata()?;
        Ok(snapshot)
    }
}

/// Small in-memory receipt; dirty payload stays with the inode's normal extents.
pub(crate) struct PendingUpload {
    _metadata_charge: crate::metadata_budget::MetadataCharge,
    key: crate::metadata_budget::SharedKey,
    size: u64,
    version: u64,
    receipt: String,
    metadata: CachedMetadata,
    // Retained on ambiguous/cancelled commits so a matching receipt can be
    // projected without competing with newer writes for fresh global admission.
    progress_credit: std::sync::Mutex<crate::metadata_budget::MetadataCharge>,
}

// Own request metadata is known before mutation. ETags/properties in a future
// HEAD are not: fund two projection working sets with 1 KiB ETag slack each.
// Responses beyond this bound require additional admission and can fail without
// acknowledging durability; no finite reservation promises arbitrary HEAD data.
const COMMIT_ETAG_SLACK: usize = 1024;

fn canonical_metadata(
    wire: &HashMap<String, String>,
    current: &CachedMetadata,
    credit: &mut crate::metadata_budget::MetadataCharge,
) -> Result<CachedMetadata> {
    let decoded = crate::backend::metadata::decode_metadata(
        wire.iter().map(|(k, v)| (k.as_str(), v.as_str())),
    );
    if current
        .iter()
        .eq(decoded.iter().map(|(k, v)| (k.as_str(), v.as_slice())))
    {
        Ok(current.clone())
    } else {
        CachedMetadata::from_admitted(decoded, credit)
    }
}

fn project_head(
    head: ObjectHead,
    previous: Option<&CachedRemote>,
    current: &CachedMetadata,
    credit: &mut crate::metadata_budget::MetadataCharge,
) -> Result<CachedRemote> {
    let required = credit
        .budget()
        .estimate(|| Inode::metadata_estimate("", Some(&head)));
    // For our own metadata this fits the pre-mutation operation credit. A truly
    // larger external response is admitted before decoding, or returns ENOMEM.
    if required > credit.bytes() {
        credit.resize(required)?;
    }
    let metadata = canonical_metadata(&head.metadata, current, credit)?;
    if let Some(previous) = previous
        && previous.size == head.size
        && previous.etag == head.etag
        && previous.metadata == metadata
    {
        return Ok(previous.clone());
    }
    CachedRemote::from_admitted(head, metadata, credit)
}

pub(crate) fn metadata_growth(config: &Config) -> usize {
    [
        &config.uid_attr,
        &config.gid_attr,
        &config.mode_attr,
        &config.rdev_attr,
        &config.mtime_attr,
    ]
    .into_iter()
    .map(|key| {
        key.len() + 32 + std::mem::size_of::<(String, Vec<u8>)>() + crate::metadata_budget::LINKS
    })
    .sum()
}

pub(crate) fn prepare_metadata(config: &Config, state: &mut State) -> Result<()> {
    let mut metadata = state.metadata.clone();
    if config.enable_perms {
        metadata.try_insert(&config.uid_attr, state.uid.to_string().as_bytes())?;
        metadata.try_insert(&config.gid_attr, state.gid.to_string().as_bytes())?;
        metadata.try_insert(&config.mode_attr, state.mode.to_string().as_bytes())?;
    }
    if config.specials_enabled() || state.mode & libc::S_IFMT != libc::S_IFREG {
        metadata.try_insert(&config.mode_attr, state.mode.to_string().as_bytes())?;
        if state.rdev != 0 {
            metadata.try_insert(&config.rdev_attr, state.rdev.to_string().as_bytes())?;
        }
    }
    if config.enable_mtime {
        metadata.try_insert(
            &config.mtime_attr,
            state
                .mtime
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
                .to_string()
                .as_bytes(),
        )?;
    }
    state.metadata = metadata;
    state.settle_metadata();
    Ok(())
}

pub fn snapshot(node: &Inode, config: &Config) -> Result<Option<Snapshot>> {
    let mut state = node.state.lock().unwrap();
    if state.deleted {
        return Err(Error::Fs(libc::ENOENT));
    }
    if state.version == state.durable {
        return Ok(None);
    }
    // Admission may have waited while writes changed both version and mtime.
    // Generated attributes and payload must describe the same locked generation.
    prepare_metadata(config, &mut state)?;
    if state
        .metadata
        .keys()
        .any(|key| key.eq_ignore_ascii_case(UPLOAD_RECEIPT))
    {
        return Err(Error::Config(
            "geesefs-upload-receipt is reserved for upload provenance".into(),
        ));
    }
    let charge = state
        .metadata_budget()
        .charge_with(|| 2 * state.metadata_bytes())?;
    let mut snapshot = Snapshot {
        metadata_charge: charge,
        key: if state.mode & libc::S_IFMT == libc::S_IFDIR && !state.key.is_empty() {
            let key = state.data_key.as_ref().unwrap_or(&state.key);
            let mut key_charge = state.metadata_budget().charge(key.len() + 1 + 128)?;
            let value = format!("{key}/");
            crate::metadata_budget::SharedKey::from_admitted(value, &mut key_charge)?
        } else {
            state.data_key.as_ref().unwrap_or(&state.key).clone()
        },
        size: if state.mode & libc::S_IFMT == libc::S_IFREG {
            state.size
        } else {
            0
        },
        version: state.version,
        remote: state.remote.clone(),
        visible: state.visible_remote_size,
        extents: state.extents.clone(),
        metadata: state.metadata.clone(),
    };
    snapshot.settle_metadata()?;
    Ok(Some(snapshot))
}

pub fn overlay(snapshot: &Snapshot, start: u64, data: &mut [u8]) {
    let end = start + data.len() as u64;
    for (&offset, extent) in snapshot.extents.range(..end) {
        let left = offset.max(start);
        let right = extent.end.min(end);
        if right <= left {
            continue;
        }
        let source = extent.data_offset + (left - offset) as usize;
        data[(left - start) as usize..(right - start) as usize]
            .copy_from_slice(&extent.data.as_slice()[source..source + (right - left) as usize]);
    }
}

/// Caller holds this inode's flush gate. New writes may proceed while a snapshot
/// uploads; only the completed generation is retired. Errors retain dirty data.
pub async fn flush_locked(
    store: &Arc<dyn Store>,
    node: &Inode,
    lane: &Budget,
    flusher: tokio::sync::OwnedSemaphorePermit,
    config: &Config,
    admission: &UploadAdmission,
) -> Result<()> {
    flush_locked_before(store, node, lane, flusher, config, admission, None).await
}

pub(crate) type BeforeMutation<'a> = dyn FnMut() -> Result<()> + Send + 'a;

fn will_mutate(before: &mut Option<&mut BeforeMutation<'_>>) -> Result<()> {
    if let Some(before) = before {
        before()?;
    }
    Ok(())
}

/// The callback publishes namespace intent only after first-request metadata
/// and body admission, immediately before a possibly applied remote mutation.
pub(crate) async fn flush_locked_before(
    store: &Arc<dyn Store>,
    node: &Inode,
    lane: &Budget,
    flusher: tokio::sync::OwnedSemaphorePermit,
    config: &Config,
    admission: &UploadAdmission,
    mut before: Option<&mut BeforeMutation<'_>>,
) -> Result<()> {
    let mut flusher = Some(flusher);
    reconcile_upload(store, node).await?;
    let Some(mut snapshot) = snapshot(node, config)? else {
        return Ok(());
    };
    if snapshot.size > config.part_sizes.max_size() {
        return Err(Error::Fs(libc::EFBIG));
    }
    if config.enable_patch
        && patchable(&snapshot, config)
        && let Some(head) =
            patch_snapshot(store, node, &mut snapshot, lane, config, 0, &mut before).await?
    {
        return retire(node, &snapshot, head);
    }
    let single_limit = config.single_part.max(5) * 1024 * 1024;
    let prefix = config.part_sizes.range(1).map_or(0, |range| range.end);
    // Bootstrap only new/fully truncated uploads: committing a short prefix of
    // an existing object would destroy an uncached source tail before copying it.
    let mut bootstrap = config.prefer_patch_uploads
        && snapshot.visible == 0
        && prefix > 0
        && snapshot.size > prefix
        && snapshot.size > single_limit;
    loop {
        // Keep request-map copies charged until the operation completes or is
        // cancelled. The durable receipt has its own, longer-lived owner.
        let mut request_metadata = snapshot.metadata_charge.budget().charge_with(|| {
            3 * (snapshot.metadata_bytes() + snapshot.metadata.encoding_bytes()) + 1024
        })?;
        let upload_size = if bootstrap { prefix } else { snapshot.size };
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).map_err(|_| Error::Fs(libc::EIO))?;
        let receipt = random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let mut metadata: HashMap<_, _> = snapshot.metadata.encode()?.into_iter().collect();
        metadata.insert(UPLOAD_RECEIPT.into(), receipt.clone());
        let expected = ObjectHead {
            metadata,
            ..Default::default()
        };
        let projection_bytes = request_metadata
            .budget()
            .estimate(|| 2 * (Inode::metadata_estimate("", Some(&expected)) + COMMIT_ETAG_SLACK));
        let required = request_metadata
            .budget()
            .estimate(|| projection_bytes + 2 * snapshot.metadata.encoding_bytes() + 1024);
        if required > request_metadata.bytes() {
            request_metadata.resize(required)?;
        }
        let metadata = canonical_metadata(
            &expected.metadata,
            &snapshot.metadata,
            &mut request_metadata,
        )?;
        let progress_credit = request_metadata.take(projection_bytes)?;
        let pending_charge = snapshot.metadata_charge.budget().charge(
            std::mem::size_of::<PendingUpload>() + snapshot.key.len() + receipt.capacity(),
        )?;
        let pending = Arc::new(PendingUpload {
            _metadata_charge: pending_charge,
            key: snapshot.key.clone(),
            size: upload_size,
            version: snapshot.version,
            receipt,
            metadata,
            progress_credit: std::sync::Mutex::new(progress_credit),
        });
        let committed = if !bootstrap && upload_size <= single_limit {
            let _part =
                tokio::time::timeout(config.http_timeout, admission.parts.clone().acquire_owned())
                    .await
                    .map_err(|_| Error::Timeout("PUT admission"))?
                    .map_err(|_| Error::Cancelled)?;
            let body = crate::upload_body::stream(store, &snapshot, 0, snapshot.size, lane)?;
            will_mutate(&mut before)?;
            node.state.lock().unwrap().pending_upload = Some(pending.clone());
            store
                .put(&snapshot.key, body, expected.metadata.clone())
                .await?
        } else {
            will_mutate(&mut before)?;
            let upload = store
                .begin_upload(&snapshot.key, expected.metadata.clone(), snapshot.size)
                .await?;
            let mut guard = UploadGuard {
                store: store.clone(),
                key: snapshot.key.to_string(),
                upload: upload.clone(),
                permit: flusher.take(),
                tasks: Some(tokio::task::JoinSet::new()),
                armed: true,
            };
            let tasks = guard.tasks.as_mut().expect("owned part tasks");
            let result: Result<String> = async {
                let mut parts = BTreeMap::new();
                let snapshot = Arc::new(snapshot.try_clone()?);
                let per_file = Arc::new(tokio::sync::Semaphore::new(config.max_parallel_parts));
                let mut offset = 0;
                let mut number = 0u32;
                let queued =
                    (config.max_parallel_parts + config.max_parallel_copy).min(config.max_requests);
                while offset < upload_size || !tasks.is_empty() {
                    while offset < upload_size && tasks.len() < queued {
                        let range = config
                            .part_sizes
                            .range(number)
                            .ok_or(Error::Fs(libc::EFBIG))?;
                        let end = range.end.min(upload_size);
                        let length = end - offset;
                        let dirty = snapshot.extents.range(..end).any(|(_, e)| e.end > offset);
                        let copy = !dirty && end <= snapshot.visible && snapshot.remote.is_some();
                        let at = offset;
                        let part_number = number as i32 + 1;
                        let queue = tokio::time::timeout(
                            config.http_timeout,
                            admission.queue.clone().acquire_owned(),
                        )
                        .await
                        .map_err(|_| Error::Timeout("part queue"))?
                        .map_err(|_| Error::Cancelled)?;
                        let (store, snapshot, upload, lane, admission, per_file, timeout) = (
                            store.clone(),
                            snapshot.clone(),
                            upload.clone(),
                            lane.clone(),
                            admission.clone(),
                            per_file.clone(),
                            config.http_timeout,
                        );
                        tasks.spawn(async move {
                            let _queue = queue;
                            let limits = async {
                                if copy {
                                    let _slot = admission
                                        .copies
                                        .clone()
                                        .acquire_owned()
                                        .await
                                        .map_err(|_| Error::Cancelled)?;
                                    let remote = snapshot.remote.as_ref().expect("copy baseline");
                                    store
                                        .copy_part(
                                            &snapshot.key,
                                            &upload,
                                            part_number,
                                            &snapshot.key,
                                            at..=end - 1,
                                            &remote.etag,
                                        )
                                        .await
                                } else {
                                    let _file = per_file
                                        .acquire_owned()
                                        .await
                                        .map_err(|_| Error::Cancelled)?;
                                    let _slot = admission
                                        .parts
                                        .acquire_owned()
                                        .await
                                        .map_err(|_| Error::Cancelled)?;
                                    let body = crate::upload_body::stream_snapshot(
                                        &store,
                                        snapshot.clone(),
                                        at,
                                        length,
                                        &lane,
                                    );
                                    store
                                        .upload_part(&snapshot.key, &upload, part_number, body)
                                        .await
                                }
                            };
                            tokio::time::timeout(timeout, limits)
                                .await
                                .map_err(|_| Error::Timeout("part admission/request/body"))?
                        });
                        offset = end;
                        number += 1;
                    }
                    match tasks.join_next().await.expect("part jobs") {
                        Ok(Ok(part)) => {
                            parts.insert(
                                part.part_number().ok_or_else(|| {
                                    Error::InvalidResponse("missing completed part number".into())
                                })?,
                                part,
                            );
                        }
                        other => {
                            return Err(match other {
                                Ok(Err(error)) => error,
                                _ => Error::Cancelled,
                            });
                        }
                    }
                }
                // Set before the request: cancellation can lose an applied
                // completion's response just as an explicit transport error can.
                node.state.lock().unwrap().pending_upload = Some(pending.clone());
                store
                    .complete_upload(&snapshot.key, &upload, parts.into_values().collect())
                    .await
            }
            .await;
            // Admission or malformed completion failures can leave sibling jobs
            // running too. Drain every exit before aborting the owned upload.
            tasks.abort_all();
            while tasks.join_next().await.is_some() {}
            let committed = match result {
                Ok(etag) => etag,
                Err(error) => {
                    // A completion error may be ambiguous. Abort only this owned upload;
                    // never clear local state, even if abort itself fails.
                    let _ = store.abort_upload(&snapshot.key, &upload).await;
                    guard.armed = false;
                    return Err(error);
                }
            };
            guard.armed = false;
            flusher = guard.permit.take();
            committed
        };
        // A successful commit already supplies its exact generation. Preserve it
        // before a fallible verification HEAD, without retiring dirty ownership.
        let remote = CachedRemote::from_admitted(
            ObjectHead {
                size: upload_size,
                etag: committed.clone(),
                ..Default::default()
            },
            pending.metadata.clone(),
            &mut pending.progress_credit.lock().unwrap(),
        )?;
        progress_version(node, snapshot.version, remote.clone());
        node.state.lock().unwrap().pending_upload = None;
        let head = store.head(&snapshot.key).await?;
        if head.size != upload_size || head.etag != committed {
            return Err(Error::InvalidResponse(
                "committed object size/generation differs from snapshot".into(),
            ));
        }
        let remote = project_head(
            head,
            Some(&remote),
            &pending.metadata,
            &mut pending.progress_credit.lock().unwrap(),
        )?;
        if !bootstrap {
            return retire(node, &snapshot, remote);
        }
        progress_version(node, snapshot.version, remote.clone());
        snapshot.visible = remote.size;
        snapshot.set_remote(remote, &mut pending.progress_credit.lock().unwrap())?;
        if let Some(head) = patch_snapshot(
            store,
            node,
            &mut snapshot,
            lane,
            config,
            upload_size,
            &mut before,
        )
        .await?
        {
            return retire(node, &snapshot, head);
        }
        // An explicit unsupported response permits a bounded full-upload fallback.
        bootstrap = false;
    }
}

fn retire(node: &Inode, snapshot: &Snapshot, remote: CachedRemote) -> Result<()> {
    let mut state = node.state.lock().unwrap();
    if state.deleted {
        return Err(Error::Fs(libc::ENOENT));
    }
    if state.version == snapshot.version {
        state.metadata = remote.metadata.clone();
    }
    state.remote = Some(remote);
    state.visible_remote_size = state
        .cuts
        .range(snapshot.version + 1..)
        .map(|(_, size)| *size)
        .min()
        .unwrap_or(snapshot.size)
        .min(snapshot.size);
    state.cuts.retain(|&version, _| version > snapshot.version);
    state.extents.retain(|_, e| e.version > snapshot.version);
    state.durable = snapshot.version;
    // A verified commit is a newer authoritative observation than a LIST that
    // was already in flight. Its metadata must not replace this generation.
    state.fetched = std::time::Instant::now();
    state.settle_metadata();
    Ok(())
}

fn patchable(snapshot: &Snapshot, config: &Config) -> bool {
    snapshot.remote.as_ref().is_some_and(|remote| {
        remote.size > 0
            && snapshot.visible >= remote.size
            && snapshot.size >= remote.size
            && (snapshot.size <= config.single_part.max(5) * 1024 * 1024
                || remote.etag.trim_matches('"').contains('-'))
    })
}

fn conflict(error: &Error) -> bool {
    matches!(error,Error::Fs(code) if matches!(*code,libc::ESTALE|libc::EBUSY))
        || matches!(error,Error::Backend{code,..} if matches!(code.as_str(),"PreconditionFailed"|"ConditionalRequestConflict"|"OperationAborted"|"Conflict"|"HTTP409"|"HTTP412"))
}

fn progress_version(node: &Inode, version: u64, remote: CachedRemote) {
    let mut state = node.state.lock().unwrap();
    state.visible_remote_size = state
        .cuts
        .range(version + 1..)
        .map(|(_, size)| *size)
        .min()
        .unwrap_or(remote.size)
        .min(remote.size);
    state.remote = Some(remote);
    state.settle_metadata();
}

async fn reconcile_upload(store: &Arc<dyn Store>, node: &Inode) -> Result<()> {
    let (pending, previous, budget) = {
        let state = node.state.lock().unwrap();
        let budget = state.metadata_budget().clone();
        (state.pending_upload.clone(), state.remote.clone(), budget)
    };
    let Some(pending) = pending else {
        return Ok(());
    };
    match store.head(&pending.key).await {
        Ok(head) => {
            if head.size == pending.size
                && head.metadata.get(UPLOAD_RECEIPT) == Some(&pending.receipt)
            {
                // Provenance permits using these bytes as the new conditional base,
                // NOT acknowledging durability. Re-upload all retained dirty data,
                // including newer writes and truncate cuts, before retiring anything.
                let remote = project_head(
                    head,
                    previous.as_ref(),
                    &pending.metadata,
                    &mut pending.progress_credit.lock().unwrap(),
                )?;
                progress_version(node, pending.version, remote);
            } else {
                let _comparison_charge = budget.charge_with(|| {
                    4 * (crate::metadata_budget::head_bytes(&head)
                        + previous.as_ref().map_or(0, CachedRemote::bytes))
                })?;
                if !unchanged_base(previous.as_ref(), &head) {
                    return Err(Error::Fs(libc::ESTALE));
                }
            }
        }
        Err(Error::Fs(libc::ENOENT)) if previous.is_none() => {}
        Err(Error::Backend { code, .. })
            if previous.is_none() && matches!(code.as_str(), "NoSuchKey" | "NotFound") => {}
        Err(error) => return Err(error),
    }
    node.state.lock().unwrap().pending_upload = None;
    Ok(())
}

fn unchanged_base(previous: Option<&CachedRemote>, head: &ObjectHead) -> bool {
    // Extended LIST exposes decoded public attributes, whereas HEAD includes the
    // private receipt too. Their public metadata must still compare identically.
    let public_metadata = |head: &ObjectHead| {
        crate::backend::metadata::decode_metadata(
            head.metadata
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str())),
        )
    };
    previous.is_some_and(|old| {
        let decoded = public_metadata(head);
        old.etag == head.etag
            && old.size == head.size
            && old
                .metadata
                .iter()
                .eq(decoded.iter().map(|(k, v)| (k.as_str(), v.as_slice())))
    })
}

async fn refresh_conflict(
    store: &Arc<dyn Store>,
    node: &Inode,
    snapshot: &Snapshot,
    start: u64,
    end: u64,
    discard: bool,
    credit: &mut crate::metadata_budget::MetadataCharge,
) -> Result<()> {
    let head = store.head(&snapshot.key).await?;
    let mut state = node.state.lock().unwrap();
    let incoming = state
        .metadata_budget()
        .estimate(|| Inode::metadata_estimate(&state.key, Some(&head)));
    let previous_bytes = state
        .metadata_budget()
        .estimate(|| snapshot.remote.as_ref().map_or(0, CachedRemote::bytes));
    let _temporary = state.metadata_budget().charge(
        4 * (incoming + previous_bytes)
            + state.extents.len() * std::mem::size_of::<(u64, Extent)>(),
    )?;
    state.admit_metadata(
        incoming + 2 * (std::mem::size_of::<(u64, Extent)>() + crate::metadata_budget::LINKS),
    )?;
    // Adopt remote changes only for metadata keys not modified locally. This
    // also preserves writes/xattrs newer than the failed snapshot.
    let previous = &snapshot.remote.as_ref().expect("PATCH baseline").metadata;
    let fresh = crate::backend::metadata::decode_metadata(
        head.metadata.iter().map(|(k, v)| (k.as_str(), v.as_str())),
    );
    let keys: std::collections::BTreeSet<_> = previous
        .keys()
        .chain(fresh.keys().map(String::as_str))
        .map(str::to_owned)
        .collect();
    let mut metadata = state.metadata.clone();
    for key in keys {
        if state.metadata.get(&key) == previous.get(&key) {
            if let Some(value) = fresh.get(&key) {
                metadata.try_insert(&key, value)?;
            } else {
                metadata.try_remove(&key)?;
            }
        }
    }
    let remote = project_head(head.clone(), state.remote.as_ref(), &state.metadata, credit)?;
    let metadata = if metadata == remote.metadata {
        remote.metadata.clone()
    } else {
        metadata
    };
    let overlapping: Vec<_> = state
        .extents
        .range(..end)
        .filter(|(_, extent)| discard && extent.end > start && extent.version <= snapshot.version)
        .map(|(&offset, extent)| (offset, extent.clone()))
        .collect();
    for (offset, extent) in overlapping {
        state.extents.remove(&offset);
        if offset < start {
            state.extents.insert(
                offset,
                Extent {
                    end: start,
                    ..extent.clone()
                },
            );
        }
        if extent.end > end {
            state.extents.insert(
                end,
                Extent {
                    data_offset: extent.data_offset + (end - offset) as usize,
                    ..extent
                },
            );
        }
    }
    if discard && state.version == snapshot.version && state.extents.is_empty() {
        state.size = head.size;
    } else if state.cuts.range(snapshot.version + 1..).next().is_none() {
        state.size = state.size.max(head.size);
    }
    state.visible_remote_size = state
        .cuts
        .range(snapshot.version + 1..)
        .map(|(_, size)| *size)
        .min()
        .unwrap_or(head.size)
        .min(head.size);
    state.metadata = metadata;
    state.remote = Some(remote);
    state.settle_metadata();
    Ok(())
}

async fn patch_snapshot(
    store: &Arc<dyn Store>,
    node: &Inode,
    snapshot: &mut Snapshot,
    lane: &Budget,
    config: &Config,
    skip_before: u64,
    before: &mut Option<&mut BeforeMutation<'_>>,
) -> Result<Option<CachedRemote>> {
    let mut patch_metadata = snapshot.metadata_charge.budget().charge_with(|| {
        3 * (snapshot.metadata_bytes() + snapshot.metadata.encoding_bytes()) + 4096
    })?;
    let remote = snapshot.remote.as_ref().expect("PATCH eligibility").clone();
    // Dirty extents and a growing zero tail are the only requested mutations.
    // Keep each request within a fixed payload lane; never materialize an object.
    let mut ranges: Vec<(u64, u64)> = snapshot
        .extents
        .iter()
        .filter(|(_, e)| e.end > skip_before)
        .map(|(&start, e)| (start.max(skip_before), e.end))
        .collect();
    if snapshot.size > remote.size {
        ranges.push((remote.size, snapshot.size));
    }
    ranges.sort_unstable();
    let mut merged: Vec<(u64, u64)> = Vec::with_capacity(ranges.len());
    for (start, end) in ranges {
        if let Some(last) = merged.last_mut().filter(|last| start <= last.1) {
            last.1 = last.1.max(end);
        } else {
            merged.push((start, end));
        }
    }
    for (start, end) in merged {
        let mut offset = start;
        while offset < end {
            let (number, part) = config
                .part_sizes
                .at_offset(offset)
                .ok_or(Error::Fs(libc::EFBIG))?;
            let part_size = if number > 0
                && offset == part.start
                && offset == snapshot.remote.as_ref().expect("PATCH baseline").size
            {
                let previous = config.part_sizes.range(number - 1).expect("previous part");
                previous.end - previous.start
            } else {
                part.end - part.start
            };
            let length = (end - offset)
                .min(part.end - offset)
                .min(crate::upload_body::CHUNK_SIZE as u64) as usize;
            let remote = snapshot.remote.as_ref().expect("PATCH baseline");
            let mut generation_credit = snapshot
                .metadata_charge
                .budget()
                .charge_with(|| remote.bytes() + 1024)?;
            let current = remote.clone();
            let result = async {
                let body =
                    crate::upload_body::admitted(store, snapshot, offset, length, lane).await?;
                will_mutate(before)?;
                store
                    .patch(&snapshot.key, offset, body, &current.etag, Some(part_size))
                    .await
            }
            .await;
            let result = match result {
                Ok(result) => result,
                Err(Error::Fs(code)) if code == libc::ENOSYS => return Ok(None),
                Err(error) => {
                    if conflict(&error) {
                        refresh_conflict(
                            store,
                            node,
                            snapshot,
                            offset,
                            offset + length as u64,
                            config.drop_patch_conflicts,
                            &mut patch_metadata,
                        )
                        .await?;
                        return Err(Error::Fs(libc::EBUSY));
                    }
                    return Err(error);
                }
            };
            if result.etag.is_empty() {
                return Err(Error::InvalidResponse(
                    "PATCH missing committed generation".into(),
                ));
            }
            let remote = current.with_generation_from(
                current.size.max(offset + length as u64),
                result.etag,
                &mut generation_credit,
            )?;
            {
                let mut state = node.state.lock().unwrap();
                state.visible_remote_size = state
                    .cuts
                    .range(snapshot.version + 1..)
                    .map(|(_, size)| *size)
                    .min()
                    .unwrap_or(remote.size)
                    .min(remote.size);
                state.remote = Some(remote.clone());
                state.settle_metadata();
            }
            snapshot.visible = remote.size;
            snapshot.set_remote(remote, &mut generation_credit)?;
            offset += length as u64;
        }
    }
    let current = snapshot.remote.as_ref().expect("PATCH baseline");
    let metadata_changed = current.metadata != snapshot.metadata;
    let result = if metadata_changed {
        will_mutate(before)?;
        store
            .replace_metadata(
                &snapshot.key,
                &current.etag,
                snapshot.metadata.encode()?.into_iter().collect(),
            )
            .await
    } else {
        store.head(&snapshot.key).await
    };
    let head = match result {
        Ok(head) => head,
        Err(error) => {
            if conflict(&error) {
                refresh_conflict(store, node, snapshot, 0, 0, false, &mut patch_metadata).await?;
            }
            return Err(error);
        }
    };
    if head.size != snapshot.size || (!metadata_changed && head.etag != current.etag) {
        refresh_conflict(store, node, snapshot, 0, 0, false, &mut patch_metadata).await?;
        return Err(Error::Fs(libc::ESTALE));
    }
    let remote = project_head(
        head,
        snapshot.remote.as_ref(),
        &snapshot.metadata,
        &mut patch_metadata,
    )?;
    Ok(Some(remote))
}

/// Cancellation cleanup keeps its admission token until abort completes, so
/// cancelled uploads cannot create an unbounded detached cleanup queue.
struct UploadGuard {
    store: Arc<dyn Store>,
    key: String,
    upload: String,
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
    tasks: Option<tokio::task::JoinSet<Result<aws_sdk_s3::types::CompletedPart>>>,
    armed: bool,
}

impl Drop for UploadGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let store = self.store.clone();
            let key = self.key.clone();
            let upload = self.upload.clone();
            let permit = self.permit.take();
            let mut tasks = self.tasks.take().expect("owned part tasks");
            tasks.abort_all();
            runtime.spawn(async move {
                let _permit = permit;
                while tasks.join_next().await.is_some() {}
                let _ = store.abort_upload(&key, &upload).await;
            });
        }
    }
}

/// A fully visible base range with no dirty overlay can keep the backend's one
/// charged allocation, including its full Range/body/checksum validation.
pub(crate) fn remote_only(snapshot: &Snapshot, offset: u64, length: usize) -> bool {
    snapshot.remote.as_ref().is_some_and(|remote| {
        (length as u64)
            <= snapshot
                .size
                .min(snapshot.visible)
                .min(remote.size)
                .saturating_sub(offset)
            && !snapshot
                .extents
                .range(..offset + length as u64)
                .any(|(_, extent)| extent.end > offset)
    })
}

pub async fn materialize(
    store: &Arc<dyn Store>,
    snapshot: &Snapshot,
    offset: u64,
    length: usize,
    budget: &Budget,
) -> Result<Block> {
    if remote_only(snapshot, offset, length) {
        return store
            .read_range(
                &snapshot.key,
                offset,
                length,
                &snapshot.remote.as_ref().unwrap().read_head(),
                budget,
            )
            .await;
    }
    let mut data = budget.reserve(length).await?.zeroed();
    if let Some(remote) = &snapshot.remote {
        let available = snapshot
            .visible
            .min(remote.size)
            .saturating_sub(offset)
            .min(length as u64) as usize;
        let mut covered = offset;
        for (&start, extent) in snapshot.extents.range(..offset + available as u64) {
            if start > covered {
                break;
            }
            if extent.end > covered {
                covered = extent.end;
            }
        }
        if available > 0 && covered < offset + available as u64 {
            let old = store
                .read_range(
                    &snapshot.key,
                    offset,
                    available,
                    &remote.read_head(),
                    budget,
                )
                .await?;
            data.as_mut_slice()[..available].copy_from_slice(old.as_slice());
        }
    }
    overlay(snapshot, offset, data.as_mut_slice());
    Ok(data)
}
