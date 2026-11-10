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

#[tokio::test]
async fn disk_cache_invalid_configuration_never_contacts_backend_or_mounts() {
    let root = tempfile::tempdir().unwrap();
    let mount = root.path().join("mount");
    let cache = root.path().join("cache");
    let stats = root.path().join("stats.json");
    std::fs::create_dir(&mount).unwrap();
    std::fs::create_dir(&cache).unwrap();
    let inside = mount.join("inside");
    std::fs::create_dir(&inside).unwrap();
    let alias = root.path().join("mount-alias");
    std::os::unix::fs::symlink(&mount, &alias).unwrap();
    let file = root.path().join("not-a-directory");
    std::fs::write(&file, b"fixture").unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let complete = vec![
        "--disk-cache-dir".to_owned(),
        cache.to_str().unwrap().to_owned(),
        "--disk-cache-max-size".into(),
        "1 TiB".into(),
        "--disk-cache-regex".into(),
        r"\.(deb|rpm)$".into(),
    ];
    let mut cases = Vec::new();
    for mask in 1..7 {
        let mut partial = Vec::new();
        for bit in 0..3 {
            if mask & (1 << bit) != 0 {
                partial.extend_from_slice(&complete[bit * 2..bit * 2 + 2]);
            }
        }
        cases.push(partial);
    }
    for (index, value) in [
        (1, mount.to_str().unwrap()),
        (1, root.path().to_str().unwrap()),
        (1, inside.to_str().unwrap()),
        (1, alias.to_str().unwrap()),
        (1, file.to_str().unwrap()),
        (1, "/nonexistent-geesefs-cache-fixture"),
        (3, "0"),
        (3, "18446744073709551616 B"),
        (3, "invalid"),
        (3, "1 MiB"),
        (3, "8388609 B"),
        (3, "17 TiB"),
        (5, "["),
        (5, r"(deb)\1"),
        (5, "(?=deb)"),
    ] {
        let mut args = complete.clone();
        args[index] = value.into();
        cases.push(args);
    }
    let mut oversized = complete.clone();
    oversized[5] = "a".repeat(16 * 1024 + 1);
    cases.push(oversized);
    for option in [
        "--disk-cache-ttl=1h",
        "--disk-cache-inactivity=1h",
        "--disk-cache-workers=2",
        "--disk-cache-queue=2",
        "--disk-cache-write-mode=always",
        "--disk-cache-config=cache.yaml",
    ] {
        let mut args = complete.clone();
        args.push(option.into());
        cases.push(args);
    }
    for (index, args) in cases.iter().enumerate() {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_geesefs-rs"));
        command
            .env_clear()
            .env("HOME", root.path())
            .env("AWS_EC2_METADATA_DISABLED", "true")
            .env("AWS_ACCESS_KEY_ID", "disk-cache-fixture")
            .env("AWS_SECRET_ACCESS_KEY", "disk-cache-fixture-not-a-secret")
            .kill_on_drop(true)
            .args([
                "-f",
                "--no-detect",
                "--signature-version=anonymous",
                "--sdk-max-retries=0",
                "--http-timeout=1s",
                "--request-timeout=1s",
                &format!("--endpoint=http://{}", listener.local_addr().unwrap()),
                "--stats-file",
                stats.to_str().unwrap(),
                "bucket:packages",
                mount.to_str().unwrap(),
            ])
            .args(args);
        let output = tokio::time::timeout(Duration::from_secs(5), command.output()).await;
        assert_eq!(
            listener.accept().err().map(|error| error.kind()),
            Some(std::io::ErrorKind::WouldBlock),
            "case={index}: invalid/unsupported disk cache caused backend contact"
        );
        let output = output
            .expect("disk config must fail before backend initialization")
            .unwrap();
        assert!(!output.status.success(), "case={index}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("disk-cache"), "case={index}: {stderr}");
        assert!(!stats.exists(), "case={index}: statistics side effect");
        assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 0);
        let entries: Vec<_> = std::fs::read_dir(&mount)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(entries, [std::ffi::OsString::from("inside")]);
    }
}

#[tokio::test]
async fn disk_cache_directory_is_locked_before_sdk_initialization() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let cache = root.path().join("cache");
    let mount = root.path().join("mount");
    std::fs::create_dir(&cache).unwrap();
    std::fs::set_permissions(&cache, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::create_dir(&mount).unwrap();
    let _owner =
        geesefs_rs::disk_cache::PreparedDiskCache::prepare(&cache, 16 * 1024 * 1024).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_geesefs-rs"));
    command
        .env_clear()
        .env("HOME", root.path())
        .env("AWS_EC2_METADATA_DISABLED", "true")
        .kill_on_drop(true)
        .args([
            "-f",
            "--no-detect",
            "--signature-version=anonymous",
            "--sdk-max-retries=0",
            "--http-timeout=1s",
            "--request-timeout=1s",
            "--memory-limit=32",
            &format!("--endpoint=http://{}", listener.local_addr().unwrap()),
            "--disk-cache-dir",
            cache.to_str().unwrap(),
            "--disk-cache-max-size=16 MiB",
            r"--disk-cache-regex=\.(deb|rpm)$",
            "bucket",
            mount.to_str().unwrap(),
        ]);
    let output = tokio::time::timeout(Duration::from_secs(5), command.output())
        .await
        .unwrap()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(
        listener.accept().err().map(|error| error.kind()),
        Some(std::io::ErrorKind::WouldBlock)
    );
    assert!(
        !cache.join("payload-v1.cache").exists(),
        "directory exclusion must precede engine preallocation"
    );
    assert!(std::fs::read_dir(&mount).unwrap().next().is_none());
}
