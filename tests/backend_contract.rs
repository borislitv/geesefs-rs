use std::{
    collections::{BTreeMap, HashMap},
    io::{Cursor, Read},
    thread::JoinHandle,
    time::Duration,
};

use aws_sdk_s3::{
    Client,
    config::{BehaviorVersion, Credentials, Region, retry::RetryConfig, timeout::TimeoutConfig},
    primitives::ByteStream,
    types::CompletedPart,
};
use geesefs_rs::{
    backend::{
        RangeSink,
        s3::{ObjectHead, S3},
    },
    budget::Budget,
    error::Error,
};
use tiny_http::{Header, Response, Server, StatusCode};

#[derive(Debug)]
struct Captured {
    method: String,
    url: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

fn capture(request: &mut tiny_http::Request) -> Captured {
    let mut captured = Captured {
        method: request.method().to_string(),
        url: request.url().to_owned(),
        headers: request
            .headers()
            .iter()
            .map(|header| {
                (
                    header.field.as_str().to_string().to_ascii_lowercase(),
                    header.value.as_str().to_owned(),
                )
            })
            .collect(),
        body: Vec::new(),
    };
    request.as_reader().read_to_end(&mut captured.body).unwrap();
    captured
}

fn server(
    status: u16,
    headers: &[(&str, &str)],
    body: &[u8],
    declared: Option<usize>,
) -> (S3, JoinHandle<Captured>) {
    server_reader(
        status,
        headers,
        Cursor::new(body.to_vec()),
        declared.unwrap_or(body.len()),
    )
}

fn server_reader(
    status: u16,
    headers: &[(&str, &str)],
    body: impl Read + Send + 'static,
    length: usize,
) -> (S3, JoinHandle<Captured>) {
    let server = Server::http("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", server.server_addr());
    let headers: Vec<_> = headers
        .iter()
        .map(|(name, value)| Header::from_bytes(name.as_bytes(), value.as_bytes()).unwrap())
        .collect();
    let task = std::thread::spawn(move || {
        let mut request = server
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .expect("SDK did not send expected request");
        let captured = capture(&mut request);
        let _ = request.respond(
            Response::new(StatusCode(status), headers, body, Some(length), None)
                .with_chunked_threshold(usize::MAX),
        );
        captured
    });
    (backend(endpoint), task)
}

#[derive(Debug)]
struct CloseConnection;

impl aws_smithy_runtime_api::client::interceptors::Intercept for CloseConnection {
    fn name(&self) -> &'static str {
        "CloseDisposableTestConnection"
    }

    fn modify_before_signing(
        &self,
        context: &mut aws_smithy_runtime_api::client::interceptors::context::BeforeTransmitInterceptorContextMut<'_>,
        _: &aws_smithy_runtime_api::client::runtime_components::RuntimeComponents,
        _: &mut aws_smithy_types::config_bag::ConfigBag,
    ) -> Result<(), aws_smithy_runtime_api::box_error::BoxError> {
        // A deliberately short body must produce EOF, not a keep-alive stall.
        context
            .request_mut()
            .headers_mut()
            .insert("connection", "close");
        Ok(())
    }
}

fn backend(endpoint: String) -> S3 {
    let config = aws_sdk_s3::Config::builder()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new("us-east-1"))
        .credentials_provider(Credentials::new(
            "disposable-access",
            "disposable-secret",
            None,
            None,
            "test-only",
        ))
        .endpoint_url(endpoint)
        .force_path_style(true)
        .interceptor(CloseConnection)
        .retry_config(RetryConfig::standard().with_max_attempts(1))
        .timeout_config(
            TimeoutConfig::builder()
                .operation_timeout(Duration::from_secs(3))
                .build(),
        )
        .build();
    S3::new(Client::from_conf(config), "bucket".into(), "prefix".into()).unwrap()
}

struct GatedBody {
    prefix: Cursor<Vec<u8>>,
    tail: Cursor<Vec<u8>>,
    entered: Option<tokio::sync::oneshot::Sender<()>>,
    release: Option<std::sync::mpsc::Receiver<()>>,
}

