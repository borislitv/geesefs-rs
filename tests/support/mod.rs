#![allow(dead_code)]
use aws_sdk_s3::{primitives::ByteStream, types::CompletedPart};
use clap::Parser;
use geesefs_rs::{
    backend::{
        Store,
        extended_list::{ListPage, ListedObject},
        metadata::decode_metadata,
        s3::ObjectHead,
    },
    budget::{Block, Budget},
    config::Config,
    error::{Error, Result},
    filesystem::Filesystem,
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};
use tokio::sync::oneshot;

#[derive(Clone)]
pub struct Object {
    pub bytes: Vec<u8>,
    pub metadata: HashMap<String, String>,
    pub etag: String,
}
struct Upload {
    key: String,
    metadata: HashMap<String, String>,
    parts: BTreeMap<i32, Vec<u8>>,
}
type Pause = Option<(oneshot::Sender<()>, oneshot::Receiver<()>)>;
struct ActiveHead<'a>(&'a AtomicUsize);
impl Drop for ActiveHead<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
#[derive(Default)]
pub struct MemoryStore {
    pub objects: Mutex<BTreeMap<String, Object>>,
    uploads: Mutex<HashMap<String, Upload>>,
    sequence: AtomicU64,
    pub fail_put: AtomicBool,
    pub overwrite_after_put: AtomicBool,
    pub patch_calls: AtomicUsize,
    pub patch_ranges: Mutex<Vec<(u64, usize)>>,
    pub fail_patch_after: AtomicUsize,
    pub patch_unsupported: AtomicBool,
    pub patch_lost_response: AtomicBool,
    pub upload_sizes: Mutex<Vec<u64>>,
    pub put_calls: AtomicUsize,
    pub upload_part_sizes: Mutex<Vec<(i32, u64)>>,
    pub max_upload_frame: AtomicUsize,
    pub virtual_size: AtomicU64,
    pub copied_ranges: Mutex<Vec<std::ops::RangeInclusive<u64>>>,
    pub part_starts: AtomicUsize,
    pub part_gate: Mutex<Option<Arc<tokio::sync::Semaphore>>>,
    pub part_started: tokio::sync::Notify,
    pub completed_parts: AtomicUsize,
    pub part_completed: tokio::sync::Notify,
    pub read_gate: Mutex<Option<Arc<tokio::sync::Semaphore>>>,
    pub read_started: tokio::sync::Notify,
    pub head_keys: Mutex<Vec<String>>,
    pub head_gate: Mutex<Option<Arc<tokio::sync::Semaphore>>>,
    pub head_started: tokio::sync::Notify,
    pub head_starts: AtomicUsize,
    pub active_heads: AtomicUsize,
    pub peak_heads: AtomicUsize,
    pub list_calls: AtomicUsize,
    patch_pause: Mutex<Pause>,
    head_pause: Mutex<Pause>,
    head_response_pause: Mutex<Pause>,
    delete_pause: Mutex<Pause>,
    delete_response_pause: Mutex<Pause>,
    pub delete_calls: AtomicUsize,
    pub fail_head_after_put_once: AtomicBool,
    pub fail_head_after_complete_once: AtomicBool,
    pub standard_listing: AtomicBool,
    pub last_modified: Mutex<Option<std::time::SystemTime>>,
    pub fail_head_once: AtomicBool,
    pub fail_copy: AtomicBool,
    pub fail_complete: AtomicBool,
    pub fail_delete: AtomicBool,
    pub retain_deleted: AtomicBool,
    pub delete_applied_error_once: AtomicBool,
    pub page_size: AtomicUsize,
    pub copied_parts: AtomicUsize,
    pub read_calls: AtomicUsize,
    pub read_ranges: Mutex<Vec<(u64, usize)>>,
    pub read_materialization_peak: AtomicUsize,
    pub copy_calls: AtomicUsize,
    pub fail_copy_after: AtomicUsize,
    pause: Mutex<Pause>,
    list_pause: Mutex<Pause>,
    part_pause: Mutex<Pause>,
}

