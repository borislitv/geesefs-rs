use crate::{
    backend::{Store, metadata::encode_metadata, s3::ObjectHead},
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
    Ok(())
}

pub fn truncate(state: &mut State, size: u64) -> Result<()> {
    if state.deleted {
        return Err(Error::Fs(libc::ENOENT));
    }
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
    Ok(())
}

#[derive(Clone)]
pub struct Snapshot {
    pub key: String,
    pub size: u64,
    pub version: u64,
    pub remote: Option<ObjectHead>,
    pub visible: u64,
    pub extents: BTreeMap<u64, Extent>,
    pub metadata: HashMap<String, String>,
}

pub fn snapshot(node: &Inode) -> Result<Option<Snapshot>> {
    let state = node.state.lock().unwrap();
    if state.deleted {
        return Err(Error::Fs(libc::ENOENT));
    }
    if state.version == state.durable {
        return Ok(None);
    }
    Ok(Some(Snapshot {
        key: if state.mode & libc::S_IFMT == libc::S_IFDIR && !state.key.is_empty() {
            format!("{}/", state.data_key.as_ref().unwrap_or(&state.key))
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
        metadata: encode_metadata(&state.metadata)?.into_iter().collect(),
    }))
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
    let mut flusher = Some(flusher);
    let Some(mut snapshot) = snapshot(node)? else {
        return Ok(());
    };
    if snapshot.size > config.part_sizes.max_size() {
        return Err(Error::Fs(libc::EFBIG));
    }
    if config.enable_patch
        && patchable(&snapshot, config)
        && let Some(head) = patch_snapshot(store, node, &mut snapshot, lane, config, 0).await?
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
        let upload_size = if bootstrap { prefix } else { snapshot.size };
        let committed = if !bootstrap && upload_size <= single_limit {
            let _part =
                tokio::time::timeout(config.http_timeout, admission.parts.clone().acquire_owned())
                    .await
                    .map_err(|_| Error::Timeout("PUT admission"))?
                    .map_err(|_| Error::Cancelled)?;
            let body = crate::upload_body::stream(store, &snapshot, 0, snapshot.size, lane);
            store
                .put(&snapshot.key, body, snapshot.metadata.clone())
                .await?
        } else {
            let upload = store
                .begin_upload(&snapshot.key, snapshot.metadata.clone(), snapshot.size)
                .await?;
            let mut guard = UploadGuard {
                store: store.clone(),
                key: snapshot.key.clone(),
                upload: upload.clone(),
                permit: flusher.take(),
                tasks: Some(tokio::task::JoinSet::new()),
                armed: true,
            };
            let tasks = guard.tasks.as_mut().expect("owned part tasks");
            let result: Result<String> = async {
                let mut parts = BTreeMap::new();
                let snapshot = Arc::new(snapshot.clone());
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
        progress(
            node,
            &snapshot,
            &ObjectHead {
                size: upload_size,
                etag: committed.clone(),
                metadata: snapshot.metadata.clone(),
                ..Default::default()
            },
        );
        let head = store.head(&snapshot.key).await?;
        if head.size != upload_size || head.etag != committed {
            return Err(Error::InvalidResponse(
                "committed object size/generation differs from snapshot".into(),
            ));
        }
        if !bootstrap {
            return retire(node, &snapshot, head);
        }
        progress(node, &snapshot, &head);
        snapshot.visible = head.size;
        snapshot.remote = Some(head);
        if let Some(head) =
            patch_snapshot(store, node, &mut snapshot, lane, config, upload_size).await?
        {
            return retire(node, &snapshot, head);
        }
        // An explicit unsupported response permits a bounded full-upload fallback.
        bootstrap = false;
    }
}

fn retire(node: &Inode, snapshot: &Snapshot, head: ObjectHead) -> Result<()> {
    let mut state = node.state.lock().unwrap();
    if state.deleted {
        return Err(Error::Fs(libc::ENOENT));
    }
    state.remote = Some(head);
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

fn progress(node: &Inode, snapshot: &Snapshot, head: &ObjectHead) {
    let mut state = node.state.lock().unwrap();
    state.remote = Some(head.clone());
    state.visible_remote_size = state
        .cuts
        .range(snapshot.version + 1..)
        .map(|(_, size)| *size)
        .min()
        .unwrap_or(head.size)
        .min(head.size);
}

async fn refresh_conflict(
    store: &Arc<dyn Store>,
    node: &Inode,
    snapshot: &Snapshot,
    start: u64,
    end: u64,
    discard: bool,
) -> Result<()> {
    let head = store.head(&snapshot.key).await?;
    let mut state = node.state.lock().unwrap();
    // Adopt remote changes only for metadata keys not modified locally. This
    // also preserves writes/xattrs newer than the failed snapshot.
    let previous = crate::backend::metadata::decode_metadata(
        snapshot
            .remote
            .as_ref()
            .expect("PATCH baseline")
            .metadata
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str())),
    );
    let fresh = crate::backend::metadata::decode_metadata(
        head.metadata.iter().map(|(k, v)| (k.as_str(), v.as_str())),
    );
    let keys: std::collections::BTreeSet<_> =
        previous.keys().chain(fresh.keys()).cloned().collect();
    for key in keys {
        if state.metadata.get(&key) == previous.get(&key) {
            if let Some(value) = fresh.get(&key) {
                state.metadata.insert(key, value.clone());
            } else {
                state.metadata.remove(&key);
            }
        }
    }
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
    state.remote = Some(head);
    Ok(())
}

async fn patch_snapshot(
    store: &Arc<dyn Store>,
    node: &Inode,
    snapshot: &mut Snapshot,
    lane: &Budget,
    config: &Config,
    skip_before: u64,
) -> Result<Option<ObjectHead>> {
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
            let current = snapshot.remote.as_ref().expect("PATCH baseline").clone();
            let result = async {
                let body =
                    crate::upload_body::admitted(store, snapshot, offset, length, lane).await?;
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
            let head = ObjectHead {
                etag: result.etag,
                size: current.size.max(offset + length as u64),
                metadata: current.metadata,
                last_modified: std::time::SystemTime::try_from(result.last_modified).ok(),
                properties: current.properties,
            };
            progress(node, snapshot, &head);
            snapshot.visible = head.size;
            snapshot.remote = Some(head);
            offset += length as u64;
        }
    }
    let current = snapshot.remote.as_ref().expect("PATCH baseline");
    let metadata_changed = current.metadata != snapshot.metadata;
    let result = if metadata_changed {
        store
            .replace_metadata(&snapshot.key, &current.etag, snapshot.metadata.clone())
            .await
    } else {
        store.head(&snapshot.key).await
    };
    let head = match result {
        Ok(head) => head,
        Err(error) => {
            if conflict(&error) {
                refresh_conflict(store, node, snapshot, 0, 0, false).await?;
            }
            return Err(error);
        }
    };
    if head.size != snapshot.size || (!metadata_changed && head.etag != current.etag) {
        refresh_conflict(store, node, snapshot, 0, 0, false).await?;
        return Err(Error::Fs(libc::ESTALE));
    }
    Ok(Some(head))
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

pub async fn materialize(
    store: &Arc<dyn Store>,
    snapshot: &Snapshot,
    offset: u64,
    length: usize,
    budget: &Budget,
) -> Result<Block> {
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
                .read_range(&snapshot.key, offset, available, remote, budget)
                .await?;
            data.as_mut_slice()[..available].copy_from_slice(old.as_slice());
        }
    }
    overlay(snapshot, offset, data.as_mut_slice());
    Ok(data)
}
