#![forbid(unsafe_code)]
mod support;

use aws_sdk_s3::{primitives::ByteStream, types::CompletedPart};
use geesefs_rs::{
    backend::{Store, extended_list::ListPage, s3::ObjectHead},
    budget::{Block, Budget},
    error::{Error, Result},
    filesystem::Filesystem,
};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, UNIX_EPOCH},
};
use support::MemoryStore;
use tokio::sync::oneshot;

type Pause = (oneshot::Sender<()>, oneshot::Receiver<()>);

/// The backend applies the real body, then loses or withholds just its response.
/// Filesystem generations, upload bodies and conditional base reads are real.
struct AmbiguousCommit {
    inner: Arc<MemoryStore>,
    lose: AtomicBool,
    pause: Mutex<Option<Pause>>,
}

impl AmbiguousCommit {
    fn new(inner: Arc<MemoryStore>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            lose: AtomicBool::new(true),
            pause: Mutex::new(None),
        })
    }

    async fn response(&self, etag: String) -> Result<String> {
        let pause = self.pause.lock().unwrap().take();
        if let Some((entered, release)) = pause {
            let _ = entered.send(());
            let _ = release.await;
        }
        if self.lose.swap(false, Ordering::SeqCst) {
            return Err(Error::Timeout("commit applied, response lost"));
        }
        Ok(etag)
    }
}

#[async_trait::async_trait]
impl Store for AmbiguousCommit {
    async fn head(&self, key: &str) -> Result<ObjectHead> {
        self.inner.head(key).await
    }
    async fn read_range(
        &self,
        key: &str,
        offset: u64,
        length: usize,
        head: &ObjectHead,
        budget: &Budget,
    ) -> Result<Block> {
        self.inner
            .read_range(key, offset, length, head, budget)
            .await
    }
    async fn put(
        &self,
        key: &str,
        body: ByteStream,
        metadata: HashMap<String, String>,
    ) -> Result<String> {
        let etag = self.inner.put(key, body, metadata).await?;
        self.response(etag).await
    }
    async fn begin_upload(
        &self,
        key: &str,
        metadata: HashMap<String, String>,
        size: u64,
    ) -> Result<String> {
        self.inner.begin_upload(key, metadata, size).await
    }
    async fn replace_metadata(
        &self,
        key: &str,
        etag: &str,
        metadata: HashMap<String, String>,
    ) -> Result<ObjectHead> {
        self.inner.replace_metadata(key, etag, metadata).await
    }
    async fn upload_part(
        &self,
        key: &str,
        upload: &str,
        part: i32,
        body: ByteStream,
    ) -> Result<CompletedPart> {
        self.inner.upload_part(key, upload, part, body).await
    }
    async fn copy_part(
        &self,
        key: &str,
        upload: &str,
        part: i32,
        source: &str,
        range: std::ops::RangeInclusive<u64>,
        etag: &str,
    ) -> Result<CompletedPart> {
        self.inner
            .copy_part(key, upload, part, source, range, etag)
            .await
    }
    async fn complete_upload(
        &self,
        key: &str,
        upload: &str,
        parts: Vec<CompletedPart>,
    ) -> Result<String> {
        let etag = self.inner.complete_upload(key, upload, parts).await?;
        self.response(etag).await
    }
    async fn abort_upload(&self, key: &str, upload: &str) -> Result<()> {
        self.inner.abort_upload(key, upload).await
    }
    async fn copy(&self, source: &str, destination: &str, etag: &str) -> Result<String> {
        self.inner.copy(source, destination, etag).await
    }
    async fn delete(&self, key: &str) -> Result<()> {
        self.inner.delete(key).await
    }
    async fn list(
        &self,
        prefix: &str,
        delimiter: Option<&str>,
        token: Option<&str>,
    ) -> Result<ListPage> {
        self.inner.list(prefix, delimiter, token).await
    }
}

async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(10), future)
        .await
        .expect("writer regression deadline")
}

#[tokio::test]
async fn applied_put_with_lost_response_remains_retryable() {
    lost_response_is_retryable(6).await;
}

#[tokio::test]
async fn applied_completion_with_lost_response_remains_retryable() {
    lost_response_is_retryable(6 * 1024 * 1024).await;
}

