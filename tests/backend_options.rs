#![forbid(unsafe_code)]
use aws_sdk_s3::{
    Client,
    config::{BehaviorVersion, Credentials, Region, retry::RetryConfig, timeout::TimeoutConfig},
    primitives::ByteStream,
};
use clap::Parser;
use geesefs_rs::{backend::s3::S3, budget::Budget, config::Config, error::Error};
use std::{
    collections::{BTreeMap, HashMap},
    io::Cursor,
    time::{Duration, SystemTime},
};
use tiny_http::{Header, Response, Server, StatusCode};

struct Reply {
    status: u16,
    headers: Vec<(&'static str, &'static str)>,
    body: String,
}
fn xml(body: &str) -> Reply {
    Reply {
        status: 200,
        headers: vec![("Content-Type", "application/xml")],
        body: body.into(),
    }
}
fn head() -> Reply {
    Reply {
        status: 200,
        headers: vec![("ETag", "\"generation\"")],
        body: String::new(),
    }
}
#[derive(Debug)]
struct Request {
    method: String,
    url: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}
fn server(replies: Vec<Reply>) -> (String, std::thread::JoinHandle<Vec<Request>>) {
    let server = Server::http("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", server.server_addr());
    let task = std::thread::spawn(move || {
        let mut captured = Vec::new();
        for reply in replies {
            let mut request = server
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .expect("missing expected request");
            let mut body = Vec::new();
            request.as_reader().read_to_end(&mut body).unwrap();
            captured.push(Request {
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
                body,
            });
            let headers = reply
                .headers
                .iter()
                .map(|(n, v)| Header::from_bytes(n.as_bytes(), v.as_bytes()).unwrap())
                .collect();
            let length = reply.body.len();
            request
                .respond(
                    Response::new(
                        StatusCode(reply.status),
                        headers,
                        Cursor::new(reply.body),
                        Some(length),
                        None,
                    )
                    .with_chunked_threshold(usize::MAX),
                )
                .unwrap();
        }
        captured
    });
    (endpoint, task)
}
fn config(endpoint: &str, extra: &[&str]) -> Config {
    let mut args = vec![
        "geesefs-rs",
        "-f",
        "--sdk-max-retries=0",
        "--http-timeout=2s",
        "--endpoint",
        endpoint,
    ];
    args.extend_from_slice(extra);
    args.extend(["bucket:prefix", "/tmp/mount"]);
    Config::try_parse_from(args).unwrap()
}
fn store(endpoint: &str, c: &Config) -> S3 {
    store_at(endpoint, c, None)
}
#[derive(Clone, Debug)]
struct Clock(std::sync::Arc<std::sync::atomic::AtomicU64>);
impl aws_smithy_async::time::TimeSource for Clock {
    fn now(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH
            + Duration::from_secs(self.0.load(std::sync::atomic::Ordering::SeqCst))
    }
}
fn store_at(endpoint: &str, c: &Config, clock: Option<Clock>) -> S3 {
    let mut builder = aws_sdk_s3::Config::builder()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new("us-east-1"))
        .credentials_provider(Credentials::new(
            "disposable-access",
            "disposable-secret",
            None,
            None,
            "fixture",
        ))
        .endpoint_url(endpoint)
        .force_path_style(true)
        .retry_config(RetryConfig::standard().with_max_attempts(1))
        .timeout_config(
            TimeoutConfig::builder()
                .operation_timeout(Duration::from_secs(2))
                .build(),
        );
    if let Some(clock) = clock {
        builder = builder.time_source(clock);
    }
    let sdk = builder.build();
    S3::new(Client::from_conf(sdk), "bucket".into(), "prefix".into())
        .unwrap()
        .with_options(c)
        .unwrap()
}

#[tokio::test]
async fn sse_kms_storage_acl_mime_and_requester_are_signed_on_real_requests() {
    let (endpoint, task) = server(vec![head(), head(), head()]);
    let c = config(
        &endpoint,
        &[
            "--sse-kms=public-key-id",
            "--storage-class=STANDARD_IA",
            "--cold-min-size=4",
            "--acl=private",
            "--use-content-type",
            "--requester-pays",
        ],
    );
    let s = store(&endpoint, &c);
    s.put("small.txt", ByteStream::from_static(b"abc"), HashMap::new())
        .await
        .unwrap();
    s.put(
        "large.txt",
        ByteStream::from_static(b"abcd"),
        HashMap::new(),
    )
    .await
    .unwrap();
    s.head("large.txt").await.unwrap();
    let requests = task.join().unwrap();
    assert_eq!(requests[0].body, b"abc");
    assert_eq!(requests[0].headers["x-amz-storage-class"], "STANDARD");
    assert_eq!(requests[1].headers["x-amz-storage-class"], "STANDARD_IA");
    for r in &requests[..2] {
        assert_eq!(r.headers["x-amz-server-side-encryption"], "aws:kms");
        assert_eq!(
            r.headers["x-amz-server-side-encryption-aws-kms-key-id"],
            "public-key-id"
        );
        assert_eq!(r.headers["x-amz-acl"], "private");
        assert!(r.headers["content-type"].starts_with("text/plain"));
        assert!(r.headers["authorization"].contains("x-amz-server-side-encryption"));
        assert_eq!(r.headers["x-amz-request-payer"], "requester");
    }
    assert_eq!(requests[2].headers["x-amz-request-payer"], "requester");
    assert!(
        !requests[2]
            .headers
            .contains_key("x-amz-server-side-encryption")
    );
}