impl MemoryStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            page_size: AtomicUsize::new(1000),
            fail_copy_after: AtomicUsize::new(usize::MAX),
            fail_patch_after: AtomicUsize::new(usize::MAX),
            ..Self::default()
        })
    }
    pub fn config(memory: usize) -> Config {
        Config::try_parse_from([
            "geesefs-rs",
            "-f",
            "--endpoint=http://fixture.invalid",
            "--enable-perms",
            "--enable-mtime",
            &format!("--memory-limit={memory}"),
            "bucket",
            "/tmp/mount",
        ])
        .unwrap()
    }
    pub fn filesystem(self: &Arc<Self>, memory: usize) -> Arc<Filesystem> {
        Filesystem::new(Self::config(memory), self.clone()).unwrap()
    }
    pub fn pause_put(&self) -> (oneshot::Receiver<()>, oneshot::Sender<()>) {
        let (entered, seen) = oneshot::channel();
        let (release, wait) = oneshot::channel();
        *self.pause.lock().unwrap() = Some((entered, wait));
        (seen, release)
    }
    pub fn pause_patch_response(&self) -> (oneshot::Receiver<()>, oneshot::Sender<()>) {
        let (entered, seen) = oneshot::channel();
        let (release, wait) = oneshot::channel();
        *self.patch_pause.lock().unwrap() = Some((entered, wait));
        (seen, release)
    }
    pub fn pause_head(&self) -> (oneshot::Receiver<()>, oneshot::Sender<()>) {
        let (entered, seen) = oneshot::channel();
        let (release, wait) = oneshot::channel();
        *self.head_pause.lock().unwrap() = Some((entered, wait));
        (seen, release)
    }
    pub fn pause_head_response(&self) -> (oneshot::Receiver<()>, oneshot::Sender<()>) {
        let (entered, seen) = oneshot::channel();
        let (release, wait) = oneshot::channel();
        *self.head_response_pause.lock().unwrap() = Some((entered, wait));
        (seen, release)
    }
    pub fn pause_delete(&self, after: bool) -> (oneshot::Receiver<()>, oneshot::Sender<()>) {
        let (entered, seen) = oneshot::channel();
        let (release, wait) = oneshot::channel();
        *if after {
            &self.delete_response_pause
        } else {
            &self.delete_pause
        }
        .lock()
        .unwrap() = Some((entered, wait));
        (seen, release)
    }
    pub fn pause_list(&self) -> (oneshot::Receiver<()>, oneshot::Sender<()>) {
        let (entered, seen) = oneshot::channel();
        let (release, wait) = oneshot::channel();
        *self.list_pause.lock().unwrap() = Some((entered, wait));
        (seen, release)
    }
    async fn barrier(pause: &Mutex<Pause>) {
        let pause = pause.lock().unwrap().take();
        if let Some((entered, wait)) = pause {
            let _ = entered.send(());
            let _ = wait.await;
        }
    }
    pub fn pause_part(&self) -> (oneshot::Receiver<()>, oneshot::Sender<()>) {
        let (entered, seen) = oneshot::channel();
        let (release, wait) = oneshot::channel();
        *self.part_pause.lock().unwrap() = Some((entered, wait));
        (seen, release)
    }
    pub fn pending_uploads(&self) -> usize {
        self.uploads.lock().unwrap().len()
    }
    fn etag(&self) -> String {
        format!(
            "\"generation-{}\"",
            self.sequence.fetch_add(1, Ordering::Relaxed)
        )
    }
    fn failure(operation: &'static str) -> Error {
        Error::Backend {
            operation,
            code: "InjectedFailure".into(),
            message: "disposable deterministic fixture".into(),
        }
    }
    pub fn seed(&self, key: &str, bytes: &[u8]) {
        self.objects.lock().unwrap().insert(
            key.into(),
            Object {
                bytes: bytes.into(),
                metadata: HashMap::new(),
                etag: self.etag(),
            },
        );
    }
    pub fn bytes(&self, key: &str) -> Vec<u8> {
        self.objects.lock().unwrap()[key].bytes.clone()
    }
}