async fn lost_response_is_retryable(size: usize) {
    bounded(async {
        let inner = MemoryStore::new();
        inner.seed("file", &vec![b'a'; size]);
        let store = AmbiguousCommit::new(inner.clone());
        let fs = Filesystem::new(MemoryStore::config(32), store).unwrap();
        let node = fs.lookup(1, "file").await.unwrap();
        fs.write(node.ino, 2, b"XY").await.unwrap();
        assert!(fs.fsync(node.ino).await.is_err());
        assert_eq!(&inner.bytes("file")[..6], b"aaXYaa");
        {
            let state = node.state.lock().unwrap();
            assert!(state.version > state.durable);
            assert!(!state.extents.is_empty());
        }
        fs.write(node.ino, 4, b"Z").await.unwrap();
        fs.fsync(node.ino)
            .await
            .expect("healthy retry after lost commit response");
        assert_eq!(&inner.bytes("file")[..6], b"aaXYZa");
        assert!(inner.bytes("file")[6..].iter().all(|byte| *byte == b'a'));
        assert_eq!(inner.pending_uploads(), 0);
    })
    .await;
}

#[tokio::test]
async fn cancelled_applied_commit_preserves_newer_writes_and_truncate_cuts() {
    bounded(async {
        for size in [6, 6 * 1024 * 1024] {
            let inner = MemoryStore::new();
            inner.seed("file", &vec![b'a'; size]);
            let store = AmbiguousCommit::new(inner.clone());
            store.lose.store(false, Ordering::SeqCst);
            let (entered, seen) = oneshot::channel();
            let (_release, wait) = oneshot::channel();
            *store.pause.lock().unwrap() = Some((entered, wait));
            let fs = Filesystem::new(MemoryStore::config(32), store).unwrap();
            let node = fs.lookup(1, "file").await.unwrap();
            fs.write(node.ino, 2, b"XY").await.unwrap();
            let flush = tokio::spawn({
                let fs = fs.clone();
                let ino = node.ino;
                async move { fs.fsync(ino).await }
            });
            seen.await.unwrap();
            fs.truncate(node.ino, 4).unwrap();
            fs.truncate(node.ino, 6).unwrap();
            fs.write(node.ino, 5, b"Z").await.unwrap();
            flush.abort();
            assert!(flush.await.unwrap_err().is_cancelled());
            {
                let state = node.state.lock().unwrap();
                assert!(state.durable < state.version);
            }
            fs.fsync(node.ino)
                .await
                .expect("retry cancelled applied commit");
            assert_eq!(inner.bytes("file"), b"aaXY\0Z");
            fs.shutdown().await.unwrap();
            assert_eq!(inner.pending_uploads(), 0);
        }
    })
    .await;
}

#[tokio::test]
async fn ambiguous_commit_does_not_adopt_an_unrelated_remote_generation() {
    bounded(async {
        for size in [6, 6 * 1024 * 1024] {
            let inner = MemoryStore::new();
            inner.seed("file", &vec![b'a'; size]);
            let store = AmbiguousCommit::new(inner.clone());
            let fs = Filesystem::new(MemoryStore::config(32), store).unwrap();
            let node = fs.lookup(1, "file").await.unwrap();
            fs.write(node.ino, 2, b"XY").await.unwrap();
            assert!(fs.fsync(node.ino).await.is_err());
            inner.seed("file", &vec![b'!'; size]);
            fs.write(node.ino, 4, b"Z").await.unwrap();
            assert!(fs.fsync(node.ino).await.is_err());
            let state = node.state.lock().unwrap();
            assert!(state.durable < state.version);
            assert!(!state.extents.is_empty());
            assert_eq!(inner.bytes("file"), vec![b'!'; size]);
        }
    })
    .await;
}

