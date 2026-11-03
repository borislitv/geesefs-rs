#![forbid(unsafe_code)]
mod support;

use geesefs_rs::{config::Config, filesystem::Filesystem};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tiny_http::{Response, Server};

async fn rejected_before_backend(memory: usize) {
    let server = Arc::new(Server::http("127.0.0.1:0").unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let receiver = std::thread::spawn({
        let server = server.clone();
        let requests = requests.clone();
        let stop = stop.clone();
        move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !stop.load(Ordering::Acquire) && Instant::now() < deadline {
                if let Some(request) = server.recv_timeout(Duration::from_millis(100)).unwrap() {
                    let method = request.method().to_string();
                    requests
                        .lock()
                        .unwrap()
                        .push((method.clone(), request.url().to_owned()));
                    // Faithful cleanup responses let the unfixed binary reach its
                    // late budget error, rather than fail on an unrelated HTTP error.
                    let response = if method == "GET" {
                        Response::from_string(concat!(
                            "<ListMultipartUploadsResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">",
                            "<Bucket>bucket</Bucket><IsTruncated>false</IsTruncated><Upload>",
                            "<Key>pending</Key><UploadId>fixture-upload</UploadId>",
                            "<Initiated>2020-01-01T00:00:00Z</Initiated></Upload>",
                            "</ListMultipartUploadsResult>"
                        ))
                    } else {
                        Response::from_string("").with_status_code(204)
                    };
                    request.respond(response).unwrap();
                }
            }
        }
    });
    let directory = tempfile::tempdir().unwrap();
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_geesefs-rs"));
    child.env_clear().kill_on_drop(true).args([
        "-f",
        "--no-detect",
        "--signature-version=anonymous",
        "--sdk-max-retries=0",
        "--http-timeout=2s",
        &format!("--endpoint=http://{}", server.server_addr()),
        &format!("--memory-limit={memory}"),
        "-o",
        "ro",
        "bucket",
        directory.path().to_str().unwrap(),
    ]);
    let output = tokio::time::timeout(Duration::from_secs(5), child.output()).await;
    stop.store(true, Ordering::Release);
    server.unblock();
    receiver.join().unwrap();
    let output = output
        .expect("invalid startup must finish promptly")
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("memory-limit") || stderr.contains("byte budget"),
        "{stderr}"
    );
    let requests = requests.lock().unwrap();
    assert!(
        requests.is_empty(),
        "invalid budgets must cause no backend requests: {:?}",
        *requests
    );
}

#[tokio::test]
async fn insufficient_positive_memory_is_rejected_before_startup_cleanup() {
    rejected_before_backend(1).await;
}

#[tokio::test]
async fn unrepresentable_byte_lanes_are_rejected_before_startup_cleanup() {
    rejected_before_backend(usize::MAX / (1024 * 1024)).await;
}

#[test]
fn budget_validation_and_constructor_agree_at_supported_boundaries() {
    for (memory, valid) in [
        (0, false),
        (1, false),
        (14, false),
        (15, true),
        (16, true),
        (32768, true),
    ] {
        let config = support::MemoryStore::config(memory);
        assert_eq!(
            config.validate().is_ok(),
            valid,
            "validation of {memory} MiB"
        );
        assert_eq!(
            Filesystem::new(config, support::MemoryStore::new()).is_ok(),
            valid,
            "construction of {memory} MiB"
        );
    }
    let config: Config = support::MemoryStore::config(16);
    let fs = Filesystem::new(config, support::MemoryStore::new()).unwrap();
    let budgets = fs.effective_budgets();
    assert_eq!(budgets.data_shared_bytes, 5 * 1024 * 1024);
    assert_eq!(budgets.ingress_bytes, 1024 * 1024);
    assert_eq!(budgets.flush_bytes, 10 * 1024 * 1024);
    assert_eq!(budgets.payload_limit_bytes, 16 * 1024 * 1024);
}