impl Read for GatedBody {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let read = self.prefix.read(buffer)?;
        if read != 0 {
            return Ok(read);
        }
        if let Some(release) = self.release.take() {
            let _ = self.entered.take().unwrap().send(());
            release
                .recv_timeout(Duration::from_secs(5))
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::TimedOut, error))?;
        }
        self.tail.read(buffer)
    }
}

fn file_server(body: impl Read + Send + 'static, length: usize) -> (S3, JoinHandle<Captured>) {
    let server = Server::http("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", server.server_addr());
    let task = std::thread::spawn(move || {
        let request = server
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        assert_eq!(request.method().to_string(), "HEAD");
        request
            .respond(
                Response::new(
                    StatusCode(200),
                    vec![Header::from_bytes("ETag", "\"generation-one\"").unwrap()],
                    Cursor::new(Vec::<u8>::new()),
                    Some(length),
                    None,
                )
                .with_chunked_threshold(usize::MAX),
            )
            .unwrap();
        let mut request = server
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        let captured = capture(&mut request);
        let _ = request.respond(
            Response::new(
                StatusCode(206),
                vec![
                    Header::from_bytes("Content-Range", format!("bytes 0-{}/{length}", length - 1))
                        .unwrap(),
                ],
                body,
                Some(length),
                None,
            )
            .with_chunked_threshold(usize::MAX),
        );
        captured
    });
    (backend(endpoint), task)
}

fn filesystem(store: S3) -> std::sync::Arc<geesefs_rs::filesystem::Filesystem> {
    use clap::Parser;
    use geesefs_rs::{config::Config, filesystem::Filesystem};
    let config = Config::try_parse_from([
        "geesefs-rs",
        "-f",
        "--list-type",
        "2",
        "--memory-limit",
        "32",
        "bucket",
        "/tmp/mount",
    ])
    .unwrap();
    Filesystem::new(config, std::sync::Arc::new(store)).unwrap()
}

#[tokio::test]
async fn a_complete_body_prefix_is_visible_before_the_conditional_get_finishes() {
    const CHUNK: usize = 128 * 1024;
    let expected: Vec<u8> = (0..4 * CHUNK).map(|at| (at % 251) as u8).collect();
    let (entered, at_tail) = tokio::sync::oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let body = GatedBody {
        prefix: Cursor::new(expected[..CHUNK].to_vec()),
        tail: Cursor::new(expected[CHUNK..].to_vec()),
        entered: Some(entered),
        release: Some(wait),
    };
    let (store, server) = server_reader(
        206,
        &[("Content-Range", "bytes 3-524290/524302")],
        body,
        expected.len(),
    );
    let data = Budget::new(expected.len()).unwrap();
    let local = Budget::new(expected.len()).unwrap();
    let (sender, mut received) = tokio::sync::mpsc::channel(2);
    let output = RangeSink::new(sender, data.reserve(expected.len()).await.unwrap());
    let load = tokio::spawn(async move {
        store
            .read_range_chunks("file", 3, 4 * CHUNK, &head(524302), &local, &output)
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), at_tail)
        .await
        .unwrap()
        .unwrap();
    let first = tokio::time::timeout(Duration::from_secs(2), received.recv()).await;
    let completed_early = load.is_finished();
    release.send(()).unwrap();
    let mut blocks = first
        .as_ref()
        .ok()
        .and_then(|value| value.as_ref())
        .map(|(at, block)| (*at, block.clone()))
        .into_iter()
        .collect::<Vec<_>>();
    while let Some(block) = tokio::time::timeout(Duration::from_secs(3), received.recv())
        .await
        .unwrap()
    {
        blocks.push(block);
    }
    let outcome = tokio::time::timeout(Duration::from_secs(3), load)
        .await
        .unwrap()
        .unwrap();
    let captured = server.join().unwrap();
    outcome.unwrap();
    let (offset, prefix) = first
        .expect("a complete 128 KiB body prefix must be published before the GET tail is released")
        .unwrap();
    assert!(
        !completed_early,
        "the same producer must still own the unfinished GET"
    );
    assert_eq!(offset, 3);
    assert_eq!(prefix.as_slice(), &expected[..CHUNK]);
    assert_eq!(captured.headers["range"], "bytes=3-524290");
    assert_eq!(captured.headers["if-match"], "\"generation-one\"");
    let mut next = 3;
    let mut observed = Vec::new();
    for (offset, block) in &blocks {
        assert_eq!(*offset, next);
        observed.extend_from_slice(block.as_slice());
        next += block.as_slice().len() as u64;
    }
    assert_eq!(observed, expected);
    assert_eq!(
        data.used(),
        expected.len(),
        "published fragments must retain the whole admitted GET charge"
    );
    drop(prefix);
    drop(blocks);
    assert_eq!(data.used(), 0);
}