#[tokio::test]
async fn write_waiting_for_flusher_persists_the_snapshotted_mtime() {
    bounded(async {
        let store = MemoryStore::new();
        let mut config = MemoryStore::config(32);
        config.max_flushers = 1;
        let fs = Filesystem::new(config, store.clone()).unwrap();
        let first = fs
            .create(1, "first", libc::S_IFREG | 0o644, None, 0)
            .await
            .unwrap();
        let second = fs
            .create(1, "second", libc::S_IFREG | 0o644, None, 0)
            .await
            .unwrap();
        fs.write(first.ino, 0, b"hold").await.unwrap();
        fs.setattr(
            second.ino,
            None,
            None,
            None,
            Some(UNIX_EPOCH + Duration::from_secs(1234567890)),
        )
        .unwrap();
        let (entered, release) = store.pause_put();
        let flushing = tokio::spawn({
            let fs = fs.clone();
            let ino = first.ino;
            async move { fs.fsync(ino).await }
        });
        entered.await.unwrap();
        let mut waiting = Box::pin(fs.fsync(second.ino));
        std::future::poll_fn(|cx| {
            assert!(waiting.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        fs.write(second.ino, 0, b"new").await.unwrap();
        let expected = second
            .attr()
            .mtime
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        release.send(()).unwrap();
        flushing.await.unwrap().unwrap();
        waiting.await.unwrap();
        fs.fsync(second.ino).await.unwrap();
        assert_eq!(store.bytes("second"), b"new");
        let reopened = store.filesystem(32).lookup(1, "second").await.unwrap();
        assert_eq!(
            reopened.attr().mtime,
            UNIX_EPOCH + Duration::from_secs(expected)
        );
    })
    .await;
}

#[tokio::test]
async fn precommit_failure_keeps_the_original_base_retryable() {
    bounded(async {
        let store = MemoryStore::new();
        store.seed("file", b"abcdef");
        let fs = store.filesystem(32);
        let node = fs.lookup(1, "file").await.unwrap();
        fs.write(node.ino, 2, b"XY").await.unwrap();
        store.fail_put.store(true, Ordering::SeqCst);
        assert!(fs.fsync(node.ino).await.is_err());
        assert_eq!(store.bytes("file"), b"abcdef");
        store.fail_put.store(false, Ordering::SeqCst);
        fs.fsync(node.ino).await.unwrap();
        assert_eq!(store.bytes("file"), b"abXYef");
    })
    .await;
}

#[tokio::test]
async fn listed_objects_with_private_receipts_can_retry_a_precommit_failure() {
    bounded(async {
        let store = MemoryStore::new();
        store.seed("file", b"abcdef");
        let writer = store.filesystem(32);
        let node = writer.lookup(1, "file").await.unwrap();
        writer.write(node.ino, 2, b"XY").await.unwrap();
        writer.fsync(node.ino).await.unwrap();
        let listed = store.filesystem(32);
        listed.directory_chunk(1, "", 10).await.unwrap();
        let node = listed.lookup(1, "file").await.unwrap();
        listed.write(node.ino, 4, b"Z").await.unwrap();
        store.fail_put.store(true, Ordering::SeqCst);
        assert!(listed.fsync(node.ino).await.is_err());
        store.fail_put.store(false, Ordering::SeqCst);
        listed.fsync(node.ino).await.expect(
            "LIST stripping private metadata cannot turn an unchanged base into a conflict",
        );
        assert_eq!(store.bytes("file"), b"abXYZf");
    })
    .await;
}

#[tokio::test]
async fn unchanged_content_etag_does_not_hide_an_external_metadata_conflict() {
    bounded(async {
        let store = MemoryStore::new();
        store.seed("file", b"abcdef");
        let fs = store.filesystem(32);
        let node = fs.lookup(1, "file").await.unwrap();
        fs.write(node.ino, 2, b"XY").await.unwrap();
        store.fail_put.store(true, Ordering::SeqCst);
        assert!(fs.fsync(node.ino).await.is_err());
        store.fail_put.store(false, Ordering::SeqCst);
        store
            .objects
            .lock()
            .unwrap()
            .get_mut("file")
            .unwrap()
            .metadata
            .insert("external".into(), "changed".into());
        assert!(matches!(
            fs.fsync(node.ino).await,
            Err(Error::Fs(libc::ESTALE))
        ));
        assert_eq!(store.bytes("file"), b"abcdef");
        assert_eq!(
            store.objects.lock().unwrap()["file"].metadata["external"],
            "changed"
        );
    })
    .await;
}

#[tokio::test]
async fn recovered_baseline_is_not_durable_until_a_new_commit_finishes() {
    bounded(async {
        for size in [6, 6 * 1024 * 1024] {
            let inner = MemoryStore::new();
            inner.seed("file", &vec![b'a'; size]);
            let store = AmbiguousCommit::new(inner.clone());
            let fs = Filesystem::new(MemoryStore::config(32), store).unwrap();
            let node = fs.lookup(1, "file").await.unwrap();
            fs.write(node.ino, 2, b"XY").await.unwrap();
            assert!(fs.fsync(node.ino).await.is_err());
            let durable = node.state.lock().unwrap().durable;
            fs.write(node.ino, 4, b"Z").await.unwrap();
            inner.fail_head_once.store(true, Ordering::SeqCst);
            assert!(
                fs.fsync(node.ino).await.is_err(),
                "failed reconciliation HEAD must remain retryable"
            );
            let (entered, release) = if size == 6 {
                inner.pause_put()
            } else {
                inner.pause_part()
            };
            let retry = tokio::spawn({
                let fs = fs.clone();
                let ino = node.ino;
                async move { fs.fsync(ino).await }
            });
            entered.await.unwrap();
            {
                let state = node.state.lock().unwrap();
                assert_eq!(
                    state.durable, durable,
                    "receipt-bearing HEAD is not a durability acknowledgement"
                );
                assert_eq!(
                    state.extents.len(),
                    2,
                    "both old and new dirty writes remain owned"
                );
            }
            release.send(()).unwrap();
            retry.await.unwrap().unwrap();
            assert_eq!(&inner.bytes("file")[..6], b"aaXYZa");
        }
    })
    .await;
}

#[tokio::test]
async fn upload_receipts_are_private_and_a_matching_receipt_with_wrong_size_is_not_adopted() {
    bounded(async {
        let inner = MemoryStore::new();
        inner.seed("file", b"abcdef");
        let store = AmbiguousCommit::new(inner.clone());
        let fs = Filesystem::new(MemoryStore::config(32), store).unwrap();
        let node = fs.lookup(1, "file").await.unwrap();
        fs.write(node.ino, 2, b"XY").await.unwrap();
        assert!(fs.fsync(node.ino).await.is_err());
        assert_eq!(
            inner.objects.lock().unwrap()["file"].metadata["geesefs-upload-receipt"].len(),
            32
        );
        let attrs = fs.xattrs(node.ino).unwrap();
        assert!(!attrs.contains_key("user.geesefs-upload-receipt"));
        let fresh = inner.filesystem(32);
        let reopened = fresh.lookup(1, "file").await.unwrap();
        assert!(
            !fresh
                .xattrs(reopened.ino)
                .unwrap()
                .contains_key("user.geesefs-upload-receipt")
        );
        assert!(matches!(
            fs.setxattr(node.ino, "user.geesefs-upload-receipt", Some(b"forged"), 0),
            Err(Error::Fs(libc::EPERM))
        ));
        inner
            .objects
            .lock()
            .unwrap()
            .get_mut("file")
            .unwrap()
            .bytes
            .push(b'!');
        assert!(matches!(
            fs.fsync(node.ino).await,
            Err(Error::Fs(libc::ESTALE))
        ));
        let state = node.state.lock().unwrap();
        assert!(state.version > state.durable);
        assert_eq!(inner.bytes("file"), b"abXYef!");
    })
    .await;
}

#[tokio::test]
async fn generated_attribute_names_cannot_overwrite_private_provenance() {
    let store = MemoryStore::new();
    let mut config = MemoryStore::config(32);
    config.mtime_attr = "GEESEFS-UPLOAD-RECEIPT".into();
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let node = fs
        .create(1, "file", libc::S_IFREG | 0o644, None, 0)
        .await
        .unwrap();
    fs.write(node.ino, 0, b"new").await.unwrap();
    assert!(matches!(fs.fsync(node.ino).await, Err(Error::Config(_))));
    assert!(store.objects.lock().unwrap().is_empty());
}

#[derive(Clone)]
struct WireObject {
    bytes: Vec<u8>,
    metadata: HashMap<String, String>,
    etag: String,
}

/// Real SDK transport: the first commit applies, returns a retryable 503, and
/// subsequent GET/complete retries obey the now-changed generation/upload ID.
fn wire_backend(
    size: usize,
) -> (
    String,
    Arc<Mutex<WireObject>>,
    Arc<AtomicBool>,
    std::thread::JoinHandle<()>,
) {
    use tiny_http::{Header, Response, Server, StatusCode};
    let server = Server::http("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", server.server_addr());
    let object = Arc::new(Mutex::new(WireObject {
        bytes: vec![b'a'; size],
        metadata: HashMap::new(),
        etag: "old".into(),
    }));
    let stop = Arc::new(AtomicBool::new(false));
    let worker_object = object.clone();
    let worker_stop = stop.clone();
    let worker = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let state = Arc::new(Mutex::new((
            std::collections::BTreeMap::<
                String,
                (
                    HashMap<String, String>,
                    std::collections::BTreeMap<i32, Vec<u8>>,
                ),
            >::new(),
            0,
            0,
        )));
        let mut requests = Vec::new();
        while !worker_stop.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
            let Some(mut request) = server.recv_timeout(Duration::from_millis(50)).unwrap() else {
                continue;
            };
            let worker_object = worker_object.clone();
            let state = state.clone();
            requests.push(std::thread::spawn(move || {
            let headers: HashMap<_,_> = request.headers().iter().map(|header| (header.field.as_str().to_string().to_ascii_lowercase(),header.value.as_str().to_owned())).collect();
            let metadata: HashMap<_,_> = headers.iter().filter_map(|(key,value)| key.strip_prefix("x-amz-meta-").map(|key| (key.to_owned(),value.clone()))).collect();
            let url = reqwest::Url::parse(&format!("http://localhost{}",request.url())).unwrap();
            let query: HashMap<_,_> = url.query_pairs().into_owned().collect();
            let upload = query.get("uploadId");
            let mut body = Vec::new();
            if request.as_reader().read_to_end(&mut body).is_err() { return; }
            let mut state = state.lock().unwrap();
            let (uploads, next_upload, commits) = &mut *state;
            let response = match request.method().as_str() {
                "HEAD" => {
                    let object = worker_object.lock().unwrap();
                    let mut response = Response::from_data(object.bytes.clone()).with_chunked_threshold(usize::MAX).with_header(Header::from_bytes("ETag", object.etag.as_str()).unwrap());
                    for (name,value) in &object.metadata { response.add_header(Header::from_bytes(format!("x-amz-meta-{name}"),value.as_str()).unwrap()); }
                    response
                }
                "GET" => {
                    let object = worker_object.lock().unwrap();
                    if headers.get("if-match") != Some(&object.etag) {
                        Response::from_string("<Error><Code>PreconditionFailed</Code></Error>").with_status_code(StatusCode(412))
                    } else {
                        let range = headers["range"].strip_prefix("bytes=").unwrap();
                        let (start,end) = range.split_once('-').unwrap();
                        let start: usize = start.parse().unwrap(); let end: usize = end.parse().unwrap();
                        Response::from_data(object.bytes[start..=end].to_vec()).with_chunked_threshold(usize::MAX).with_status_code(StatusCode(206)).with_header(Header::from_bytes("Content-Range",format!("bytes {start}-{end}/{}",object.bytes.len())).unwrap())
                    }
                }
                "POST" if upload.is_none() => {
                    *next_upload += 1;
                    let id = format!("upload-{next_upload}");
                    uploads.insert(id.clone(), (metadata,std::collections::BTreeMap::new()));
                    Response::from_string(format!("<InitiateMultipartUploadResult><UploadId>{id}</UploadId></InitiateMultipartUploadResult>"))
                }
                "PUT" if upload.is_some() => {
                    let upload = uploads.get_mut(upload.unwrap()).unwrap();
                    let part: i32 = query["partNumber"].parse().unwrap();
                    if let Some(range) = headers.get("x-amz-copy-source-range") {
                        let object = worker_object.lock().unwrap();
                        assert_eq!(headers["x-amz-copy-source-if-match"],object.etag);
                        let (start,end) = range.strip_prefix("bytes=").unwrap().split_once('-').unwrap();
                        let start: usize = start.parse().unwrap(); let end: usize = end.parse().unwrap();
                        upload.1.insert(part,object.bytes[start..=end].to_vec());
                        Response::from_string(format!("<CopyPartResult><ETag>part-{part}</ETag></CopyPartResult>"))
                    } else {
                        upload.1.insert(part,body);
                        Response::from_data(Vec::new()).with_header(Header::from_bytes("ETag",format!("part-{part}")).unwrap())
                    }
                }
                "PUT" | "POST" => {
                    let content = if let Some(id) = upload {
                        uploads.remove(id).map(|(metadata,parts)| (parts.into_values().flatten().collect(),metadata))
                    } else { Some((body,metadata)) };
                    if let Some((bytes,metadata)) = content {
                        *commits += 1;
                        let etag = format!("committed-{commits}");
                        *worker_object.lock().unwrap() = WireObject { bytes,metadata,etag: etag.clone() };
                        if *commits == 1 {
                            Response::from_string("<Error><Code>ServiceUnavailable</Code></Error>").with_status_code(StatusCode(503))
                        } else if upload.is_some() {
                            Response::from_string(format!("<CompleteMultipartUploadResult><ETag>{etag}</ETag></CompleteMultipartUploadResult>"))
                        } else {
                            Response::from_data(Vec::new()).with_header(Header::from_bytes("ETag",etag).unwrap())
                        }
                    } else { Response::from_string("<Error><Code>NoSuchUpload</Code></Error>").with_status_code(StatusCode(404)) }
                }
                "DELETE" => {
                    uploads.remove(upload.unwrap());
                    Response::from_data(Vec::new()).with_status_code(StatusCode(204))
                }
                other => panic!("unexpected wire operation {other}"),
            };
            drop(state);
            // Cancelling a failed retry may have already closed its connection.
            let _ = request.respond(response);
            }));
        }
        for request in requests {
            request.join().unwrap();
        }
    });
    (endpoint, object, stop, worker)
}

