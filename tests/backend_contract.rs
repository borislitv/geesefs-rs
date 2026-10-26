use std::{
    collections::{BTreeMap, HashMap},
    io::Cursor,
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
    backend::s3::{ObjectHead, S3},
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

fn server(
    status: u16,
    headers: &[(&str, &str)],
    body: &[u8],
    declared: Option<usize>,
) -> (S3, JoinHandle<Captured>) {
    let server = Server::http("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", server.server_addr());
    let headers: Vec<_> = headers
        .iter()
        .map(|(name, value)| Header::from_bytes(name.as_bytes(), value.as_bytes()).unwrap())
        .collect();
    let body = body.to_vec();
    let task = std::thread::spawn(move || {
        let mut request = server
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .expect("SDK did not send expected request");
        let mut captured = Captured {
            method: request.method().to_string(),
            url: request.url().to_owned(),
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
        let length = declared.unwrap_or(body.len());
        let _ = request.respond(Response::new(
            StatusCode(status),
            headers,
            Cursor::new(body),
            Some(length),
            None,
        ));
        captured
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
        .timeout_config(
            TimeoutConfig::builder()
                .operation_timeout(Duration::from_secs(3))
                .build(),
        )
        .build();
    (
        S3::new(Client::from_conf(config), "bucket".into(), "prefix".into()).unwrap(),
        task,
    )
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
