#![forbid(unsafe_code)]
mod support;

use geesefs_rs::{filesystem::Filesystem, fuse::Fuse};
use std::{
    os::unix::net::UnixDatagram,
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

#[global_allocator]
#[cfg(not(feature = "system-allocator"))]
static ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
#[global_allocator]
#[cfg(feature = "system-allocator")]
static ALLOCATOR: &stats_alloc::StatsAlloc<std::alloc::System> = &stats_alloc::INSTRUMENTED_SYSTEM;

/// Exercise fuser's real parsing, reply ownership and Drop without a kernel mount.
struct Session {
    peer: UnixDatagram,
    finished: mpsc::Receiver<std::io::Result<()>>,
    worker: Option<std::thread::JoinHandle<()>>,
    shutdown_error: Arc<Mutex<Option<String>>>,
    sequence: u64,
}

impl Session {
    fn new(fs: Arc<Filesystem>, runtime: &tokio::runtime::Runtime) -> Self {
        let adapter = Fuse::new(fs, runtime.handle().clone());
        let shutdown_error = adapter.shutdown_error.clone();
        let (peer, device) = UnixDatagram::pair().unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        peer.set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let (done, finished) = mpsc::channel();
        let mut fixture = Self {
            peer,
            finished,
            worker: None,
            shutdown_error,
            sequence: 1,
        };
        let init = [7u32, 31, 131072, 0]
            .into_iter()
            .flat_map(u32::to_ne_bytes)
            .collect::<Vec<_>>();
        fixture.send(26, 0, &init);
        let session = fuser::Session::from_fd(
            adapter,
            device.into(),
            fuser::SessionACL::All,
            fuser::Config::default(),
        )
        .unwrap();
        assert_eq!(errno(&fixture.receive()), 0);
        fixture.worker = Some(std::thread::spawn(move || {
            let _ = done.send(session.run());
        }));
        fixture
    }

    fn send(&mut self, opcode: u32, ino: u64, body: &[u8]) -> u64 {
        let unique = self.sequence;
        self.sequence += 1;
        let mut packet = Vec::new();
        packet.extend_from_slice(&(40 + body.len() as u32).to_ne_bytes());
        packet.extend_from_slice(&opcode.to_ne_bytes());
        packet.extend_from_slice(&unique.to_ne_bytes());
        packet.extend_from_slice(&ino.to_ne_bytes());
        packet.extend_from_slice(&[0u8; 16]);
        packet.extend_from_slice(body);
        self.peer.send(&packet).unwrap();
        unique
    }

    fn receive(&self) -> Vec<u8> {
        let mut buffer = vec![0u8; 131072];
        let size = self.peer.recv(&mut buffer).unwrap();
        buffer.truncate(size);
        buffer
    }

    fn close(&mut self) {
        self.send(38, 0, &[]);
        self.finished
            .recv_timeout(Duration::from_secs(3))
            .expect("session destroy must drain admission")
            .unwrap();
        self.worker.take().unwrap().join().unwrap();
        assert!(self.shutdown_error.lock().unwrap().is_none());
        // Exactly one reply remains: DESTROY. A deadline after early READ
        // publication must not enqueue another reply for that READ request.
        assert_eq!(errno(&self.receive()), 0);
        self.peer.set_nonblocking(true).unwrap();
        assert_eq!(
            self.peer.recv(&mut [0u8; 256]).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.worker.is_some() {
            // Keep failed assertions from leaving the fake receiver blocked.
            let _ = self.peer.send(&destroy_packet());
            if self.finished.recv_timeout(Duration::from_secs(3)).is_ok() {
                let _ = self.worker.take().unwrap().join();
            }
        }
    }
}

fn destroy_packet() -> Vec<u8> {
    let mut packet = vec![0u8; 40];
    packet[..4].copy_from_slice(&40u32.to_ne_bytes());
    packet[4..8].copy_from_slice(&38u32.to_ne_bytes());
    packet[8..16].copy_from_slice(&u64::MAX.to_ne_bytes());
    packet
}

fn errno(reply: &[u8]) -> i32 {
    i32::from_ne_bytes(reply[4..8].try_into().unwrap())
}
fn unique(reply: &[u8]) -> u64 {
    u64::from_ne_bytes(reply[8..16].try_into().unwrap())
}

fn fixture() -> (
    tokio::runtime::Runtime,
    Arc<support::MemoryStore>,
    Arc<Filesystem>,
) {
    configured_fixture(|_| {})
}

fn configured_fixture(
    configure: impl FnOnce(&mut geesefs_rs::config::Config),
) -> (
    tokio::runtime::Runtime,
    Arc<support::MemoryStore>,
    Arc<Filesystem>,
) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let store = support::MemoryStore::new();
    let mut config = support::MemoryStore::config(32);
    config.request_timeout = Duration::from_millis(100);
    config.shutdown_timeout = Duration::from_secs(1);
    config.max_requests = 1;
    config.metrics_listen = Some("127.0.0.1:0".parse().unwrap());
    config.large_read_cutoff = 0;
    config.read_ahead_large = 1024;
    config.read_ahead_parallel = 512;
    configure(&mut config);
    let fs = Filesystem::new(config, store.clone()).unwrap();
    (runtime, store, fs)
}

fn metric(fs: &Filesystem, name: &str) -> f64 {
    geesefs_rs::metrics::render(fs)
        .unwrap()
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once(' ')?;
            (key == name).then(|| value.parse().unwrap())
        })
        .unwrap_or_else(|| panic!("missing metric {name}"))
}