#[async_trait::async_trait]
impl Store for MemoryStore {
    async fn patch(
        &self,
        key: &str,
        offset: u64,
        body: Block,
        etag: &str,
        _append_part_size: Option<u64>,
    ) -> Result<geesefs_rs::backend::s3::PatchResult> {
        self.patch_ranges
            .lock()
            .unwrap()
            .push((offset, body.as_slice().len()));
        let call = self.patch_calls.fetch_add(1, Ordering::Relaxed);
        if self.patch_unsupported.load(Ordering::Relaxed) {
            return Err(Error::Fs(libc::ENOSYS));
        }
        if call >= self.fail_patch_after.load(Ordering::Relaxed) {
            return Err(Self::failure("PATCH"));
        }
        let etag = {
            let mut objects = self.objects.lock().unwrap();
            let object = objects.get_mut(key).ok_or(Error::Fs(libc::ENOENT))?;
            if object.etag != etag {
                return Err(Error::Backend {
                    operation: "PATCH",
                    code: "PreconditionFailed".into(),
                    message: "conditional generation mismatch".into(),
                });
            }
            if offset > object.bytes.len() as u64 {
                return Err(Error::Fs(libc::ERANGE));
            }
            let end = offset as usize + body.as_slice().len();
            object.bytes.resize(object.bytes.len().max(end), 0);
            object.bytes[offset as usize..end].copy_from_slice(body.as_slice());
            object.etag = self.etag();
            object.etag.clone()
        };
        Self::barrier(&self.patch_pause).await;
        if self.patch_lost_response.swap(false, Ordering::Relaxed) {
            return Err(Error::Timeout("injected lost PATCH response"));
        }
        Ok(geesefs_rs::backend::s3::PatchResult {
            etag,
            last_modified: aws_sdk_s3::primitives::DateTime::from_secs(1767225600),
        })
    }
    async fn head(&self, key: &str) -> Result<ObjectHead> {
        self.head_keys.lock().unwrap().push(key.into());
        let active = self.active_heads.fetch_add(1, Ordering::SeqCst) + 1;
        let _active = ActiveHead(&self.active_heads);
        self.peak_heads.fetch_max(active, Ordering::SeqCst);
        self.head_starts.fetch_add(1, Ordering::SeqCst);
        self.head_started.notify_one();
        Self::barrier(&self.head_pause).await;
        let gate = self.head_gate.lock().unwrap().clone();
        if let Some(gate) = gate {
            gate.acquire().await.map_err(|_| Error::Cancelled)?.forget();
        }
        if self.fail_head_once.swap(false, Ordering::Relaxed) {
            return Err(Self::failure("HEAD"));
        }
        let result = {
            let objects = self.objects.lock().unwrap();
            let object = objects.get(key).ok_or(Error::Fs(libc::ENOENT))?;
            Ok(ObjectHead {
                size: if key == "virtual" && self.virtual_size.load(Ordering::Relaxed) > 0 {
                    self.virtual_size.load(Ordering::Relaxed)
                } else {
                    object.bytes.len() as u64
                },
                etag: object.etag.clone(),
                metadata: object.metadata.clone(),
                last_modified: *self.last_modified.lock().unwrap(),
                ..Default::default()
            })
        };
        Self::barrier(&self.head_response_pause).await;
        result
    }
    async fn read_range(
        &self,
        key: &str,
        offset: u64,
        length: usize,
        head: &ObjectHead,
        budget: &Budget,
    ) -> Result<Block> {
        self.read_calls.fetch_add(1, Ordering::Relaxed);
        self.read_ranges.lock().unwrap().push((offset, length));
        self.read_materialization_peak
            .fetch_max(budget.used() + length, Ordering::Relaxed);
        self.read_started.notify_one();
        let gate = self.read_gate.lock().unwrap().clone();
        if let Some(gate) = gate {
            gate.acquire().await.map_err(|_| Error::Cancelled)?.forget();
        }
        let reservation = budget.reserve(length).await?;
        let objects = self.objects.lock().unwrap();
        let object = objects.get(key).ok_or(Error::Fs(libc::ENOENT))?;
        if object.etag != head.etag {
            return Err(Error::Fs(libc::ESTALE));
        }
        if key == "virtual" && self.virtual_size.load(Ordering::Relaxed) > 0 {
            let mut data = vec![0; length];
            if offset < object.bytes.len() as u64 {
                let amount = length.min(object.bytes.len() - offset as usize);
                data[..amount]
                    .copy_from_slice(&object.bytes[offset as usize..offset as usize + amount]);
            }
            return reservation.copy(&data);
        }
        let bytes = object
            .bytes
            .get(offset as usize..offset as usize + length)
            .ok_or(Error::Fs(libc::EIO))?;
        reservation.copy(bytes)
    }
    async fn put(
        &self,
        key: &str,
        mut body: ByteStream,
        metadata: HashMap<String, String>,
    ) -> Result<String> {
        self.put_calls.fetch_add(1, Ordering::Relaxed);
        let mut bytes = Vec::new();
        while let Some(frame) = body
            .try_next()
            .await
            .map_err(|_| Self::failure("PUT body"))?
        {
            self.max_upload_frame
                .fetch_max(frame.len(), Ordering::Relaxed);
            bytes.extend_from_slice(&frame);
        }
        Self::barrier(&self.pause).await;
        if self.fail_put.load(Ordering::Relaxed) {
            return Err(Self::failure("PUT"));
        }
        let etag = self.etag();
        self.objects.lock().unwrap().insert(
            key.into(),
            Object {
                bytes,
                metadata,
                etag: etag.clone(),
            },
        );
        if self.overwrite_after_put.swap(false, Ordering::Relaxed) {
            let mut objects = self.objects.lock().unwrap();
            let object = objects.get_mut(key).unwrap();
            object.etag = self.etag();
            object.bytes.fill(b'!');
        }
        if self.fail_head_after_put_once.swap(false, Ordering::Relaxed) {
            self.fail_head_once.store(true, Ordering::Relaxed);
        }
        Ok(etag)
    }
    async fn begin_upload(
        &self,
        key: &str,
        metadata: HashMap<String, String>,
        size: u64,
    ) -> Result<String> {
        self.upload_sizes.lock().unwrap().push(size);
        let upload = self.etag();
        self.uploads.lock().unwrap().insert(
            upload.clone(),
            Upload {
                key: key.into(),
                metadata,
                parts: BTreeMap::new(),
            },
        );
        Ok(upload)
    }
    async fn upload_part(
        &self,
        _: &str,
        upload: &str,
        part: i32,
        mut body: ByteStream,
    ) -> Result<CompletedPart> {
        self.part_starts.fetch_add(1, Ordering::Relaxed);
        self.part_started.notify_one();
        let gate = self.part_gate.lock().unwrap().clone();
        if let Some(gate) = gate {
            gate.acquire().await.map_err(|_| Error::Cancelled)?.forget();
        }
        let mut data = Vec::new();
        while let Some(frame) = body
            .try_next()
            .await
            .map_err(|_| Self::failure("part body"))?
        {
            self.max_upload_frame
                .fetch_max(frame.len(), Ordering::Relaxed);
            data.extend_from_slice(&frame);
        }
        self.upload_part_sizes
            .lock()
            .unwrap()
            .push((part, data.len() as u64));
        Self::barrier(&self.part_pause).await;
        self.uploads
            .lock()
            .unwrap()
            .get_mut(upload)
            .ok_or(Error::Fs(libc::ENOENT))?
            .parts
            .insert(part, data);
        self.completed_parts.fetch_add(1, Ordering::Relaxed);
        self.part_completed.notify_one();
        Ok(CompletedPart::builder()
            .part_number(part)
            .e_tag(format!("\"part-{part}\""))
            .build())
    }
    async fn copy_part(
        &self,
        _: &str,
        upload: &str,
        part: i32,
        source: &str,
        range: std::ops::RangeInclusive<u64>,
        etag: &str,
    ) -> Result<CompletedPart> {
        if self.fail_copy.load(Ordering::Relaxed) {
            return Err(Self::failure("copy part"));
        }
        let bytes = {
            let objects = self.objects.lock().unwrap();
            let source = objects.get(source).ok_or(Error::Fs(libc::ENOENT))?;
            if source.etag != etag {
                return Err(Error::Fs(libc::ESTALE));
            }
            self.copied_ranges.lock().unwrap().push(range.clone());
            if self.virtual_size.load(Ordering::Relaxed) > 0 {
                Vec::new()
            } else {
                source.bytes[*range.start() as usize..=*range.end() as usize].to_vec()
            }
        };
        self.uploads
            .lock()
            .unwrap()
            .get_mut(upload)
            .ok_or(Error::Fs(libc::ENOENT))?
            .parts
            .insert(part, bytes);
        self.copied_parts.fetch_add(1, Ordering::Relaxed);
        Ok(CompletedPart::builder()
            .part_number(part)
            .e_tag(format!("\"part-{part}\""))
            .build())
    }
    async fn complete_upload(
        &self,
        key: &str,
        upload: &str,
        parts: Vec<CompletedPart>,
    ) -> Result<String> {
        if self.fail_complete.load(Ordering::Relaxed) {
            return Err(Self::failure("complete"));
        }
        let upload = self.uploads.lock().unwrap().remove(upload).unwrap();
        assert_eq!(upload.key, key);
        let mut bytes = Vec::new();
        for part in parts {
            bytes.extend_from_slice(&upload.parts[&part.part_number().unwrap()]);
        }
        let etag = self.etag();
        self.objects.lock().unwrap().insert(
            key.into(),
            Object {
                bytes,
                metadata: upload.metadata,
                etag: etag.clone(),
            },
        );
        if self
            .fail_head_after_complete_once
            .swap(false, Ordering::Relaxed)
        {
            self.fail_head_once.store(true, Ordering::Relaxed);
        }
        Ok(etag)
    }
    async fn replace_metadata(
        &self,
        key: &str,
        etag: &str,
        metadata: HashMap<String, String>,
    ) -> Result<ObjectHead> {
        {
            let mut objects = self.objects.lock().unwrap();
            let object = objects.get_mut(key).ok_or(Error::Fs(libc::ENOENT))?;
            if object.etag != etag {
                return Err(Error::Fs(libc::ESTALE));
            }
            object.metadata = metadata;
            object.etag = self.etag();
        }
        self.head(key).await
    }
    async fn abort_upload(&self, _: &str, upload: &str) -> Result<()> {
        self.uploads.lock().unwrap().remove(upload);
        Ok(())
    }
    async fn copy(&self, source: &str, destination: &str, etag: &str) -> Result<String> {
        if self.fail_copy.load(Ordering::Relaxed)
            || self.copy_calls.fetch_add(1, Ordering::Relaxed)
                >= self.fail_copy_after.load(Ordering::Relaxed)
        {
            return Err(Self::failure("copy"));
        }
        let mut objects = self.objects.lock().unwrap();
        let mut object = objects.get(source).ok_or(Error::Fs(libc::ENOENT))?.clone();
        if object.etag != etag {
            return Err(Error::Fs(libc::ESTALE));
        }
        object.etag = self.etag();
        let etag = object.etag.clone();
        objects.insert(destination.into(), object);
        Ok(etag)
    }
    async fn delete(&self, key: &str) -> Result<()> {
        self.delete_calls.fetch_add(1, Ordering::Relaxed);
        Self::barrier(&self.delete_pause).await;
        if self.fail_delete.load(Ordering::Relaxed) {
            return Err(Error::Backend {
                operation: "delete",
                code: "AccessDenied".into(),
                message: "injected rejected DELETE".into(),
            });
        }
        if !self.retain_deleted.load(Ordering::Relaxed) {
            self.objects.lock().unwrap().remove(key);
        }
        Self::barrier(&self.delete_response_pause).await;
        if self
            .delete_applied_error_once
            .swap(false, Ordering::Relaxed)
        {
            return Err(Error::Timeout("injected applied DELETE response loss"));
        }
        Ok(())
    }
    async fn list(
        &self,
        prefix: &str,
        delimiter: Option<&str>,
        token: Option<&str>,
    ) -> Result<ListPage> {
        self.list_calls.fetch_add(1, Ordering::Relaxed);
        let page = {
            let objects = self.objects.lock().unwrap();
            let mut prefixes = BTreeSet::new();
            let mut items = BTreeMap::new();
            for (key, object) in objects.range(prefix.to_owned()..) {
                let Some(relative) = key.strip_prefix(prefix) else {
                    break;
                };
                if delimiter == Some("/")
                    && let Some(slash) = relative.find('/')
                {
                    let directory = format!("{prefix}{}", &relative[..=slash]);
                    prefixes.insert(directory);
                    continue;
                }
                items.insert(key.clone(), object.clone());
            }
            let all: BTreeSet<_> = items
                .keys()
                .cloned()
                .chain(prefixes.iter().cloned())
                .filter(|k| token.is_none_or(|t| k.as_str() > t))
                .collect();
            let limit = self.page_size.load(Ordering::Relaxed).max(1);
            let keys: Vec<_> = all.iter().take(limit).cloned().collect();
            let next_token = (all.len() > keys.len()).then(|| keys.last().unwrap().clone());
            let contents = keys
                .iter()
                .filter_map(|key| {
                    items.get(key).map(|o| ListedObject {
                        key: key.clone(),
                        size: o.bytes.len() as u64,
                        etag: Some(o.etag.clone()),
                        last_modified: self
                            .last_modified
                            .lock()
                            .unwrap()
                            .map(aws_sdk_s3::primitives::DateTime::from),
                        metadata: (!self.standard_listing.load(Ordering::Relaxed)).then(|| {
                            decode_metadata(
                                o.metadata.iter().map(|(k, v)| (k.as_str(), v.as_str())),
                            )
                        }),
                    })
                })
                .collect();
            let common_prefixes = keys.into_iter().filter(|k| prefixes.contains(k)).collect();
            ListPage {
                objects: contents,
                common_prefixes,
                next_token,
            }
        };
        Self::barrier(&self.list_pause).await;
        Ok(page)
    }
}