#[tokio::test]
async fn range_delivery_retains_admission_after_the_producer_has_finished() {
    let (store, server) = server(206, &[("Content-Range", "bytes 0-3/4")], b"abcd", None);
    let data = Budget::new(4).unwrap();
    let local = Budget::new(4).unwrap();
    let (sender, mut received) = tokio::sync::mpsc::channel(1);
    let output = RangeSink::new(sender, data.reserve(4).await.unwrap());
    store
        .read_range_chunks("file", 0, 4, &head(4), &local, &output)
        .await
        .unwrap();
    drop(output);
    let (offset, block) = received.recv().await.unwrap();
    server.join().unwrap();
    assert_eq!(offset, 0);
    assert_eq!(block.as_slice(), b"abcd");
    assert_eq!(
        data.used(),
        4,
        "delivery must transfer admission to its retained allocation"
    );
    drop(block);
    assert_eq!(data.used(), 0);
}

#[tokio::test]
async fn invalid_range_or_whole_body_checksum_never_publishes_a_fragment() {
    let bytes = vec![b'x'; 256 * 1024];
    for (range, checksum) in [
        ("bytes 1-262144/262145", None),
        ("bytes 0-262143/262144", Some("AAAAAA==")),
    ] {
        let mut headers = vec![("Content-Range", range)];
        if let Some(checksum) = checksum {
            headers.push(("x-amz-checksum-crc32", checksum));
        }
        let (store, server) = server(206, &headers, &bytes, None);
        let data = Budget::new(bytes.len()).unwrap();
        let local = Budget::new(bytes.len()).unwrap();
        let (sender, mut received) = tokio::sync::mpsc::channel(4);
        let output = RangeSink::new(sender, data.reserve(bytes.len()).await.unwrap());
        let result = store
            .read_range_chunks(
                "file",
                0,
                bytes.len(),
                &head(bytes.len() as u64),
                &local,
                &output,
            )
            .await;
        drop(output);
        server.join().unwrap();
        assert!(
            result.is_err(),
            "range and SDK checksum failures must remain errors"
        );
        assert!(
            received.recv().await.is_none(),
            "unvalidated bytes must not be published"
        );
        assert_eq!(data.used(), 0);
        assert_eq!(local.used(), 0);
    }
}