#[tokio::test]
async fn customer_key_and_digest_propagate_to_copy_and_multipart_operations() {
    let (endpoint, task) = server(vec![
        head(),
        head(),
        xml(
            "<InitiateMultipartUploadResult><UploadId>owned</UploadId></InitiateMultipartUploadResult>",
        ),
        head(),
        xml("<CopyPartResult><ETag>part-copy</ETag></CopyPartResult>"),
        xml("<CompleteMultipartUploadResult><ETag>complete</ETag></CompleteMultipartUploadResult>"),
        Reply {
            status: 200,
            headers: vec![("ETag", "old")],
            body: String::new(),
        },
        xml("<CopyObjectResult><ETag>copy</ETag></CopyObjectResult>"),
    ]);
    let key = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
    let c = config(&endpoint, &["--sse-c", key]);
    let s = store(&endpoint, &c);
    s.head("file").await.unwrap();
    s.put("file", ByteStream::from_static(b"abc"), HashMap::new())
        .await
        .unwrap();
    let upload = s.begin_upload("file", HashMap::new(), 3).await.unwrap();
    let part = s
        .upload_part("file", &upload, 1, ByteStream::from_static(b"abc"))
        .await
        .unwrap();
    s.copy_part("file", &upload, 2, "source", 0..=2, "old")
        .await
        .unwrap();
    s.complete_upload("file", &upload, vec![part])
        .await
        .unwrap();
    s.copy("source", "target", "old").await.unwrap();
    for (i, r) in task.join().unwrap().iter().enumerate() {
        assert_eq!(
            r.headers["x-amz-server-side-encryption-customer-algorithm"],
            "AES256"
        );
        assert_eq!(r.headers["x-amz-server-side-encryption-customer-key"], key);
        assert_eq!(
            r.headers["x-amz-server-side-encryption-customer-key-md5"],
            "cLyPS3KoaSFGi/joRB3OUQ=="
        );
        if i == 4 || i == 7 {
            assert_eq!(
                r.headers["x-amz-copy-source-server-side-encryption-customer-key"],
                key
            );
        }
    }
    assert!(
        !format!("{c:?}").contains(key),
        "Config Debug must redact keys"
    );
}