#[tokio::test]
async fn real_sdk_commit_errors_and_automatic_retries_recover_using_uploaded_provenance() {
    use aws_sdk_s3::config::{
        BehaviorVersion, Credentials, Region, retry::RetryConfig, timeout::TimeoutConfig,
    };
    for size in [6, 6 * 1024 * 1024] {
        let (endpoint, object, stop, worker) = wire_backend(size);
        let mut config = MemoryStore::config(32);
        config.endpoint = endpoint.clone();
        config.no_checksum = true;
        config.sdk_max_retries = 1;
        config.http_timeout = Duration::from_secs(3);
        let sdk = aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new("us-east-1"))
            .credentials_provider(Credentials::new(
                "fixture-access",
                "fixture-secret",
                None,
                None,
                "fixture",
            ))
            .endpoint_url(endpoint)
            .force_path_style(true)
            .retry_config(RetryConfig::standard().with_max_attempts(2))
            .timeout_config(
                TimeoutConfig::builder()
                    .operation_timeout(Duration::from_secs(3))
                    .build(),
            )
            .build();
        let store = geesefs_rs::backend::s3::S3::new(
            aws_sdk_s3::Client::from_conf(sdk),
            "bucket".into(),
            String::new(),
        )
        .unwrap()
        .with_options(&config)
        .unwrap();
        let fs = Filesystem::new(config, Arc::new(store)).unwrap();
        let result = tokio::time::timeout(Duration::from_secs(15), async {
            let node = fs.lookup(1, "file").await.unwrap();
            fs.write(node.ino, 2, b"XY").await.unwrap();
            let failed = fs.fsync(node.ino).await;
            eprintln!("first SDK fsync size={size}: {failed:?}");
            assert!(failed.is_err());
            assert_eq!(&object.lock().unwrap().bytes[..6], b"aaXYaa");
            {
                let state = node.state.lock().unwrap();
                assert!(state.durable < state.version);
            }
            fs.write(node.ino, 4, b"Z").await.unwrap();
            fs.fsync(node.ino).await.unwrap();
            assert_eq!(&object.lock().unwrap().bytes[..6], b"aaXYZa");
            assert!(
                !fs.xattrs(node.ino)
                    .unwrap()
                    .contains_key("user.geesefs-upload-receipt")
            );
        })
        .await;
        stop.store(true, Ordering::SeqCst);
        worker.join().unwrap();
        result.unwrap();
    }
}