#[tokio::test]
async fn a_checksummed_prefix_waits_for_complete_sdk_validation() {
    use base64::Engine;
    const CHUNK: usize = 128 * 1024;
    let expected: Vec<u8> = (0..2 * CHUNK).map(|at| (at % 251) as u8).collect();
    let checksum =
        base64::engine::general_purpose::STANDARD.encode(crc32fast::hash(&expected).to_be_bytes());
    let (entered, at_tail) = tokio::sync::oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let body = GatedBody {
        prefix: Cursor::new(expected[..CHUNK].to_vec()),
        tail: Cursor::new(expected[CHUNK..].to_vec()),
        entered: Some(entered),
        release: Some(wait),
    };
    let (store, server) = server_reader(
        206,
        &[
            ("Content-Range", "bytes 0-262143/262144"),
            ("x-amz-checksum-crc32", &checksum),
        ],
        body,
        expected.len(),
    );
    let data = Budget::new(expected.len()).unwrap();
    let local = Budget::new(expected.len()).unwrap();
    let (sender, mut received) = tokio::sync::mpsc::channel(4);
    let output = RangeSink::new(sender, data.reserve(expected.len()).await.unwrap());
    let load = tokio::spawn(async move {
        store
            .read_range_chunks(
                "file",
                0,
                2 * CHUNK,
                &head((2 * CHUNK) as u64),
                &local,
                &output,
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), at_tail)
        .await
        .unwrap()
        .unwrap();
    let early = tokio::time::timeout(Duration::from_millis(500), received.recv()).await;
    release.send(()).unwrap();
    let mut observed = Vec::new();
    if let Ok(Some((_, block))) = &early {
        observed.extend_from_slice(block.as_slice());
    }
    while let Some((_, block)) = tokio::time::timeout(Duration::from_secs(3), received.recv())
        .await
        .unwrap()
    {
        observed.extend_from_slice(block.as_slice());
    }
    let result = tokio::time::timeout(Duration::from_secs(3), load)
        .await
        .unwrap()
        .unwrap();
    server.join().unwrap();
    result.unwrap();
    assert!(
        early.is_err(),
        "a whole-body checksum must be validated before any prefix is exposed"
    );
    assert_eq!(observed, expected);
    assert_eq!(data.used(), 0);
}

#[tokio::test]
async fn filesystem_reply_publishes_a_body_prefix_while_its_get_remains_owned() {
    const CHUNK: usize = 128 * 1024;
    let expected: Vec<u8> = (0..4 * CHUNK).map(|at| (at % 251) as u8).collect();
    let (entered, at_tail) = tokio::sync::oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let body = GatedBody {
        prefix: Cursor::new(expected[..CHUNK].to_vec()),
        tail: Cursor::new(expected[CHUNK..].to_vec()),
        entered: Some(entered),
        release: Some(wait),
    };
    let (store, task) = file_server(body, expected.len());
    let fs = filesystem(store);
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    let (ready, received) = tokio::sync::oneshot::channel();
    let load = tokio::spawn({
        let fs = fs.clone();
        async move {
            fs.read_reply(ino, 0, CHUNK, move |result| {
                let _ = ready.send(result);
            })
            .await
        }
    });
    tokio::time::timeout(Duration::from_secs(2), at_tail)
        .await
        .unwrap()
        .unwrap();
    let early = tokio::time::timeout(Duration::from_secs(2), received).await;
    let completed_early = load.is_finished();
    release.send(()).unwrap();
    let outcome = tokio::time::timeout(Duration::from_secs(3), load)
        .await
        .unwrap()
        .unwrap();
    let captured = task.join().unwrap();
    assert!(outcome.is_none());
    let block = early
        .expect("FUSE-core requested bytes must be published before the GET body finishes")
        .unwrap()
        .unwrap();
    assert!(
        !completed_early,
        "the original admitted future must own the remaining body"
    );
    assert_eq!(block.as_slice(), &expected[..CHUNK]);
    assert_eq!(captured.headers["range"], "bytes=0-524287");
    assert_eq!(captured.headers["if-match"], "\"generation-one\"");
    assert!(fs.effective_budgets().data_used_bytes <= fs.effective_budgets().data_shared_bytes);
    let following = tokio::time::timeout(
        Duration::from_secs(3),
        fs.read(ino, CHUNK as u64 + 7, CHUNK),
    )
    .await
    .expect("an already cached unaligned fragment span must not wait for another GET")
    .expect("an already cached unaligned fragment span must not require another GET");
    assert_eq!(following.as_slice(), &expected[CHUNK + 7..2 * CHUNK + 7]);
    fs.write(ino, CHUNK as u64 + 3, b"changed").await.unwrap();
    let changed = tokio::time::timeout(Duration::from_secs(3), fs.read(ino, CHUNK as u64 + 3, 7))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(changed.as_slice(), b"changed");
    assert_eq!(following.as_slice(), &expected[CHUNK + 7..2 * CHUNK + 7]);
}

#[tokio::test]
async fn a_short_body_publishes_only_complete_prefixes_and_releases_unpublished_charge() {
    const CHUNK: usize = 128 * 1024;
    let expected: Vec<u8> = (0..2 * CHUNK).map(|at| (at % 251) as u8).collect();
    for length in [CHUNK - 1, CHUNK + 17] {
        let (store, task) = server(
            206,
            &[("Content-Range", "bytes 0-262143/262144")],
            &expected[..length],
            Some(expected.len()),
        );
        let data = Budget::new(expected.len()).unwrap();
        let local = Budget::new(expected.len()).unwrap();
        let (sender, mut received) = tokio::sync::mpsc::channel(2);
        let output = RangeSink::new(sender, data.reserve(expected.len()).await.unwrap());
        let result = store
            .read_range_chunks(
                "file",
                0,
                expected.len(),
                &head(expected.len() as u64),
                &local,
                &output,
            )
            .await;
        drop(output);
        task.join().unwrap();
        assert!(
            matches!(
                result,
                Err(Error::Backend {
                    operation: "GET body",
                    ..
                })
            ),
            "a truncated body must remain an observable body error: {result:?}"
        );
        let mut blocks = Vec::new();
        while let Some(block) = tokio::time::timeout(Duration::from_secs(3), received.recv())
            .await
            .unwrap()
        {
            blocks.push(block);
        }
        assert_eq!(
            blocks.len(),
            length / CHUNK,
            "an incomplete fragment must never be published"
        );
        for (offset, block) in &blocks {
            assert_eq!(*offset, 0);
            assert_eq!(block.as_slice(), &expected[..CHUNK]);
        }
        assert_eq!(local.used(), blocks.len() * CHUNK);
        assert_eq!(
            data.used(),
            if blocks.is_empty() { 0 } else { expected.len() }
        );
        drop(blocks);
        assert_eq!(local.used(), 0);
        assert_eq!(data.used(), 0);
    }
}

#[tokio::test]
async fn closing_delivery_after_a_prefix_cancels_the_get_without_losing_its_pinned_owner() {
    const CHUNK: usize = 128 * 1024;
    let expected: Vec<u8> = (0..4 * CHUNK).map(|at| (at % 251) as u8).collect();
    let (entered, at_tail) = tokio::sync::oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let body = GatedBody {
        prefix: Cursor::new(expected[..CHUNK].to_vec()),
        tail: Cursor::new(expected[CHUNK..].to_vec()),
        entered: Some(entered),
        release: Some(wait),
    };
    let (store, task) = server_reader(
        206,
        &[("Content-Range", "bytes 0-524287/524288")],
        body,
        expected.len(),
    );
    let data = Budget::new(expected.len()).unwrap();
    let local = Budget::new(expected.len()).unwrap();
    let (sender, mut received) = tokio::sync::mpsc::channel(1);
    let output = RangeSink::new(sender, data.reserve(expected.len()).await.unwrap());
    let load = tokio::spawn({
        let local = local.clone();
        async move {
            store
                .read_range_chunks(
                    "file",
                    0,
                    4 * CHUNK,
                    &head((4 * CHUNK) as u64),
                    &local,
                    &output,
                )
                .await
        }
    });
    tokio::time::timeout(Duration::from_secs(2), at_tail)
        .await
        .unwrap()
        .unwrap();
    let (_, block) = tokio::time::timeout(Duration::from_secs(2), received.recv())
        .await
        .unwrap()
        .unwrap();
    drop(received);
    release.send(()).unwrap();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(3), load)
            .await
            .unwrap()
            .unwrap(),
        Err(Error::Cancelled)
    ));
    task.join().unwrap();
    assert_eq!(block.as_slice(), &expected[..CHUNK]);
    assert_eq!(data.used(), expected.len());
    assert_eq!(local.used(), CHUNK);
    drop(block);
    assert_eq!(data.used(), 0);
    assert_eq!(local.used(), 0);
}