#[tokio::test]
async fn iam_refresh_uses_wall_time_after_a_pause_without_sleeping_or_using_an_expired_token() {
    let (metadata, metadata_task) = server(vec![
        xml(r#"{"access_token":"first","expires_in":3600}"#),
        xml(r#"{"access_token":"refreshed","expires_in":3600}"#),
    ]);
    let (endpoint, task) = server(vec![head(), head(), head()]);
    let c = config(&endpoint, &["--iam", "--iam-url", &metadata]);
    let clock = Clock(std::sync::Arc::new(std::sync::atomic::AtomicU64::new(
        1767225600,
    )));
    let s = store_at(&endpoint, &c, Some(clock.clone()));
    s.head("a").await.unwrap();
    clock
        .0
        .store(1767225630, std::sync::atomic::Ordering::SeqCst);
    s.head("a").await.unwrap();
    clock
        .0
        .store(1767229201, std::sync::atomic::Ordering::SeqCst);
    s.head("a").await.unwrap();
    assert_eq!(metadata_task.join().unwrap().len(), 2);
    let requests = task.join().unwrap();
    assert_eq!(requests[0].headers["x-yacloud-subjecttoken"], "first");
    assert_eq!(requests[1].headers["x-yacloud-subjecttoken"], "first");
    assert_eq!(requests[2].headers["x-yacloud-subjecttoken"], "refreshed");
}

#[tokio::test]
async fn gcp_iam_is_cached_and_authenticates_sdk_and_extension_without_aws_signature() {
    let (metadata, metadata_task) = server(vec![xml(
        r#"{"access_token":"disposable-token","expires_in":3600}"#,
    )]);
    let (endpoint, task) = server(vec![
        head(),
        head(),
        xml("<ListBucketResult><IsTruncated>false</IsTruncated></ListBucketResult>"),
    ]);
    let c = config(
        &endpoint,
        &[
            "--iam",
            "--iam-url",
            &metadata,
            "--iam-header=Custom-Token",
            "--list-type=ext-v1",
        ],
    );
    let s = S3::from_config(&c).await.unwrap();
    let (a, b) = tokio::join!(s.head("a"), s.head("b"));
    a.unwrap();
    b.unwrap();
    s.list("", None, None).await.unwrap();
    let metadata_requests = metadata_task.join().unwrap();
    assert_eq!(metadata_requests.len(), 1);
    assert_eq!(metadata_requests[0].headers["metadata-flavor"], "Google");
    for r in task.join().unwrap() {
        assert_eq!(r.headers["custom-token"], "disposable-token");
        assert!(!r.headers.contains_key("authorization"));
        assert!(!r.url.contains("X-Amz-Signature"));
        assert!(!r.url.contains("disposable-access"));
    }
}

#[tokio::test]
async fn imds_token_schema_and_failed_or_expired_metadata_never_fall_back_to_aws() {
    for (body, success) in [
        (
            r#"{"Code":"Success","Token":"imds-token","Expiration":"2099-01-01T00:00:00Z"}"#,
            true,
        ),
        (
            r#"{"Code":"Denied","Token":"do-not-log","Expiration":"2099-01-01T00:00:00Z"}"#,
            false,
        ),
        (
            r#"{"Code":"Success","Token":"do-not-log","Expiration":"2000-01-01T00:00:00Z"}"#,
            false,
        ),
    ] {
        let (metadata, metadata_task) = server(vec![xml(body)]);
        let (endpoint, task) = server(if success { vec![head()] } else { vec![] });
        let c = config(
            &endpoint,
            &["--iam", "--iam-flavor=imdsv1", "--iam-url", &metadata],
        );
        let s = store(&endpoint, &c);
        let result = s.head("file").await;
        if success {
            result.unwrap();
            assert_eq!(
                task.join().unwrap()[0].headers["x-yacloud-subjecttoken"],
                "imds-token"
            );
        } else {
            let error = result.unwrap_err().to_string();
            assert!(!error.contains("do-not-log"));
            assert!(task.join().unwrap().is_empty());
        }
        assert!(
            !metadata_task.join().unwrap()[0]
                .headers
                .contains_key("metadata-flavor")
        );
    }
}

#[tokio::test]
async fn iam_rejects_http_failures_redirects_and_oversize_without_contacting_s3() {
    for (status, body, headers) in [
        (302,"do-not-log".into(),vec![("Location","/must-not-follow")]),
        (403,"do-not-log".into(),vec![]),
        (200,serde_json::json!({"access_token":"do-not-log","expires_in":3600,"padding":"x".repeat(65536)}).to_string(),vec![]),
    ] {
        let metadata=std::sync::Arc::new(Server::http("127.0.0.1:0").unwrap());
        let metadata_url=format!("http://{}",metadata.server_addr());
        let responder=metadata.clone();
        let task=std::thread::spawn(move || {
            let request=responder.recv_timeout(Duration::from_secs(5)).unwrap().expect("IAM metadata request");
            let mut response=Response::from_string(body).with_status_code(StatusCode(status));
            for(name,value)in headers{response.add_header(Header::from_bytes(name,value).unwrap());}
            request.respond(response).unwrap();
        });
        // Keep the S3 listener alive: a real forbidden request would be queued,
        // not confused with an unrelated connection-refused failure.
        let endpoint=Server::http("127.0.0.1:0").unwrap();let url=format!("http://{}",endpoint.server_addr());
        let c=config(&url,&["--iam","--iam-url",&metadata_url]);let s=store(&url,&c);
        let result=tokio::time::timeout(Duration::from_secs(3),s.head("file")).await.expect("IAM failure must terminate within the SDK deadline");
        assert!(result.is_err());assert!(!result.unwrap_err().to_string().contains("do-not-log"));
        task.join().unwrap();assert!(metadata.try_recv().unwrap().is_none(),"metadata redirects must not be followed");assert!(endpoint.try_recv().unwrap().is_none(),"rejected IAM metadata must never permit an S3 request");
    }
}

#[tokio::test]
async fn multipart_cold_storage_uses_final_size_at_the_exact_boundary() {
    for (size, expected) in [(3, "STANDARD"), (4, "STANDARD_IA")] {
        let (endpoint, task) = server(vec![xml(
            "<InitiateMultipartUploadResult><UploadId>owned</UploadId></InitiateMultipartUploadResult>",
        )]);
        let c = config(
            &endpoint,
            &["--storage-class=STANDARD_IA", "--cold-min-size=4"],
        );
        let s = store(&endpoint, &c);
        s.begin_upload("file", HashMap::new(), size).await.unwrap();
        let requests = task.join().unwrap();
        assert_eq!(requests[0].headers["x-amz-storage-class"], expected);
    }
}

#[tokio::test]
async fn metadata_replacement_is_conditional_and_validates_its_committed_generation() {
    for changed in [false, true] {
        let (endpoint, task) = server(vec![
            Reply {
                status: 200,
                headers: vec![("ETag", "old"), ("x-amz-storage-class", "STANDARD")],
                body: "abc".into(),
            },
            xml("<CopyObjectResult><ETag>new</ETag></CopyObjectResult>"),
            Reply {
                status: 200,
                headers: vec![
                    ("ETag", if changed { "foreign" } else { "new" }),
                    ("x-amz-meta-custom", "value"),
                ],
                body: "abc".into(),
            },
        ]);
        let c = config(&endpoint, &["--sse", "--storage-class=STANDARD_IA"]);
        let s = store(&endpoint, &c);
        let result = s
            .replace_metadata(
                "file",
                "old",
                HashMap::from([("custom".into(), "value".into())]),
            )
            .await;
        if changed {
            assert!(matches!(result, Err(Error::Fs(libc::ESTALE))));
        } else {
            let head = result.unwrap();
            assert_eq!(head.etag, "new");
            assert_eq!(head.size, 3);
            assert_eq!(head.metadata["custom"], "value");
        }
        let requests = task.join().unwrap();
        assert_eq!(requests[1].headers["x-amz-copy-source-if-match"], "old");
        assert_eq!(requests[1].headers["x-amz-metadata-directive"], "REPLACE");
        assert_eq!(requests[1].headers["x-amz-meta-custom"], "value");
        assert_eq!(
            requests[1].headers["x-amz-server-side-encryption"],
            "AES256"
        );
        assert_eq!(requests[1].headers["x-amz-storage-class"], "STANDARD");
    }
}

#[tokio::test]
async fn requester_pays_is_signed_on_put_parts_copy_and_all_delete_phases() {
    let (endpoint, task) = server(vec![
        head(),
        xml(
            "<InitiateMultipartUploadResult><UploadId>owned</UploadId></InitiateMultipartUploadResult>",
        ),
        head(),
        xml("<CopyPartResult><ETag>copied</ETag></CopyPartResult>"),
        Reply {
            status: 204,
            headers: vec![],
            body: String::new(),
        },
        Reply {
            status: 204,
            headers: vec![],
            body: String::new(),
        },
    ]);
    let c = config(&endpoint, &["--requester-pays", "--no-checksum"]);
    let s = store(&endpoint, &c);
    s.put("file", ByteStream::from_static(b"abc"), HashMap::new())
        .await
        .unwrap();
    let upload = s.begin_upload("file", HashMap::new(), 6).await.unwrap();
    s.upload_part("file", &upload, 1, ByteStream::from_static(b"abc"))
        .await
        .unwrap();
    s.copy_part("file", &upload, 2, "source", 0..=2, "old")
        .await
        .unwrap();
    s.abort_upload("file", &upload).await.unwrap();
    s.delete("file").await.unwrap();
    for r in task.join().unwrap() {
        assert_eq!(
            r.headers.get("x-amz-request-payer").map(String::as_str),
            Some("requester"),
            "{} {}",
            r.method,
            r.url
        );
        assert!(r.headers["authorization"].contains("x-amz-request-payer"));
    }
}

#[tokio::test]
async fn replacing_user_metadata_preserves_http_object_properties_in_copy_and_multipart() {
    for multipart in [false, true] {
        let properties = vec![
            ("Content-Type", "application/x-kept"),
            ("Content-Encoding", "gzip"),
            ("Content-Disposition", "attachment; filename=original.bin"),
            ("Content-Language", "fr"),
            ("Cache-Control", "max-age=123"),
            ("Expires", "Mon, 02 Jan 2023 00:00:00 GMT"),
            ("x-amz-website-redirect-location", "/kept"),
            ("Last-Modified", "Sun, 01 Jan 2023 00:00:00 GMT"),
        ];
        let size = if multipart { 6 * 1024 * 1024 } else { 3 };
        let mut headers = properties.clone();
        headers.push(("ETag", "old"));
        let mut replies = vec![Reply {
            status: 200,
            headers,
            body: "x".repeat(size),
        }];
        if multipart {
            replies.extend([xml("<InitiateMultipartUploadResult><UploadId>owned</UploadId></InitiateMultipartUploadResult>"),xml("<CopyPartResult><ETag>one</ETag></CopyPartResult>"),xml("<CopyPartResult><ETag>two</ETag></CopyPartResult>"),xml("<CompleteMultipartUploadResult><ETag>new</ETag></CompleteMultipartUploadResult>")]);
        } else {
            replies.push(xml("<CopyObjectResult><ETag>new</ETag></CopyObjectResult>"));
        }
        let mut headers = properties.clone();
        headers.push(("ETag", "new"));
        replies.push(Reply {
            status: 200,
            headers,
            body: "x".repeat(size),
        });
        let (endpoint, task) = server(replies);
        let c = config(
            &endpoint,
            if multipart {
                &[
                    "--multipart-copy-threshold=1",
                    "--max-parallel-parts=1",
                    "--use-content-type",
                ]
            } else {
                &["--use-content-type"]
            },
        );
        let s = store(&endpoint, &c);
        let head = s
            .replace_metadata(
                "file.txt",
                "old",
                HashMap::from([("custom".into(), "new".into())]),
            )
            .await
            .unwrap();
        assert_eq!(
            head.last_modified,
            Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1672531200))
        );
        let requests = task.join().unwrap();
        for (name, value) in properties
            .into_iter()
            .filter(|(n, _)| *n != "Last-Modified")
        {
            assert_eq!(
                requests[1]
                    .headers
                    .get(&name.to_ascii_lowercase())
                    .map(String::as_str),
                Some(value),
                "property {name}, multipart={multipart}"
            );
        }
    }
}