#[test]
fn lookup_deadline_returns_etimedout_and_returns_the_request_slot() {
    let (runtime, store, fs) = fixture();
    store.seed("slow", b"independent bytes");
    *store.head_gate.lock().unwrap() = Some(Arc::new(tokio::sync::Semaphore::new(0)));
    let mut session = Session::new(fs.clone(), &runtime);
    let request = session.send(1, 1, b"slow\0");
    let reply = session.receive();
    assert_eq!(unique(&reply), request);
    assert_eq!(
        errno(&reply),
        -libc::ETIMEDOUT,
        "outer FUSE deadline must preserve timeout errno"
    );
    assert_eq!(
        store.active_heads.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    // The wire reply precedes the last local destructors. With one worker this
    // barrier runs after the timed-out poll has returned its admission permit.
    runtime.block_on(runtime.spawn(async {})).unwrap();
    assert_eq!(
        metric(
            &fs,
            "geesefs_fuse_reply_duration_seconds_count{opcode=\"lookup\"}"
        ),
        1.
    );
    assert_eq!(
        metric(
            &fs,
            "geesefs_fuse_replies_total{opcode=\"lookup\",errno=\"ETIMEDOUT\"}"
        ),
        1.
    );
    assert_eq!(
        metric(
            &fs,
            "geesefs_fuse_inflight{opcode=\"lookup\",state=\"reply_pending\"}"
        ),
        0.
    );
    *store.head_gate.lock().unwrap() = None;
    let request = session.send(1, 1, b"slow\0");
    let reply = session.receive();
    assert_eq!(unique(&reply), request);
    assert_eq!(
        errno(&reply),
        0,
        "the single admission slot must have been returned"
    );
    session.close();
}

fn open(session: &mut Session, ino: u64) -> u64 {
    session.send(14, ino, &[0u8; 8]);
    let reply = session.receive();
    assert_eq!(errno(&reply), 0);
    u64::from_ne_bytes(reply[16..24].try_into().unwrap())
}

fn read(session: &mut Session, ino: u64, handle: u64) -> u64 {
    read_range(session, ino, handle, 0, 128)
}

fn read_range(session: &mut Session, ino: u64, handle: u64, offset: u64, size: u32) -> u64 {
    let mut body = handle.to_ne_bytes().to_vec();
    body.extend_from_slice(&offset.to_ne_bytes());
    body.extend_from_slice(&size.to_ne_bytes());
    body.extend_from_slice(&[0u8; 20]);
    session.send(15, ino, &body)
}

#[test]
fn read_deadline_before_publication_returns_etimedout_and_drops_payload_owners() {
    let (runtime, store, fs) = fixture();
    store.seed("file", &[73u8; 1024]);
    let ino = runtime.block_on(fs.lookup(1, "file")).unwrap().ino;
    *store.read_gate.lock().unwrap() = Some(Arc::new(tokio::sync::Semaphore::new(0)));
    let mut session = Session::new(fs.clone(), &runtime);
    let handle = open(&mut session, ino);
    let request = read(&mut session, ino, handle);
    let reply = session.receive();
    assert_eq!(unique(&reply), request);
    assert_eq!(errno(&reply), -libc::ETIMEDOUT);
    session.close();
    assert_eq!(fs.effective_budgets().data_used_bytes, 0);
}

#[test]
fn read_deadline_after_publication_preserves_bytes_without_a_second_reply() {
    let (runtime, store, fs) = fixture();
    store.seed("file", &vec![73u8; 1024 * 1024]);
    let ino = runtime.block_on(fs.lookup(1, "file")).unwrap().ino;
    let tail = Arc::new(tokio::sync::Semaphore::new(0));
    store
        .read_range_gates
        .lock()
        .unwrap()
        .insert((512 * 1024, 512 * 1024), tail.clone());
    let mut session = Session::new(fs.clone(), &runtime);
    let handle = open(&mut session, ino);
    let request = read(&mut session, ino, handle);
    let reply = session.receive();
    assert_eq!(unique(&reply), request);
    assert_eq!(errno(&reply), 0);
    assert_eq!(&reply[16..], &[73u8; 128]);
    runtime.block_on(runtime.spawn(async {})).unwrap();
    assert_eq!(
        metric(
            &fs,
            "geesefs_fuse_reply_duration_seconds_count{opcode=\"read\"}"
        ),
        1.
    );
    assert_eq!(
        metric(
            &fs,
            "geesefs_fuse_replies_total{opcode=\"read\",errno=\"OK\"}"
        ),
        1.
    );
    assert_eq!(
        metric(
            &fs,
            "geesefs_fuse_inflight{opcode=\"read\",state=\"reply_pending\"}"
        ),
        0.
    );
    assert_eq!(
        metric(
            &fs,
            "geesefs_fuse_inflight{opcode=\"read\",state=\"after_reply\"}"
        ),
        1.
    );
    session.close();
    assert_eq!(
        metric(
            &fs,
            "geesefs_fuse_replies_total{opcode=\"read\",errno=\"ETIMEDOUT\"}"
        ),
        0.
    );
    assert_eq!(
        metric(
            &fs,
            "geesefs_diagnostic_events_total{reason=\"deadline_after_reply\"}"
        ),
        1.
    );
    runtime.block_on(runtime.spawn(async {})).unwrap();
    assert_eq!(metric(&fs, "geesefs_read_post_reply_seconds_count"), 1.);
    assert!(metric(&fs, "geesefs_read_post_reply_seconds_sum") > 0.);
    tail.add_permits(1);
    assert_eq!(
        tail.available_permits(),
        1,
        "deadline must drop the read-ahead waiter"
    );
    assert_eq!(
        store.read_calls.load(std::sync::atomic::Ordering::SeqCst),
        2
    );
    assert_eq!(fs.effective_budgets().data_used_bytes, 512 * 1024);
}

#[test]
fn full_admission_is_attributed_without_inventing_a_backend_failure() {
    let (runtime, store, fs) = fixture();
    store.seed("slow", b"independent bytes");
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *store.head_gate.lock().unwrap() = Some(gate.clone());
    let started = store.head_started.notified();
    let mut session = Session::new(fs.clone(), &runtime);
    let first = session.send(1, 1, b"slow\0");
    runtime
        .block_on(async { tokio::time::timeout(Duration::from_secs(1), started).await })
        .unwrap();
    assert_eq!(
        metric(
            &fs,
            "geesefs_fuse_inflight{opcode=\"lookup\",state=\"reply_pending\"}"
        ),
        1.
    );
    assert!(
        metric(
            &fs,
            "geesefs_fuse_oldest_seconds{opcode=\"lookup\",state=\"reply_pending\"}"
        ) > 0.
    );
    let rejected = session.send(1, 1, b"slow\0");
    let reply = session.receive();
    assert_eq!((unique(&reply), errno(&reply)), (rejected, -libc::EAGAIN));
    assert_eq!(
        metric(
            &fs,
            "geesefs_diagnostic_events_total{reason=\"admission_full\"}"
        ),
        1.
    );
    assert_eq!(
        metric(
            &fs,
            "geesefs_fuse_replies_total{opcode=\"lookup\",errno=\"EAGAIN\"}"
        ),
        1.
    );
    gate.add_permits(1);
    let reply = session.receive();
    assert_eq!((unique(&reply), errno(&reply)), (first, 0));
    session.close();
    assert_eq!(
        store.head_starts.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
}

#[test]
fn diagnostic_logs_are_bounded_attributed_and_exclude_paths_and_backend_text() {
    const CHILD: &str = "GEESEFS_DIAGNOSTIC_LOG_CHILD";
    if let Ok(mode) = std::env::var(CHILD) {
        if mode == "blocked_stderr" || mode == "queue_full" {
            let queue_full = mode == "queue_full";
            let (runtime, store, fs) = configured_fixture(|config| {
                config.request_timeout = Duration::from_secs(5);
            });
            store.seed("success", b"independent bytes");
            let (ready, held) = mpsc::channel();
            let (release, wait) = mpsc::channel();
            let blocker = std::thread::spawn(move || {
                let _stderr = std::io::stderr().lock();
                ready.send(()).unwrap();
                wait.recv_timeout(Duration::from_secs(30)).unwrap();
            });
            held.recv_timeout(Duration::from_secs(1)).unwrap();
            let mut session = Session::new(fs.clone(), &runtime);
            let mut worker_free = true;
            for index in 0..if queue_full { 18 } else { 1 } {
                store
                    .fail_head_once
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                session.send(1, 1, b"private-request-path\0");
                assert_eq!(errno(&session.receive()), -libc::EIO);
                let (finished, receiver) = mpsc::channel();
                runtime.spawn(async move {
                    finished.send(()).unwrap();
                });
                worker_free = receiver.recv_timeout(Duration::from_secs(1)).is_ok();
                if !worker_free {
                    break;
                }
                if queue_full && index != 17 {
                    // Keep stderr blocked beyond queue capacity at the real
                    // one-record/second rate, also checking no duplicate reply.
                    session
                        .peer
                        .set_read_timeout(Some(Duration::from_millis(1100)))
                        .unwrap();
                    assert!(matches!(
                        session.peer.recv(&mut [0; 256]).unwrap_err().kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ));
                    session
                        .peer
                        .set_read_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                }
            }
            let dropped = metric(&fs, "geesefs_diagnostic_logs_dropped_total");
            if worker_free {
                let request = session.send(1, 1, b"success\0");
                let reply = session.receive();
                assert_eq!(
                    (unique(&reply), errno(&reply)),
                    (request, 0),
                    "admission must be returned while stderr is blocked"
                );
            }
            release.send(()).unwrap();
            blocker.join().unwrap();
            session.close();
            assert!(
                worker_free,
                "blocked stderr must not retain the only runtime worker or request admission"
            );
            if queue_full {
                assert!(
                    (1. ..=2.).contains(&dropped),
                    "18 records exceed 16 queued plus at most one being written: {dropped}"
                );
                assert_eq!(metric(&fs, "geesefs_diagnostic_logs_suppressed_total"), 0.);
            }
            return;
        }
        if mode == "background" {
            read_deadline_after_publication_preserves_bytes_without_a_second_reply();
            return;
        }
        if mode == "listing" || mode == "listing_cancelled" {
            let cancelled = mode == "listing_cancelled";
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .unwrap();
            let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
            let endpoint = format!("http://{}", server.server_addr());
            let (release, wait) = mpsc::channel();
            let worker = std::thread::spawn(move || {
                let request = server
                    .recv_timeout(Duration::from_secs(3))
                    .unwrap()
                    .unwrap();
                assert!(request.url().contains("list-type=ext-v1"));
                if cancelled {
                    wait.recv_timeout(Duration::from_secs(3)).unwrap();
                } else {
                    request
                        .respond(tiny_http::Response::from_string("<private-backend-text/>"))
                        .unwrap();
                }
            });
            let fs = {
                let _entered = runtime.enter();
                let mut config = support::MemoryStore::config(32);
                config.metrics_listen = Some("127.0.0.1:0".parse().unwrap());
                config.sdk_max_retries = 0;
                config.http_timeout = Duration::from_secs(2);
                config.list_type = Some(geesefs_rs::config::ListType::Extended);
                if cancelled {
                    config.request_timeout = Duration::from_millis(100);
                }
                let sdk = aws_sdk_s3::Config::builder()
                    .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
                    .region(aws_sdk_s3::config::Region::new("us-east-1"))
                    .credentials_provider(aws_sdk_s3::config::Credentials::new(
                        "disposable-access",
                        "disposable-secret",
                        None,
                        None,
                        "test-only",
                    ))
                    .endpoint_url(endpoint)
                    .force_path_style(true)
                    .build();
                let store = geesefs_rs::backend::s3::S3::new(
                    aws_sdk_s3::Client::from_conf(sdk),
                    "bucket".into(),
                    "".into(),
                )
                .unwrap()
                .with_options(&config)
                .unwrap();
                Filesystem::new(config, Arc::new(store)).unwrap()
            };
            let mut session = Session::new(fs, &runtime);
            session.send(27, 1, &[0; 8]); // OPENDIR
            let reply = session.receive();
            assert_eq!(errno(&reply), 0);
            let mut body = reply[16..24].to_vec();
            body.extend_from_slice(&0u64.to_ne_bytes());
            body.extend_from_slice(&4096u32.to_ne_bytes());
            body.extend_from_slice(&[0; 20]);
            let request = session.send(28, 1, &body); // READDIR
            let reply = session.receive();
            assert_eq!(
                (unique(&reply), errno(&reply)),
                (
                    request,
                    if cancelled {
                        -libc::ETIMEDOUT
                    } else {
                        -libc::EIO
                    }
                )
            );
            if cancelled {
                release.send(()).unwrap();
            }
            session.close();
            worker.join().unwrap();
            return;
        }
        let (runtime, store, fs) = configured_fixture(|config| {
            config.request_timeout = Duration::from_secs(5);
            if mode == "backend_off" {
                config.diagnostic_log_level = geesefs_rs::config::DiagnosticLogLevel::Off;
            } else if mode == "slow_warn" || mode == "backend_warn" {
                config.diagnostic_log_level = geesefs_rs::config::DiagnosticLogLevel::Warn;
            }
        });
        let mut session = Session::new(fs.clone(), &runtime);
        if mode == "slow_warn" || mode == "slow_error" {
            store.seed("private-request-path", b"independent bytes");
            let gate = Arc::new(tokio::sync::Semaphore::new(0));
            *store.head_gate.lock().unwrap() = Some(gate.clone());
            let started = store.head_started.notified();
            let request = session.send(1, 1, b"private-request-path\0");
            runtime.block_on(async {
                tokio::time::timeout(Duration::from_secs(1), started)
                    .await
                    .unwrap();
            });
            // Observe an intentionally gated operation beyond the one-second
            // slow threshold. This is a timing contract, not a sleep for a race.
            session
                .peer
                .set_read_timeout(Some(Duration::from_millis(1100)))
                .unwrap();
            assert!(matches!(
                session.peer.recv(&mut [0; 256]).unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ));
            session
                .peer
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            gate.add_permits(1);
            let reply = session.receive();
            assert_eq!((unique(&reply), errno(&reply)), (request, 0));
        } else if mode == "admission" {
            store.seed("private-request-path", b"fixture");
            let gate = Arc::new(tokio::sync::Semaphore::new(0));
            *store.head_gate.lock().unwrap() = Some(gate.clone());
            let started = store.head_started.notified();
            let first = session.send(1, 1, b"private-request-path\0");
            runtime.block_on(async {
                tokio::time::timeout(Duration::from_secs(1), started)
                    .await
                    .unwrap();
            });
            for _ in 0..20 {
                let rejected = session.send(1, 1, b"private-request-path\0");
                let reply = session.receive();
                assert_eq!((unique(&reply), errno(&reply)), (rejected, -libc::EAGAIN));
            }
            assert_eq!(metric(&fs, "geesefs_diagnostic_logs_suppressed_total"), 19.);
            gate.add_permits(1);
            assert_eq!(unique(&session.receive()), first);
        } else if mode == "generation" {
            let (started, release) = store.pause_head();
            session.send(1, 1, b"private-request-path\0");
            runtime.block_on(async {
                tokio::time::timeout(Duration::from_secs(1), started)
                    .await
                    .unwrap()
                    .unwrap();
                fs.create(1, "local-generation", libc::S_IFREG | 0o644, None, 0)
                    .await
                    .unwrap();
            });
            release.send(()).unwrap();
            assert_eq!(errno(&session.receive()), -libc::EAGAIN);
        } else {
            assert!(["backend", "backend_off", "backend_warn"].contains(&mode.as_str()));
            store
                .fail_head_once
                .store(true, std::sync::atomic::Ordering::Relaxed);
            session.send(1, 1, b"private-request-path\0");
            assert_eq!(errno(&session.receive()), -libc::EIO);
        }
        session.close();
        if mode == "backend_off" {
            assert_eq!(metric(&fs, "geesefs_diagnostics_enabled"), 1.);
            assert_eq!(
                metric(
                    &fs,
                    "geesefs_fuse_replies_total{opcode=\"lookup\",errno=\"EIO\"}"
                ),
                1.
            );
            assert_eq!(metric(&fs, "geesefs_diagnostic_logs_dropped_total"), 0.);
        }
        return;
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    for (mode, event, reason, errno, replied) in [
        (
            "blocked_stderr",
            "fuse_reply",
            "backend_response",
            libc::EIO,
            true,
        ),
        (
            "queue_full",
            "fuse_reply",
            "backend_response",
            libc::EIO,
            true,
        ),
        (
            "backend_off",
            "fuse_reply",
            "backend_response",
            libc::EIO,
            true,
        ),
        (
            "backend_warn",
            "fuse_reply",
            "backend_response",
            libc::EIO,
            true,
        ),
        ("slow_error", "fuse_reply", "ok", 0, true),
        ("slow_warn", "fuse_reply", "ok", 0, true),
        (
            "admission",
            "fuse_reply",
            "admission_full",
            libc::EAGAIN,
            true,
        ),
        (
            "generation",
            "fuse_reply",
            "missing_epoch_changed",
            libc::EAGAIN,
            true,
        ),
        ("backend", "fuse_reply", "backend_response", libc::EIO, true),
        ("listing", "fuse_reply", "invalid_response", libc::EIO, true),
        (
            "listing_cancelled",
            "fuse_reply",
            "deadline",
            libc::ETIMEDOUT,
            true,
        ),
        (
            "background",
            "fuse_background_deadline",
            "deadline_after_reply",
            libc::ETIMEDOUT,
            true,
        ),
    ] {
        let mut child = tokio::process::Command::new(std::env::current_exe().unwrap());
        child
            .args([
                "--exact",
                "diagnostic_logs_are_bounded_attributed_and_exclude_paths_and_backend_text",
                "--nocapture",
            ])
            .env_clear()
            .env(CHILD, mode)
            .kill_on_drop(true);
        let output = runtime.block_on(async {
            tokio::time::timeout(
                Duration::from_secs(if mode == "queue_full" { 35 } else { 10 }),
                child.output(),
            )
            .await
            .unwrap()
            .unwrap()
        });
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(output.status.success(), "{mode}: {stderr}");
        let records: Vec<serde_json::Value> = stderr
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        if mode == "backend_off" || mode == "slow_error" {
            assert!(records.is_empty(), "{mode}: {stderr}");
            continue;
        }
        if mode == "queue_full" {
            assert!((16..=17).contains(&records.len()), "{mode}: {stderr}");
        } else {
            assert_eq!(records.len(), 1, "{mode}: {stderr}");
        }
        let record = &records[0];
        assert_eq!(
            record["level"],
            if mode == "slow_warn" { "warn" } else { "error" }
        );
        assert_eq!(record["event"], event, "{stderr}");
        assert_eq!(record["reason"], reason, "{stderr}");
        assert_eq!(record["errno"], errno, "{stderr}");
        assert_eq!(record["reply_sent"], replied);
        assert!(record["request"].as_u64().unwrap() > 0);
        assert!(record["phases"].is_object());
        if mode == "listing" || mode == "listing_cancelled" {
            assert_eq!(
                record["phases"]["s3_list"]["failed"],
                mode == "listing",
                "{stderr}"
            );
            assert_eq!(
                record["phases"]["s3_list"]["cancelled"],
                mode == "listing_cancelled",
                "{stderr}"
            );
        }
        for (stage, scope) in record["phases"].as_object().unwrap() {
            if stage.starts_with("namespace_") {
                assert_eq!(
                    scope["active"], 0,
                    "completed namespace guard is not active: {stderr}"
                );
            }
        }
        assert!(!stderr.contains("private-request-path"));
        assert!(!stderr.contains("disposable deterministic fixture"));
        assert!(!stderr.contains("InjectedFailure"));
        assert!(!stderr.contains("private-backend-text"));
    }
}