#[tokio::test]
async fn filesystem_keeps_a_valid_prefix_after_tail_failure_or_cancellation() {
    const CHUNK: usize = 128 * 1024;
    let expected: Vec<u8> = (0..4 * CHUNK).map(|at| (at % 251) as u8).collect();
    for cancel in [false, true] {
        let (entered, at_tail) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let body = GatedBody {
            prefix: Cursor::new(expected[..CHUNK].to_vec()),
            tail: Cursor::new(expected[CHUNK..CHUNK + 17].to_vec()),
            entered: Some(entered),
            release: Some(wait),
        };
        let (store, task) = file_server(body, expected.len());
        let fs = filesystem(store);
        let ino = fs.lookup(1, "file").await.unwrap().ino;
        let (ready, received) = tokio::sync::oneshot::channel();
        let load = tokio::spawn({
            let fs = fs.clone();
            async move {
                fs.read_reply(ino, 0, CHUNK, move |result| {
                    let _ = ready.send(result);
                })
                .await
            }
        });
        tokio::time::timeout(Duration::from_secs(2), at_tail)
            .await
            .unwrap()
            .unwrap();
        let block = tokio::time::timeout(Duration::from_secs(2), received)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(block.as_slice(), &expected[..CHUNK]);
        assert!(!load.is_finished());
        if cancel {
            load.abort();
            assert!(
                tokio::time::timeout(Duration::from_secs(3), load)
                    .await
                    .unwrap()
                    .unwrap_err()
                    .is_cancelled()
            );
            release.send(()).unwrap();
        } else {
            release.send(()).unwrap();
            let tail = tokio::time::timeout(Duration::from_secs(3), load)
                .await
                .unwrap()
                .unwrap();
            assert!(
                matches!(
                    tail,
                    Some(Error::Backend {
                        operation: "GET body",
                        ..
                    })
                ),
                "a late body failure must remain observable after the successful reply: {tail:?}"
            );
        }
        task.join().unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            while fs.effective_budgets().data_used_bytes != expected.len() + CHUNK {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("only the cache's complete GET charge and published reply may remain");
        let hit = tokio::time::timeout(Duration::from_secs(3), fs.read(ino, 3, 7))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(hit.as_slice(), &expected[3..10]);
        assert_eq!(block.as_slice(), &expected[..CHUNK]);
        drop(hit);
        drop(block);
        assert_eq!(fs.effective_budgets().data_used_bytes, expected.len());
    }
}

fn head(size: u64) -> ObjectHead {
    ObjectHead {
        size,
        etag: "\"generation-one\"".into(),
        metadata: HashMap::new(),
        ..Default::default()
    }
}

#[tokio::test]
async fn actual_conditional_unaligned_range_and_eof() {
    let (store, task) = server(206, &[("Content-Range", "bytes 3-6/10")], b"3456", None);
    let budget = Budget::new(16).unwrap();
    let block = store
        .read_range("file", 3, 4, &head(10), &budget)
        .await
        .unwrap();
    assert_eq!(block.as_slice(), b"3456");
    assert_eq!(budget.used(), 4);
    let captured = task.join().unwrap();
    assert_eq!(captured.method, "GET");
    assert_eq!(captured.url, "/bucket/prefix/file?x-id=GetObject");
    assert_eq!(captured.headers["range"], "bytes=3-6");
    assert_eq!(
        captured.headers.get("if-match").map(String::as_str),
        Some("\"generation-one\""),
        "GET must retain its source generation condition"
    );
    assert!(captured.headers["authorization"].starts_with("AWS4-HMAC-SHA256 "));
    // No HTTP request is possible after the owned server has exited.
    let eof = store
        .read_range("file", 10, 100, &head(10), &budget)
        .await
        .unwrap();
    assert!(eof.as_slice().is_empty());
    drop(block);
    assert_eq!(budget.used(), 0);
}

#[tokio::test]
async fn stored_gzip_is_returned_verbatim_not_decompressed() {
    // A complete gzip member for ASCII "hello", not a generated expectation.
    let gzip = b"\x1f\x8b\x08\x00\x00\x00\x00\x00\x02\x03\xcb\x48\xcd\xc9\xc9\x07\x00\x86\xa6\x10\x36\x05\x00\x00\x00";
    let (store, task) = server(
        206,
        &[
            ("Content-Range", "bytes 0-24/25"),
            ("Content-Encoding", "gzip"),
        ],
        gzip,
        None,
    );
    let block = store
        .read_range("archive", 0, 100, &head(25), &Budget::new(32).unwrap())
        .await
        .unwrap();
    assert_eq!(block.as_slice(), gzip);
    task.join().unwrap();
}

#[tokio::test]
async fn short_or_wrong_range_body_never_publishes_a_block_or_leaks_budget() {
    for (range, body, declared) in [
        ("bytes 0-3/4", b"ab".as_slice(), Some(4)),
        ("bytes 1-4/5", b"abcd".as_slice(), None),
    ] {
        let (store, task) = server(206, &[("Content-Range", range)], body, declared);
        let budget = Budget::new(8).unwrap();
        assert!(
            store
                .read_range("file", 0, 4, &head(4), &budget)
                .await
                .is_err()
        );
        task.join().unwrap();
        assert_eq!(budget.used(), 0);
    }
}

#[tokio::test]
async fn access_error_preserves_modeled_code_and_releases_body_charge() {
    let (store, task) = server(
        403,
        &[("Content-Type", "application/xml")],
        b"<Error><Code>AccessDenied</Code><Message>fixture denial</Message></Error>",
        None,
    );
    let budget = Budget::new(8).unwrap();
    let error = store
        .read_range("file", 0, 4, &head(4), &budget)
        .await
        .err()
        .unwrap();
    assert!(matches!(error, Error::Backend { code, .. } if code == "AccessDenied"));
    assert_eq!(budget.used(), 0);
    task.join().unwrap();
}

#[tokio::test]
async fn multipart_copy_escapes_source_and_preserves_generation_condition() {
    let (store, task) = server(
        200,
        &[("Content-Type", "application/xml")],
        b"<CopyPartResult><ETag>\"copied\"</ETag></CopyPartResult>",
        None,
    );
    let part = store
        .copy_part(
            "new",
            "upload-fixture",
            2,
            "folder/a b%é",
            5..=9,
            "\"source-etag\"",
        )
        .await
        .unwrap();
    assert_eq!(part.part_number(), Some(2));
    assert_eq!(part.e_tag(), Some("\"copied\""));
    let captured = task.join().unwrap();
    assert_eq!(captured.method, "PUT");
    assert!(captured.url.contains("partNumber=2"));
    assert!(captured.url.contains("uploadId=upload-fixture"));
    assert_eq!(
        captured.headers["x-amz-copy-source"],
        "bucket/prefix/folder/a%20b%25%C3%A9"
    );
    assert_eq!(
        captured.headers["x-amz-copy-source-if-match"],
        "\"source-etag\""
    );
    assert_eq!(captured.headers["x-amz-copy-source-range"], "bytes=5-9");
}

#[tokio::test]
async fn embedded_200_completion_error_is_not_durability_success() {
    let (store, task) = server(
        200,
        &[("Content-Type", "application/xml")],
        b"<Error><Code>InternalError</Code><Message>injected completion failure</Message></Error>",
        None,
    );
    let parts = vec![
        CompletedPart::builder()
            .part_number(1)
            .e_tag("\"part\"")
            .build(),
    ];
    let error = store
        .complete_upload("new", "upload-fixture", parts)
        .await
        .unwrap_err();
    assert!(matches!(error, Error::Backend { code, .. } if code == "InternalError"));
    let captured = task.join().unwrap();
    assert_eq!(captured.method, "POST");
    let body = String::from_utf8(captured.body).unwrap();
    assert!(body.contains("<PartNumber>1</PartNumber>"));
    assert!(body.contains("part"));
}

#[tokio::test]
async fn put_and_head_retain_explicit_metadata() {
    let (store, task) = server(200, &[("ETag", "\"put\"")], b"", None);
    store
        .put(
            "new",
            ByteStream::from_static(b"payload"),
            HashMap::from([("mtime".into(), "1700000000".into())]),
        )
        .await
        .unwrap();
    let captured = task.join().unwrap();
    assert_eq!(captured.method, "PUT");
    assert_eq!(captured.body, b"payload");
    assert_eq!(captured.headers["x-amz-meta-mtime"], "1700000000");

    let (store, task) = server(
        200,
        &[("ETag", "\"head\""), ("x-amz-meta-custom", "escaped%25")],
        b"",
        Some(7),
    );
    let object = store.head("new").await.unwrap();
    assert_eq!(object.size, 7);
    assert_eq!(object.etag, "\"head\"");
    assert_eq!(object.metadata["custom"], "escaped%25");
    assert_eq!(task.join().unwrap().method, "HEAD");
}

fn listing_server(responses: Vec<(u16, Vec<u8>, bool)>) -> (S3, JoinHandle<Vec<Captured>>) {
    let server = Server::http("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", server.server_addr());
    let task = std::thread::spawn(move || {
        responses
            .into_iter()
            .map(|(status, body, gzip)| {
                let mut request = server
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap()
                    .expect("expected listing request");
                let mut captured = Captured {
                    method: request.method().to_string(),
                    url: request.url().into(),
                    headers: request
                        .headers()
                        .iter()
                        .map(|h| {
                            (
                                h.field.as_str().to_string().to_ascii_lowercase(),
                                h.value.as_str().to_owned(),
                            )
                        })
                        .collect(),
                    body: Vec::new(),
                };
                request.as_reader().read_to_end(&mut captured.body).unwrap();
                let mut response = Response::from_data(body)
                    .with_status_code(status)
                    .with_header(Header::from_bytes("Content-Type", "application/xml").unwrap());
                if gzip {
                    response = response
                        .with_header(Header::from_bytes("Content-Encoding", "gzip").unwrap());
                }
                request.respond(response).unwrap();
                captured
            })
            .collect()
    });
    let config = aws_sdk_s3::Config::builder()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new("us-east-1"))
        .credentials_provider(Credentials::new(
            "disposable-access",
            "disposable-secret",
            None,
            None,
            "test-only",
        ))
        .endpoint_url(endpoint)
        .force_path_style(true)
        .retry_config(RetryConfig::standard().with_max_attempts(1))
        .build();
    (
        S3::new(Client::from_conf(config), "bucket".into(), "prefix".into()).unwrap(),
        task,
    )
}