#[tokio::test]
async fn no_checksum_uses_sdk_unsigned_payload_without_optional_crc() {
    let (endpoint, task) = server(vec![head()]);
    let c = config(&endpoint, &["--no-checksum", "--sse"]);
    let s = store(&endpoint, &c);
    s.put("file", ByteStream::from_static(b"abc"), HashMap::new())
        .await
        .unwrap();
    let r = task.join().unwrap().remove(0);
    assert_eq!(r.headers["x-amz-content-sha256"], "UNSIGNED-PAYLOAD");
    assert!(!r.headers.contains_key("x-amz-checksum-crc32"));
    assert_eq!(r.headers["x-amz-server-side-encryption"], "AES256");
    assert_eq!(r.body, b"abc");
}

#[tokio::test]
async fn new_multipart_upload_still_applies_guessed_content_type_when_no_property_was_supplied() {
    let (endpoint, task) = server(vec![xml(
        "<InitiateMultipartUploadResult><UploadId>new</UploadId></InitiateMultipartUploadResult>",
    )]);
    let c = config(&endpoint, &["--use-content-type"]);
    let s = store(&endpoint, &c);
    s.begin_upload("new.txt", HashMap::new(), 6 * 1024 * 1024)
        .await
        .unwrap();
    assert!(task.join().unwrap()[0].headers["content-type"].starts_with("text/plain"));
}

#[tokio::test]
async fn multipart_expiry_pages_are_confined_and_only_old_uploads_are_aborted() {
    let (endpoint, task) = server(vec![
        xml(
            "<ListMultipartUploadsResult><IsTruncated>true</IsTruncated><NextKeyMarker>prefix/a</NextKeyMarker><NextUploadIdMarker>old</NextUploadIdMarker><Upload><Key>prefix/a</Key><UploadId>old</UploadId><Initiated>2000-01-01T00:00:00Z</Initiated></Upload><Upload><Key>prefix/new</Key><UploadId>fresh</UploadId><Initiated>2099-01-01T00:00:00Z</Initiated></Upload></ListMultipartUploadsResult>",
        ),
        Reply {
            status: 204,
            headers: vec![],
            body: String::new(),
        },
        xml(
            "<ListMultipartUploadsResult><IsTruncated>false</IsTruncated><Upload><Key>prefix/z</Key><UploadId>old-two</UploadId><Initiated>2000-01-01T00:00:00Z</Initiated></Upload></ListMultipartUploadsResult>",
        ),
        Reply {
            status: 204,
            headers: vec![],
            body: String::new(),
        },
    ]);
    let c = config(&endpoint, &[]);
    let s = store(&endpoint, &c);
    assert_eq!(s.expire_multipart(SystemTime::now()).await.unwrap(), 2);
    let r = task.join().unwrap();
    assert_eq!(r.len(), 4);
    assert_eq!(r[1].method, "DELETE");
    assert!(r[1].url.contains("uploadId=old"));
    assert!(r[2].url.contains("key-marker=prefix%2Fa"));
    assert!(r[2].url.contains("upload-id-marker=old"));
    assert!(r[0].url.contains("prefix=prefix%2F"));
    let (endpoint, task) = server(vec![xml(
        "<ListMultipartUploadsResult><Upload><Key>other/not-owned</Key><UploadId>x</UploadId><Initiated>2000-01-01T00:00:00Z</Initiated></Upload></ListMultipartUploadsResult>",
    )]);
    let c = config(&endpoint, &[]);
    assert!(
        store(&endpoint, &c)
            .expire_multipart(SystemTime::now())
            .await
            .is_err()
    );
    assert_eq!(task.join().unwrap().len(), 1);
    let (endpoint, task) = server(vec![]);
    let c = config(&endpoint, &["--no-expire-multipart"]);
    assert_eq!(
        store(&endpoint, &c)
            .expire_multipart(SystemTime::now())
            .await
            .unwrap(),
        0
    );
    assert!(task.join().unwrap().is_empty());
}