#[tokio::test]
async fn extension_uses_sdk_signed_query_and_only_exact_unsupported_fallback() {
    use geesefs_rs::config::ListType;
    let standard=b"<ListBucketResult><IsTruncated>false</IsTruncated><Contents><Key>prefix/a</Key><Size>1</Size><ETag>\"a\"</ETag></Contents></ListBucketResult>".to_vec();
    let (store, task) = listing_server(vec![
        (
            501,
            b"<Error><Code>NotImplemented</Code></Error>".to_vec(),
            false,
        ),
        (200, standard, false),
    ]);
    let page = store
        .with_list_type(ListType::Extended)
        .list("", Some("/"), None)
        .await
        .unwrap();
    assert_eq!(page.objects[0].key, "a");
    assert!(page.objects[0].metadata.is_none());
    let requests = task.join().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].url.starts_with("/bucket?"));
    assert!(requests[0].url.contains("list-type=ext-v1"));
    assert!(requests[0].url.contains("X-Amz-Signature="));
    assert!(!requests[1].url.contains("list-type="));
    let (store, task) = listing_server(vec![(
        403,
        b"<Error><Code>AccessDenied</Code></Error>".to_vec(),
        false,
    )]);
    let error = store
        .with_list_type(ListType::Extended)
        .list("", Some("/"), None)
        .await
        .unwrap_err();
    assert!(matches!(error,Error::Backend{code,..}if code=="AccessDenied"));
    assert_eq!(task.join().unwrap().len(), 1);
}

#[tokio::test]
async fn extension_gzip_integrity_and_authoritative_empty_metadata() {
    use geesefs_rs::config::ListType;
    use std::io::Write;
    let xml=b"<ListBucketResult><IsTruncated>false</IsTruncated><Contents><Key>prefix/a</Key><Size>0</Size></Contents></ListBucketResult>";
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(xml).unwrap();
    let bytes = encoder.finish().unwrap();
    let (store, task) = listing_server(vec![(200, bytes.clone(), true)]);
    let page = store
        .with_list_type(ListType::Extended)
        .list("", None, None)
        .await
        .unwrap();
    assert_eq!(page.objects[0].metadata, Some(BTreeMap::new()));
    assert!(task.join().unwrap()[0].headers["accept-encoding"].contains("gzip"));
    let mut corrupt = bytes;
    let length = corrupt.len();
    corrupt[length - 8] ^= 1;
    let (store, task) = listing_server(vec![(200, corrupt, true)]);
    assert!(
        store
            .with_list_type(ListType::Extended)
            .list("", None, None)
            .await
            .is_err()
    );
    task.join().unwrap();
}