#[tokio::test]
async fn conditional_patch_uses_signed_extension_range_owned_body_and_complete_metadata() {
    let (endpoint, task) = server(vec![xml(
        "<PatchObjectResult><Object><ETag>patched</ETag><LastModified>2026-01-01T00:00:00Z</LastModified></Object></PatchObjectResult>",
    )]);
    let c = config(&endpoint, &[]);
    let s = store(&endpoint, &c);
    let budget = Budget::new(32).unwrap();
    let body = budget.reserve(3).await.unwrap().copy(b"abc").unwrap();
    let result = s
        .patch("file", 7, body, "generation", Some(8388608))
        .await
        .unwrap();
    assert_eq!(result.etag, "patched");
    assert_eq!(result.last_modified.secs(), 1767225600);
    let r = task.join().unwrap().remove(0);
    assert_eq!(r.method, "PATCH");
    assert_eq!(r.body, b"abc");
    assert_eq!(r.headers["content-range"], "bytes 7-9/*");
    assert_eq!(r.headers["if-match"], "generation");
    assert_eq!(r.headers["x-yc-s3-patch-append-part-size"], "8388608");
    assert!(r.url.contains("X-Amz-Signature="));
    assert_eq!(budget.used(), 0);
}

#[tokio::test]
async fn patch_errors_are_not_retried_and_notimplemented_is_the_only_unsupported_mapping() {
    for (code, status) in [
        ("NotImplemented", 501),
        ("AccessDenied", 403),
        ("InternalError", 500),
    ] {
        let mut reply = xml(&format!("<Error><Code>{code}</Code></Error>"));
        reply.status = status;
        let (endpoint, task) = server(vec![reply]);
        let c = config(&endpoint, &[]);
        let s = store(&endpoint, &c);
        let budget = Budget::new(3).unwrap();
        let body = budget.reserve(3).await.unwrap().copy(b"abc").unwrap();
        let error = s
            .patch("file", 0, body, "generation", None)
            .await
            .unwrap_err();
        if code == "NotImplemented" {
            assert!(matches!(error, Error::Fs(libc::ENOSYS)));
        } else {
            assert!(matches!(error,Error::Backend{code:ref actual,..} if actual==code));
        }
        assert_eq!(task.join().unwrap().len(), 1);
        assert_eq!(budget.used(), 0);
    }
}
#[tokio::test]
async fn patch_never_publishes_metadata_from_wrong_roots_or_trailing_documents() {
    for response in [
        "<Wrong><Object><ETag>new</ETag><LastModified>2026-01-01T00:00:00Z</LastModified></Object></Wrong>",
        "<PatchObjectResult><Object><ETag>new</ETag><LastModified>2026-01-01T00:00:00Z</LastModified></Object></PatchObjectResult><Error/>",
        "<!DOCTYPE PatchObjectResult><PatchObjectResult><Object><ETag>new</ETag><LastModified>2026-01-01T00:00:00Z</LastModified></Object></PatchObjectResult>",
    ] {
        let (endpoint, task) = server(vec![xml(response)]);
        let c = config(&endpoint, &[]);
        let s = store(&endpoint, &c);
        let budget = Budget::new(3).unwrap();
        let body = budget.reserve(3).await.unwrap().copy(b"abc").unwrap();
        let result = s.patch("file", 0, body, "old", None).await;
        assert!(
            matches!(result, Err(Error::InvalidResponse(_))),
            "must reject malformed success metadata: {result:?}"
        );
        assert_eq!(budget.used(), 0);
        assert_eq!(task.join().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn multipart_copy_preserves_source_generation_and_aborts_failed_owned_upload() {
    for fail in [false, true] {
        let mut replies = vec![
            Reply {
                status: 200,
                headers: vec![("ETag", "old"), ("x-amz-meta-example", "retained")],
                body: "X".repeat(6 * 1024 * 1024),
            },
            xml(
                "<InitiateMultipartUploadResult><UploadId>copy-owned</UploadId></InitiateMultipartUploadResult>",
            ),
        ];
        if fail {
            replies.push(Reply {
                status: 403,
                headers: vec![],
                body: "<Error><Code>AccessDenied</Code></Error>".into(),
            });
            replies.push(Reply {
                status: 204,
                headers: vec![],
                body: String::new(),
            });
        } else {
            replies.extend([xml("<CopyPartResult><ETag>one</ETag></CopyPartResult>"),xml("<CopyPartResult><ETag>two</ETag></CopyPartResult>"),xml("<CompleteMultipartUploadResult><ETag>committed</ETag></CompleteMultipartUploadResult>")]);
        }
        let (endpoint, task) = server(replies);
        let c = config(
            &endpoint,
            &[
                "--multipart-copy-threshold=1",
                "--sse",
                "--max-parallel-parts=1",
            ],
        );
        let s = store(&endpoint, &c);
        let result = s.copy("source", "destination", "old").await;
        assert_eq!(result.is_err(), fail, "copy result: {result:?}");
        s.drain_cleanup().await.unwrap();
        let r = task.join().unwrap();
        assert_eq!(r[0].method, "HEAD");
        assert_eq!(r[1].headers["x-amz-meta-example"], "retained");
        assert_eq!(r[1].headers["x-amz-server-side-encryption"], "AES256");
        assert_eq!(r[2].headers["x-amz-copy-source-if-match"], "old");
        assert_eq!(r[2].headers["x-amz-copy-source-range"], "bytes=0-5242879");
        if fail {
            assert_eq!(r[3].method, "DELETE");
            assert!(r[3].url.contains("uploadId=copy-owned"));
        } else {
            assert_eq!(
                r[3].headers["x-amz-copy-source-range"],
                "bytes=5242880-6291455"
            );
            assert!(String::from_utf8_lossy(&r[4].body).contains("<ETag>two</ETag>"));
        }
        assert!(
            r.iter()
                .all(|request| !request.url.ends_with("/source") || request.method == "HEAD"),
            "copy never deletes its source"
        );
    }
}

#[tokio::test]
async fn backend_copy_runs_configured_parts_concurrently_and_completes_in_part_order() {
    let listener = Server::http("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.server_addr());
    let task = std::thread::spawn(move || {
        let request = listener
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        assert_eq!(request.method().as_str(), "HEAD");
        request
            .respond(
                Response::from_data(vec![0; 6 * 1024 * 1024])
                    .with_header(Header::from_bytes("ETag", "old").unwrap())
                    .with_chunked_threshold(usize::MAX),
            )
            .unwrap();
        let request = listener
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        assert_eq!(request.method().as_str(), "POST");
        request.respond(Response::from_string("<InitiateMultipartUploadResult><UploadId>parallel</UploadId></InitiateMultipartUploadResult>")).unwrap();
        let first = listener
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .expect("first copy part");
        let second = listener
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .expect("second copy must start before first response");
        assert_eq!(first.method().as_str(), "PUT");
        assert_eq!(second.method().as_str(), "PUT");
        let mut numbers = Vec::new();
        for request in [second, first] {
            let number = if request.url().contains("partNumber=1") {
                1
            } else if request.url().contains("partNumber=2") {
                2
            } else {
                panic!("unexpected copy part URL")
            };
            numbers.push(number);
            request
                .respond(Response::from_string(format!(
                    "<CopyPartResult><ETag>part-{number}</ETag></CopyPartResult>"
                )))
                .unwrap();
        }
        numbers.sort_unstable();
        assert_eq!(numbers, [1, 2]);
        let mut request = listener
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        assert_eq!(request.method().as_str(), "POST");
        let mut body = String::new();
        request.as_reader().read_to_string(&mut body).unwrap();
        assert!(
            body.find("<PartNumber>1</PartNumber>").unwrap()
                < body.find("<PartNumber>2</PartNumber>").unwrap()
        );
        request
            .respond(Response::from_string(
                "<CompleteMultipartUploadResult><ETag>done</ETag></CompleteMultipartUploadResult>",
            ))
            .unwrap();
    });
    let mut c = config(
        &endpoint,
        &["--multipart-copy-threshold=1", "--max-parallel-parts=2"],
    );
    c.http_timeout = Duration::from_secs(10);
    let s = store(&endpoint, &c);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(15), s.copy("source", "target", "old"))
            .await
            .unwrap()
            .unwrap(),
        "done"
    );
    task.join().unwrap();
}

#[tokio::test]
async fn configured_large_single_put_streams_real_sdk_bytes_and_valid_checksum_trailers() {
    use base64::Engine;
    for no_checksum in [false, true] {
        let missing = || Reply {
            status: 404,
            headers: vec![],
            body: String::new(),
        };
        let size = 12 * 1024 * 1024;
        let (endpoint, task) = server(vec![
            missing(),
            missing(),
            xml("<ListBucketResult><IsTruncated>false</IsTruncated></ListBucketResult>"),
            Reply {
                status: 200,
                headers: vec![("ETag", "streamed")],
                body: String::new(),
            },
            Reply {
                status: 200,
                headers: vec![("ETag", "streamed")],
                body: "x".repeat(size),
            },
        ]);
        let mut c = config(
            &endpoint,
            if no_checksum { &["--no-checksum"] } else { &[] },
        );
        c.memory_limit = 16;
        c.single_part = 16;
        c.http_timeout = Duration::from_secs(10);
        let s = store(&endpoint, &c);
        let fs = geesefs_rs::filesystem::Filesystem::new(c, std::sync::Arc::new(s)).unwrap();
        let node = fs
            .create(1, "file", libc::S_IFREG | 0o644, None, 0)
            .await
            .unwrap();
        fs.truncate(node.ino, size as u64).unwrap();
        fs.write(node.ino, size as u64 - 3, b"end").await.unwrap();
        tokio::time::timeout(Duration::from_secs(15), fs.fsync(node.ino))
            .await
            .unwrap()
            .unwrap();
        let requests = task.join().unwrap();
        let request = &requests[3];
        assert_eq!(request.method, "PUT");
        let decoded = if no_checksum {
            assert_eq!(request.headers["content-length"], size.to_string());
            request.body.clone()
        } else {
            assert_eq!(
                request.headers["x-amz-decoded-content-length"],
                size.to_string()
            );
            assert_eq!(request.headers["x-amz-trailer"], "x-amz-checksum-crc32");
            let mut decoded = Vec::new();
            let mut rest = request.body.as_slice();
            loop {
                let end = rest
                    .windows(2)
                    .position(|w| w == b"\r\n")
                    .expect("chunk header CRLF");
                let header = std::str::from_utf8(&rest[..end])
                    .unwrap()
                    .split(';')
                    .next()
                    .unwrap();
                let length = usize::from_str_radix(header, 16).unwrap();
                rest = &rest[end + 2..];
                if length == 0 {
                    break;
                }
                decoded.extend_from_slice(&rest[..length]);
                assert_eq!(&rest[length..length + 2], b"\r\n");
                rest = &rest[length + 2..];
            }
            let checksum = base64::engine::general_purpose::STANDARD
                .encode(crc32fast::hash(&decoded).to_be_bytes());
            assert!(
                std::str::from_utf8(rest)
                    .unwrap()
                    .contains(&format!("x-amz-checksum-crc32:{checksum}"))
            );
            decoded
        };
        assert_eq!(decoded.len(), size);
        assert!(decoded[..size - 3].iter().all(|b| *b == 0));
        assert_eq!(&decoded[size - 3..], b"end");
    }
}

#[tokio::test]
async fn changed_source_is_rejected_before_any_copy_mutation() {
    let (endpoint, task) = server(vec![head()]);
    let c = config(&endpoint, &[]);
    let s = store(&endpoint, &c);
    assert!(matches!(
        s.copy("source", "target", "older").await,
        Err(Error::Fs(libc::ESTALE))
    ));
    assert_eq!(task.join().unwrap().len(), 1);
}

#[tokio::test]
async fn extension_retry_count_is_configured_and_access_errors_are_never_retried() {
    let (endpoint, task) = server(vec![
        Reply {
            status: 500,
            headers: vec![],
            body: "<Error><Code>InternalError</Code></Error>".into(),
        },
        xml("<ListBucketResult><IsTruncated>false</IsTruncated></ListBucketResult>"),
    ]);
    let mut c = config(&endpoint, &["--list-type=ext-v1"]);
    c.sdk_max_retries = 1;
    let s = store(&endpoint, &c);
    assert!(s.list("", None, None).await.unwrap().objects.is_empty());
    assert_eq!(task.join().unwrap().len(), 2);
    let (endpoint, task) = server(vec![Reply {
        status: 403,
        headers: vec![],
        body: "<Error><Code>AccessDenied</Code></Error>".into(),
    }]);
    let mut c = config(&endpoint, &["--list-type=ext-v1"]);
    c.sdk_max_retries = 1;
    assert!(
        matches!(store(&endpoint,&c).list("",None,None).await,Err(Error::Backend{code,..})if code=="AccessDenied")
    );
    assert_eq!(task.join().unwrap().len(), 1);
}
#[tokio::test]
async fn cancelled_backend_copy_aborts_the_known_upload_before_cleanup_drain_returns() {
    let server = Server::http("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", server.server_addr());
    let (entered, seen) = tokio::sync::oneshot::channel();
    let task = std::thread::spawn(move || {
        let request = server
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        assert_eq!(request.method().as_str(), "HEAD");
        request
            .respond(
                Response::from_data(vec![0; 6 * 1024 * 1024])
                    .with_chunked_threshold(usize::MAX)
                    .with_header(Header::from_bytes("ETag", "old").unwrap()),
            )
            .unwrap();
        let request = server
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        request.respond(Response::from_string("<InitiateMultipartUploadResult><UploadId>cancel-owned</UploadId></InitiateMultipartUploadResult>")).unwrap();
        let pending = server
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        assert!(pending.url().contains("partNumber=1"));
        entered.send(()).unwrap();
        let abort = server
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .expect("cancelled copy did not abort its owned upload");
        assert_eq!(abort.method().as_str(), "DELETE");
        assert!(abort.url().contains("uploadId=cancel-owned"));
        abort.respond(Response::empty(204)).unwrap();
        drop(pending);
    });
    let c = config(
        &endpoint,
        &["--multipart-copy-threshold=1", "--max-parallel-parts=1"],
    );
    let s = store(&endpoint, &c);
    let copying = s.clone();
    let copy = tokio::spawn(async move { copying.copy("source", "destination", "old").await });
    tokio::time::timeout(Duration::from_secs(3), seen)
        .await
        .unwrap()
        .unwrap();
    copy.abort();
    assert!(copy.await.unwrap_err().is_cancelled());
    s.drain_cleanup().await.unwrap();
    task.join().unwrap();
}
#[tokio::test]
async fn sigv2_uses_public_crate_signing_with_an_independent_fixed_wire_vector_and_extensions() {
    let (endpoint, task) = server(vec![
        head(),
        xml("<ListBucketResult><IsTruncated>false</IsTruncated></ListBucketResult>"),
        xml(
            "<PatchObjectResult><Object><ETag>patched</ETag><LastModified>2026-01-01T00:00:00Z</LastModified></Object></PatchObjectResult>",
        ),
    ]);
    let c = config(
        &endpoint,
        &[
            "--signature-version=2",
            "--no-checksum",
            "--list-type=ext-v1",
        ],
    );
    let clock = Clock(std::sync::Arc::new(std::sync::atomic::AtomicU64::new(
        1767225600,
    )));
    let s = store_at(&endpoint, &c, Some(clock));
    s.head("file").await.unwrap();
    s.list("", None, None).await.unwrap();
    let budget = Budget::new(3).unwrap();
    s.patch(
        "file",
        0,
        budget.reserve(3).await.unwrap().copy(b"abc").unwrap(),
        "old",
        None,
    )
    .await
    .unwrap();
    let r = task.join().unwrap();
    // Independently calculated HMAC-SHA1 over the explicit example:
    // HEAD + four newlines + x-amz-date line + /bucket/prefix/file.
    assert_eq!(r[0].headers["x-amz-date"], "Thu, 01 Jan 2026 00:00:00 GMT");
    assert_eq!(
        r[0].headers["authorization"], "AWS disposable-access:sXdfke2lzsEnGkZtQGKlYSTCNME=",
        "{:?}",
        r[0]
    );
    for request in &r {
        assert!(request.headers["authorization"].starts_with("AWS disposable-access:"));
        assert!(!request.url.contains("X-Amz-Signature="));
    }
    assert!(r[1].url.contains("list-type=ext-v1"));
    assert_eq!(r[2].method, "PATCH");
    assert_eq!(budget.used(), 0);
}

#[tokio::test]
async fn anonymous_mode_and_no_detect_have_real_transport_semantics() {
    let (endpoint, task) = server(vec![
        head(),
        xml("<ListBucketResult><IsTruncated>false</IsTruncated></ListBucketResult>"),
    ]);
    let c = config(
        &endpoint,
        &["--signature-version=anonymous", "--list-type=ext-v1"],
    );
    let s = store(&endpoint, &c).initialize().await.unwrap();
    s.head("file").await.unwrap();
    s.list("", None, None).await.unwrap();
    for r in task.join().unwrap() {
        assert!(!r.headers.contains_key("authorization"));
        assert!(!r.url.contains("X-Amz-Credential"));
    }
    let (endpoint, task) = server(vec![head()]);
    let c = config(&endpoint, &["--no-detect"]);
    let s = store(&endpoint, &c).initialize().await.unwrap();
    s.head("file").await.unwrap();
    let r = task.join().unwrap();
    assert_eq!(r.len(), 1);
    assert!(r[0].url.contains("/prefix/file"));
    assert!(r[0].headers["authorization"].starts_with("AWS4-HMAC-SHA256"));
}

#[tokio::test]
async fn initialization_falls_back_once_to_sigv2_and_uses_detected_region_without_anonymous_downgrade()
 {
    let (endpoint, task) = server(vec![
        Reply {
            status: 403,
            headers: vec![],
            body: String::new(),
        },
        Reply {
            status: 404,
            headers: vec![],
            body: String::new(),
        },
        head(),
    ]);
    let c = config(&endpoint, &["--region=us-east-1"]);
    let s = store(&endpoint, &c).initialize().await.unwrap();
    s.head("file").await.unwrap();
    let r = task.join().unwrap();
    assert_eq!(r.len(), 3);
    assert!(r[0].headers["authorization"].starts_with("AWS4-HMAC-SHA256"));
    for request in &r[1..] {
        assert!(request.headers["authorization"].starts_with("AWS disposable-access:"));
    }
    let (endpoint, task) = server(vec![
        Reply {
            status: 200,
            headers: vec![("x-amz-bucket-region", "eu-west-1")],
            body: String::new(),
        },
        head(),
        head(),
    ]);
    let mut c = config(&endpoint, &[]);
    c.profile = Some("fixture".into());
    let s = store(&endpoint, &c).initialize().await.unwrap();
    s.head("file").await.unwrap();
    let r = task.join().unwrap();
    assert!(!r[0].headers.contains_key("authorization"));
    assert!(r[1].headers["authorization"].contains("/eu-west-1/s3/aws4_request"));
    assert!(r[2].headers["authorization"].contains("/eu-west-1/s3/aws4_request"));
}
#[tokio::test]
async fn insecure_tls_is_explicit_and_streams_raw_gzip_sdk_and_extension_bodies() {
    use std::io::Write;
    let certified = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let server = Server::https(
        "127.0.0.1:0",
        tiny_http::SslConfig {
            certificate: certified.cert.pem().into_bytes(),
            private_key: certified.signing_key.serialize_pem().into_bytes(),
        },
    )
    .unwrap();
    let endpoint = format!("https://{}", server.server_addr()).replace("127.0.0.1", "localhost");
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(b"stored compressed bytes").unwrap();
    let stored = encoder.finish().unwrap();
    let raw = stored.clone();
    let task = std::thread::spawn(move || {
        for i in 0..5 {
            let mut request = server
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .expect("missing insecure-TLS request");
            assert!(
                request
                    .headers()
                    .iter()
                    .any(|h| h.field.equiv("Authorization"))
                    || request.url().contains("X-Amz-Signature=")
            );
            let response = match i {
                0 => Response::from_data(raw.clone())
                    .with_header(Header::from_bytes("ETag", "tls").unwrap()),
                1 => Response::from_data(raw.clone())
                    .with_status_code(206)
                    .with_header(Header::from_bytes("Content-Encoding", "gzip").unwrap())
                    .with_header(
                        Header::from_bytes(
                            "Content-Range",
                            format!("bytes 0-{}/{}", raw.len() - 1, raw.len()),
                        )
                        .unwrap(),
                    ),
                2 => {
                    let mut body = Vec::new();
                    request.as_reader().read_to_end(&mut body).unwrap();
                    assert_eq!(body, b"abc");
                    Response::from_data(Vec::new())
                        .with_header(Header::from_bytes("ETag", "written").unwrap())
                }
                3 => Response::from_data(
                    b"<ListBucketResult><IsTruncated>false</IsTruncated></ListBucketResult>"
                        .to_vec(),
                ),
                _ => {
                    let mut body = Vec::new();
                    request.as_reader().read_to_end(&mut body).unwrap();
                    assert_eq!(body, b"xyz");
                    Response::from_data(b"<PatchObjectResult><Object><ETag>patched</ETag><LastModified>2026-01-01T00:00:00Z</LastModified></Object></PatchObjectResult>".to_vec())
                }
            };
            request.respond(response).unwrap();
        }
    });
    let strict = config(&endpoint, &[]);
    assert!(
        store(&endpoint, &strict).head("file").await.is_err(),
        "self-signed certificate must not be trusted by default"
    );
    let c = config(&endpoint, &["--no-verify-ssl", "--list-type=ext-v1"]);
    let s = store(&endpoint, &c);
    let head = s.head("file").await.unwrap();
    assert_eq!(head.size, stored.len() as u64);
    let budget = Budget::new(128).unwrap();
    let data = s
        .read_range("file", 0, stored.len(), &head, &budget)
        .await
        .unwrap();
    assert_eq!(data.as_slice(), stored);
    drop(data);
    s.put("file", ByteStream::from_static(b"abc"), HashMap::new())
        .await
        .unwrap();
    assert!(s.list("", None, None).await.unwrap().objects.is_empty());
    let result = s
        .patch(
            "file",
            0,
            budget.reserve(3).await.unwrap().copy(b"xyz").unwrap(),
            "tls",
            None,
        )
        .await
        .unwrap();
    assert_eq!(result.etag, "patched");
    assert_eq!(budget.used(), 0);
    task.join().unwrap();
}
