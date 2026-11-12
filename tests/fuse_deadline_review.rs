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
        Self::with_capabilities(fs, runtime, 0)
    }

    fn with_capabilities(
        fs: Arc<Filesystem>,
        runtime: &tokio::runtime::Runtime,
        flags: u32,
    ) -> Self {
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
        let init = [7u32, 31, 131072, flags]
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
        self.join();
    }

    fn join(&mut self) {
        self.join_worker();
        // Exactly one reply remains: DESTROY. A deadline after early READ
        // publication must not enqueue another reply for that READ request.
        assert_eq!(errno(&self.receive()), 0);
        self.assert_no_reply();
    }

    fn join_worker(&mut self) {
        self.finished
            .recv_timeout(Duration::from_secs(3))
            .expect("session destroy must drain admission")
            .unwrap();
        self.worker.take().unwrap().join().unwrap();
        assert!(self.shutdown_error.lock().unwrap().is_none());
    }

    fn assert_no_reply(&self) {
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

fn open_directory(session: &mut Session, ino: u64) -> u64 {
    session.send(27, ino, &[0; 8]);
    let reply = session.receive();
    assert_eq!(errno(&reply), 0);
    u64::from_ne_bytes(reply[16..24].try_into().unwrap())
}

fn read_directory(
    session: &mut Session,
    ino: u64,
    handle: u64,
    offset: u64,
    size: u32,
    plus: bool,
) -> u64 {
    let mut body = handle.to_ne_bytes().to_vec();
    body.extend_from_slice(&offset.to_ne_bytes());
    body.extend_from_slice(&size.to_ne_bytes());
    body.extend_from_slice(&[0; 20]);
    session.send(if plus { 44 } else { 28 }, ino, &body)
}

fn plus_names(reply: &[u8]) -> Vec<String> {
    assert_eq!(errno(reply), 0);
    let mut entries = &reply[16..];
    let mut names = Vec::new();
    while !entries.is_empty() {
        // Linux fuse_direntplus: 128-byte entry_out, 24-byte dirent, aligned name.
        let length = u32::from_ne_bytes(entries[144..148].try_into().unwrap()) as usize;
        names.push(
            std::str::from_utf8(&entries[152..152 + length])
                .unwrap()
                .to_owned(),
        );
        entries = &entries[(152 + length).next_multiple_of(8)..];
    }
    names
}

fn plus_session(fs: Arc<Filesystem>, runtime: &tokio::runtime::Runtime) -> Session {
    Session::with_capabilities(
        fs,
        runtime,
        fuser::InitFlags::FUSE_DO_READDIRPLUS
            .bits()
            .try_into()
            .unwrap(),
    )
}

#[test]
fn readdirplus_overlaps_directory_metadata_and_preserves_marker_attributes_and_references() {
    use std::sync::atomic::Ordering;
    let (runtime, store, fs) = configured_fixture(|config| {
        config.max_requests = 2;
        config.request_timeout = Duration::from_secs(5);
    });
    store.standard_listing.store(true, Ordering::Relaxed);
    for name in ["a", "b", "c", "d"] {
        store.seed(&format!("{name}/"), b"");
        store
            .objects
            .lock()
            .unwrap()
            .get_mut(&format!("{name}/"))
            .unwrap()
            .metadata
            .extend([
                ("mode".into(), (libc::S_IFDIR | 0o750).to_string()),
                ("uid".into(), "1234".into()),
            ]);
    }
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *store.head_gate.lock().unwrap() = Some(gate.clone());
    let mut session = plus_session(fs.clone(), &runtime);
    let handle = open_directory(&mut session, 1);
    let request = read_directory(&mut session, 1, handle, 0, 4096, true);
    let overlapped = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(1), async {
            while store.head_starts.load(Ordering::SeqCst) < 2 {
                store.head_started.notified().await;
            }
        })
        .await
        .is_ok()
    });
    gate.add_permits(4);
    let reply = session.receive();
    assert_eq!(unique(&reply), request);
    assert_eq!(plus_names(&reply), [".", "..", "a", "b", "c", "d"]);
    runtime.block_on(runtime.spawn(async {})).unwrap();
    let namespace = fs.namespace.lock().unwrap();
    for name in ["a", "b", "c", "d"] {
        let node = namespace.named(name).unwrap();
        assert_eq!(node.attr().perm, 0o750);
        assert_eq!(node.attr().uid, 1234);
        assert_eq!(node.state.lock().unwrap().lookups, 1);
    }
    drop(namespace);
    assert_eq!(store.head_starts.load(Ordering::SeqCst), 4);
    assert_eq!(
        fs.node(1).unwrap().state.lock().unwrap().lookups,
        1,
        "dot entries do not transfer lookup ownership"
    );
    assert_eq!(store.active_heads.load(Ordering::SeqCst), 0);
    assert_eq!(fs.effective_budgets().metadata_resolution_jobs_in_use, 0);
    session.close();
    assert!(
        overlapped,
        "independent directory marker HEADs are serialized before the first reply"
    );
    assert_eq!(store.peak_heads.load(Ordering::SeqCst), 2);
}

#[test]
fn readdirplus_metadata_is_globally_bounded_and_deadlines_cancel_all_work() {
    use std::sync::atomic::Ordering;
    let (runtime, store, fs) = configured_fixture(|config| {
        config.max_requests = 2;
        config.request_timeout = Duration::from_millis(250);
    });
    store.standard_listing.store(true, Ordering::Relaxed);
    let mut parents = Vec::new();
    for parent in ["left", "right"] {
        store.seed(&format!("{parent}/"), b"");
        for name in ["a", "b", "c", "d"] {
            store.seed(&format!("{parent}/{name}/"), b"");
        }
        parents.push(runtime.block_on(fs.lookup(1, parent)).unwrap().ino);
    }
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *store.head_gate.lock().unwrap() = Some(gate);
    store.head_starts.store(0, Ordering::SeqCst);
    store.peak_heads.store(0, Ordering::SeqCst);
    let mut session = plus_session(fs.clone(), &runtime);
    let first = open_directory(&mut session, parents[0]);
    let second = open_directory(&mut session, parents[1]);
    let ids = [
        read_directory(&mut session, parents[0], first, 0, 4096, true),
        read_directory(&mut session, parents[1], second, 0, 4096, true),
    ];
    let mut replies = Vec::new();
    for _ in 0..2 {
        let reply = session.receive();
        assert_eq!(errno(&reply), -libc::ETIMEDOUT);
        replies.push(unique(&reply));
    }
    replies.sort_unstable();
    assert_eq!(replies, ids);
    runtime.block_on(runtime.spawn(async {})).unwrap();
    assert_eq!(store.peak_heads.load(Ordering::SeqCst), 2);
    assert_eq!(store.active_heads.load(Ordering::SeqCst), 0);
    assert_eq!(fs.effective_budgets().metadata_resolution_jobs_in_use, 0);
    assert!(
        fs.namespace
            .lock()
            .unwrap()
            .nodes
            .values()
            .all(|node| { node.ino == 1 || node.state.lock().unwrap().lookups == 0 })
    );
    session.close();
}

#[test]
fn readdirplus_fresh_attributes_do_not_wait_for_a_busy_metadata_lane() {
    use std::sync::atomic::Ordering;
    let (runtime, store, fs) = configured_fixture(|config| {
        config.max_requests = 1;
        config.request_timeout = Duration::from_secs(1);
    });
    store.seed("warm/", b"");
    store.seed("warm/child/", b"");
    store.seed("cold", b"cold payload");
    let parent = runtime.block_on(fs.lookup(1, "warm")).unwrap();
    let page = runtime
        .block_on(fs.directory_chunk(parent.ino, "", 8))
        .unwrap();
    runtime
        .block_on(fs.refresh_inode_if_expired(page[0].ino))
        .unwrap();
    let cold = runtime.block_on(fs.lookup(1, "cold")).unwrap();
    cold.state.lock().unwrap().fetched = std::time::Instant::now() - 2 * fs.config.stat_cache_ttl;
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *store.head_gate.lock().unwrap() = Some(gate.clone());
    let heads = store.head_starts.load(Ordering::SeqCst);
    let refresh = runtime.spawn({
        let fs = fs.clone();
        async move { fs.refresh_inode_if_expired(cold.ino).await }
    });
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(1), async {
            while store.head_starts.load(Ordering::SeqCst) == heads {
                store.head_started.notified().await;
            }
        })
        .await
        .unwrap();
    });
    let mut session = plus_session(fs.clone(), &runtime);
    let handle = open_directory(&mut session, parent.ino);
    read_directory(&mut session, parent.ino, handle, 0, 4096, true);
    let reply = session.receive();
    gate.add_permits(1);
    runtime.block_on(refresh).unwrap().unwrap();
    assert_eq!(plus_names(&reply), [".", "..", "child"]);
    assert_eq!(store.head_starts.load(Ordering::SeqCst), heads + 1);
    session.close();
}

#[test]
fn readdirplus_full_frames_keep_order_and_only_transfer_emitted_references() {
    use std::sync::atomic::Ordering;
    let (runtime, store, fs) = configured_fixture(|config| {
        config.max_requests = 2;
        config.request_timeout = Duration::from_secs(5);
    });
    for name in ["a", "b", "c", "d"] {
        store.seed(&format!("{name}/"), b"");
    }
    let mut session = plus_session(fs.clone(), &runtime);
    let handle = open_directory(&mut session, 1);
    read_directory(&mut session, 1, handle, 0, 4096, false);
    assert_eq!(errno(&session.receive()), 0);
    assert_eq!(
        store.head_starts.load(Ordering::SeqCst),
        0,
        "basic READDIR needs no marker metadata"
    );
    read_directory(&mut session, 1, handle, 0, 512, true);
    assert_eq!(plus_names(&session.receive()), [".", "..", "a"]);
    runtime.block_on(runtime.spawn(async {})).unwrap();
    {
        let ns = fs.namespace.lock().unwrap();
        for name in ["a", "b", "c", "d"] {
            assert_eq!(
                ns.named(name).unwrap().state.lock().unwrap().lookups,
                u64::from(name == "a")
            );
        }
    }
    assert_eq!(store.active_heads.load(Ordering::SeqCst), 0);
    assert_eq!(fs.effective_budgets().metadata_resolution_jobs_in_use, 0);
    read_directory(&mut session, 1, handle, 3, 4096, true);
    assert_eq!(plus_names(&session.receive()), ["b", "c", "d"]);
    runtime.block_on(runtime.spawn(async {})).unwrap();
    for name in ["a", "b", "c", "d"] {
        assert_eq!(
            fs.namespace
                .lock()
                .unwrap()
                .named(name)
                .unwrap()
                .state
                .lock()
                .unwrap()
                .lookups,
            1
        );
    }
    session.close();
}

#[test]
fn readdirplus_metadata_error_cancels_siblings_without_lookup_references() {
    use std::sync::atomic::Ordering;
    let (runtime, store, fs) = configured_fixture(|config| {
        config.max_requests = 2;
        config.request_timeout = Duration::from_secs(5);
    });
    store.standard_listing.store(true, Ordering::Relaxed);
    store.seed("a/", b"");
    store.seed("b/", b"");
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *store.head_gate.lock().unwrap() = Some(gate.clone());
    let mut session = plus_session(fs.clone(), &runtime);
    let handle = open_directory(&mut session, 1);
    read_directory(&mut session, 1, handle, 0, 4096, true);
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(1), async {
            while store.head_starts.load(Ordering::SeqCst) < 2 {
                store.head_started.notified().await;
            }
        })
        .await
        .unwrap();
    });
    store.fail_head_once.store(true, Ordering::SeqCst);
    gate.add_permits(1);
    assert_eq!(errno(&session.receive()), -libc::EIO);
    runtime.block_on(runtime.spawn(async {})).unwrap();
    assert_eq!(store.active_heads.load(Ordering::SeqCst), 0);
    assert_eq!(fs.effective_budgets().metadata_resolution_jobs_in_use, 0);
    for name in ["a", "b"] {
        assert_eq!(
            fs.namespace
                .lock()
                .unwrap()
                .named(name)
                .unwrap()
                .state
                .lock()
                .unwrap()
                .lookups,
            0
        );
    }
    session.close();
}

#[test]
#[ignore = "explicit delayed-backend FUSE protocol diagnostic, not mounted acceptance"]
fn readdirplus_directory_latency_diagnostic() {
    use std::{sync::atomic::Ordering, time::Instant};
    let mut samples = Vec::new();
    for _ in 0..8 {
        let (runtime, store, fs) = configured_fixture(|config| {
            config.max_requests = 8;
            config.request_timeout = Duration::from_secs(5);
        });
        let expected: Vec<_> = [".".into(), "..".into()]
            .into_iter()
            .chain((0..110).map(|i| format!("dir-{i:03}")))
            .collect();
        for name in &expected[2..] {
            store.seed(&format!("{name}/child"), b"independent fixture bytes");
        }
        *store.head_delay.lock().unwrap() = Duration::from_millis(2);
        let mut session = plus_session(fs.clone(), &runtime);
        let handle = open_directory(&mut session, 1);
        let started = Instant::now();
        read_directory(&mut session, 1, handle, 0, 32768, true);
        let reply = session.receive();
        let elapsed = started.elapsed().as_nanos();
        assert_eq!(plus_names(&reply), expected);
        assert_eq!(store.head_starts.load(Ordering::SeqCst), 110);
        assert_eq!(store.list_calls.load(Ordering::SeqCst), 112);
        runtime.block_on(runtime.spawn(async {})).unwrap();
        assert_eq!(store.active_heads.load(Ordering::SeqCst), 0);
        samples.push(serde_json::json!({"wall_ns":elapsed,
            "peak_heads":store.peak_heads.load(Ordering::SeqCst)}));
        session.close();
    }
    println!(
        "readdirplus_latency_json={}",
        serde_json::json!({"samples":samples,
        "directories":110,"head_delay_ms":2,"metadata_slots":8,
        "heads_per_sample":110,"lists_per_sample":112,
        "scope":"fuser protocol, faithful delayed store, unoptimized test binary; not kernel/production/Go acceptance"})
    );
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
    fixture_on(runtime, configure)
}

fn fixture_on(
    runtime: tokio::runtime::Runtime,
    configure: impl FnOnce(&mut geesefs_rs::config::Config),
) -> (
    tokio::runtime::Runtime,
    Arc<support::MemoryStore>,
    Arc<Filesystem>,
) {
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

async fn wait_until(message: &str, condition: impl Fn() -> bool) {
    // A real watchdog also bounds tests with Tokio's clock explicitly paused.
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while !condition() {
        assert!(std::time::Instant::now() < deadline, "{message}");
        tokio::task::yield_now().await;
    }
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
    open_with_flags(session, ino, libc::O_RDONLY)
}

fn open_with_flags(session: &mut Session, ino: u64, flags: i32) -> u64 {
    let mut body = flags.to_ne_bytes().to_vec();
    body.extend_from_slice(&[0u8; 4]);
    session.send(14, ino, &body);
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

fn write(session: &mut Session, ino: u64, handle: u64, data: &[u8], flags: i32) -> u64 {
    let mut body = handle.to_ne_bytes().to_vec();
    body.extend_from_slice(&0u64.to_ne_bytes());
    body.extend_from_slice(&(data.len() as u32).to_ne_bytes());
    body.extend_from_slice(&[0u8; 12]); // write_flags and lock_owner
    body.extend_from_slice(&flags.to_ne_bytes());
    body.extend_from_slice(&[0u8; 4]);
    body.extend_from_slice(data);
    session.send(16, ino, &body)
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
    let (runtime, store, fs) = fixture_on(
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(true)
            .build()
            .unwrap(),
        |_| {},
    );
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
    runtime.block_on(async {
        // Observe the actual early reply and gated tail before advancing the same
        // 100 ms deadline. Real scheduler/metrics overhead must not race this state.
        wait_until("early READ did not reach its gated tail", || {
            store.read_calls.load(std::sync::atomic::Ordering::SeqCst) == 2
                && metric(
                    &fs,
                    "geesefs_fuse_inflight{opcode=\"read\",state=\"after_reply\"}",
                ) == 1.
        })
        .await;
        let reply = session.receive();
        assert_eq!(unique(&reply), request);
        assert_eq!(errno(&reply), 0);
        assert_eq!(&reply[16..], &[73u8; 128]);
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
        tokio::time::advance(fs.config.request_timeout).await;
        wait_until("expired post-reply READ retained its owner", || {
            metric(
                &fs,
                "geesefs_fuse_inflight{opcode=\"read\",state=\"after_reply\"}",
            ) == 0.
        })
        .await;
        tokio::task::spawn_blocking(move || session.close())
            .await
            .unwrap();
    });
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
fn full_admission_waits_without_inventing_a_backend_failure() {
    let (runtime, store, fs) = configured_fixture(|config| {
        config.request_timeout = Duration::from_secs(5);
    });
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
    let waiting = session.send(1, 1, b"slow\0");
    runtime.block_on(wait_until(
        "second lookup did not wait for admission",
        || metric(&fs, "geesefs_stage_inflight{stage=\"fuse_admission\"}") == 1.,
    ));
    assert_eq!(
        metric(
            &fs,
            "geesefs_diagnostic_events_total{reason=\"admission_wait\"}"
        ),
        1.
    );
    assert_eq!(
        metric(
            &fs,
            "geesefs_fuse_replies_total{opcode=\"lookup\",errno=\"EAGAIN\"}"
        ),
        0.
    );
    gate.add_permits(1);
    let reply = session.receive();
    assert_eq!((unique(&reply), errno(&reply)), (first, 0));
    let reply = session.receive();
    assert_eq!((unique(&reply), errno(&reply)), (waiting, 0));
    session.close();
    assert_eq!(
        store.head_starts.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
}

#[test]
fn admission_and_execution_share_one_deadline_on_a_driven_current_thread_runtime() {
    use std::sync::atomic::Ordering;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .unwrap();
    let store = support::MemoryStore::new();
    store.seed("first", b"first bytes");
    store.seed("second", b"second bytes");
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *store.head_gate.lock().unwrap() = Some(gate.clone());
    let mut config = support::MemoryStore::config(32);
    config.max_requests = 1;
    config.request_timeout = Duration::from_secs(10);
    config.metrics_listen = Some("127.0.0.1:0".parse().unwrap());
    let fs = Filesystem::new(config, store.clone()).unwrap();
    runtime.block_on(async {
        let mut session = Session::new(fs.clone(), &runtime);
        let first = session.send(1, 1, b"first\0");
        wait_until("first HEAD did not start", || {
            store.head_starts.load(Ordering::SeqCst) == 1
        })
        .await;
        tokio::time::advance(Duration::from_secs(3)).await;
        let second = session.send(1, 1, b"second\0");
        wait_until("second lookup did not wait for admission", || {
            metric(&fs, "geesefs_stage_inflight{stage=\"fuse_admission\"}") == 1.
        })
        .await;
        tokio::time::advance(Duration::from_secs(6)).await;
        gate.add_permits(1);
        wait_until("second HEAD did not start after capacity returned", || {
            store.head_starts.load(Ordering::SeqCst) == 2
        })
        .await;
        let reply = session.receive();
        assert_eq!((unique(&reply), errno(&reply)), (first, 0));
        tokio::time::advance(Duration::from_secs(3)).await;
        assert_eq!(store.active_heads.load(Ordering::SeqCst), 1);
        // Second callback arrived at t=3 and began work at t=9. Its deadline
        // remains t=13, not t=19; advance beyond it without real-time sleeps.
        tokio::time::advance(Duration::from_secs(1) + Duration::from_millis(1)).await;
        wait_until(
            "admission wait incorrectly renewed the execution deadline",
            || {
                metric(
                    &fs,
                    "geesefs_fuse_replies_total{opcode=\"lookup\",errno=\"ETIMEDOUT\"}",
                ) == 1.
            },
        )
        .await;
        let reply = session.receive();
        assert_eq!((unique(&reply), errno(&reply)), (second, -libc::ETIMEDOUT));
        assert_eq!(store.active_heads.load(Ordering::SeqCst), 0);
        assert_eq!(fs.effective_budgets().metadata_resolution_jobs_in_use, 0);
        gate.add_permits(1);
        let recovered = session.send(1, 1, b"second\0");
        wait_until(
            "next request did not complete after timed-out owner",
            || {
                metric(
                    &fs,
                    "geesefs_fuse_replies_total{opcode=\"lookup\",errno=\"OK\"}",
                ) == 2.
            },
        )
        .await;
        let reply = session.receive();
        assert_eq!((unique(&reply), errno(&reply)), (recovered, 0));
        // Keep driving the current-thread runtime while fuser drains shutdown.
        tokio::task::spawn_blocking(move || session.close())
            .await
            .unwrap();
    });
}

#[test]
fn expired_queued_writes_do_not_mutate_or_retain_ingress() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .unwrap();
    let store = support::MemoryStore::new();
    store.seed("file", b"original bytes");
    let mut config = support::MemoryStore::config(32);
    config.max_requests = 1;
    config.request_timeout = Duration::from_secs(10);
    config.metrics_listen = Some("127.0.0.1:0".parse().unwrap());
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let ino = runtime.block_on(fs.lookup(1, "file")).unwrap().ino;
    let mut session = Session::new(fs.clone(), &runtime);
    let handle = open_with_flags(&mut session, ino, libc::O_RDWR);
    // The executor is deliberately not polled yet. One request owns its permit
    // and the other waits in the sole receiver, both with already-funded input.
    let first = write(&mut session, ino, handle, b"first overwrite", libc::O_RDWR);
    let second = write(&mut session, ino, handle, b"second overwrite", libc::O_RDWR);
    let watchdog = std::time::Instant::now() + Duration::from_secs(3);
    while metric(&fs, "geesefs_stage_inflight{stage=\"fuse_admission\"}") != 1. {
        assert!(
            std::time::Instant::now() < watchdog,
            "second WRITE did not wait"
        );
        std::thread::yield_now();
    }
    assert_eq!(
        fs.ingress.used(),
        b"first overwrite".len() + b"second overwrite".len()
    );
    runtime.block_on(async {
        tokio::time::advance(Duration::from_secs(10)).await;
        wait_until("expired WRITE jobs did not return timeout", || {
            metric(
                &fs,
                "geesefs_fuse_replies_total{opcode=\"write\",errno=\"ETIMEDOUT\"}",
            ) == 2.
        })
        .await;
        let mut replies = [session.receive(), session.receive()];
        replies.sort_by_key(|reply| unique(reply));
        assert_eq!(
            (unique(&replies[0]), errno(&replies[0])),
            (first, -libc::ETIMEDOUT)
        );
        assert_eq!(
            (unique(&replies[1]), errno(&replies[1])),
            (second, -libc::ETIMEDOUT)
        );
        assert_eq!(fs.ingress.used(), 0);
        assert_eq!(fs.effective_budgets().data_used_bytes, 0);
        fs.fsync(ino).await.unwrap();
        assert_eq!(store.bytes("file"), b"original bytes");
        assert_eq!(store.put_calls.load(std::sync::atomic::Ordering::SeqCst), 0);
        tokio::task::spawn_blocking(move || session.close())
            .await
            .unwrap();
    });
}

#[test]
fn admitted_sync_write_preserves_failure_and_fsync_recovery() {
    use std::sync::atomic::Ordering;
    let (runtime, store, fs) = configured_fixture(|config| {
        config.request_timeout = Duration::from_secs(5);
    });
    store.seed("file", b"old bytes");
    store.seed("slow", b"independent bytes");
    let ino = runtime.block_on(fs.lookup(1, "file")).unwrap().ino;
    let mut session = Session::new(fs.clone(), &runtime);
    let handle = open_with_flags(&mut session, ino, libc::O_RDWR | libc::O_SYNC);
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *store.head_gate.lock().unwrap() = Some(gate.clone());
    let first = session.send(1, 1, b"slow\0");
    runtime.block_on(wait_until("blocking lookup did not start", || {
        store.active_heads.load(Ordering::SeqCst) == 1
    }));
    store.fail_put.store(true, Ordering::SeqCst);
    let written = write(
        &mut session,
        ino,
        handle,
        b"new bytes",
        libc::O_RDWR | libc::O_SYNC,
    );
    runtime.block_on(wait_until("WRITE did not wait for admission", || {
        metric(&fs, "geesefs_stage_inflight{stage=\"fuse_admission\"}") == 1.
    }));
    assert_eq!(fs.ingress.used(), b"new bytes".len());
    assert_eq!(store.put_calls.load(Ordering::SeqCst), 0);
    gate.add_permits(1);
    let reply = session.receive();
    assert_eq!((unique(&reply), errno(&reply)), (first, 0));
    let reply = session.receive();
    assert_eq!((unique(&reply), errno(&reply)), (written, -libc::EIO));
    assert_eq!(store.bytes("file"), b"old bytes");
    store.fail_put.store(false, Ordering::SeqCst);
    *store.head_gate.lock().unwrap() = None;
    let mut body = handle.to_ne_bytes().to_vec();
    body.extend_from_slice(&[0u8; 8]);
    let synced = session.send(20, ino, &body);
    let reply = session.receive();
    assert_eq!((unique(&reply), errno(&reply)), (synced, 0));
    session.close();
    assert_eq!(store.bytes("file"), b"new bytes");
    assert_eq!(fs.ingress.used(), 0);
}

#[test]
fn destroy_behind_saturated_admission_drains_expired_owners() {
    let (runtime, store, fs) = fixture();
    store.seed("first", b"first bytes");
    store.seed("second", b"second bytes");
    *store.head_gate.lock().unwrap() = Some(Arc::new(tokio::sync::Semaphore::new(0)));
    let mut session = Session::new(fs.clone(), &runtime);
    let first = session.send(1, 1, b"first\0");
    runtime.block_on(wait_until("blocking lookup did not start", || {
        store.active_heads.load(std::sync::atomic::Ordering::SeqCst) == 1
    }));
    let second = session.send(1, 1, b"second\0");
    runtime.block_on(wait_until(
        "second request did not wait for admission",
        || metric(&fs, "geesefs_stage_inflight{stage=\"fuse_admission\"}") == 1.,
    ));
    let destroy = session.send(38, 0, &[]);
    // fuser acknowledges DESTROY before invoking the filesystem drain hook;
    // the final pending reply may arrive after that acknowledgement.
    let mut replies = [session.receive(), session.receive(), session.receive()];
    replies.sort_by_key(|reply| unique(reply));
    for (reply, expected) in replies.iter().zip([
        (first, -libc::ETIMEDOUT),
        (second, -libc::ETIMEDOUT),
        (destroy, 0),
    ]) {
        assert_eq!((unique(reply), errno(reply)), expected);
    }
    session.join_worker();
    session.assert_no_reply();
    assert_eq!(
        store.active_heads.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    assert_eq!(
        metric(&fs, "geesefs_stage_inflight{stage=\"fuse_admission\"}"),
        0.
    );
}

#[test]
fn debug_saturation_preserves_a_severe_reply_without_blocking_fuse() {
    const CHILD: &str = "GEESEFS_DEBUG_RESERVED_ERROR_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let (runtime, store, fs) = configured_fixture(|config| {
            config.diagnostic_log_level = geesefs_rs::config::DiagnosticLogLevel::Debug;
            config.request_timeout = Duration::from_secs(5);
        });
        store.seed("private-flood-success", b"private-flood-payload");
        let (ready, held) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let blocker = std::thread::spawn(move || {
            let _stderr = std::io::stderr().lock();
            ready.send(()).unwrap();
            wait.recv_timeout(Duration::from_secs(15)).unwrap();
        });
        held.recv_timeout(Duration::from_secs(1)).unwrap();
        let mut session = Session::new(fs.clone(), &runtime);
        for _ in 0..150 {
            let request = session.send(1, 1, b"private-flood-success\0");
            let reply = session.receive();
            assert_eq!((unique(&reply), errno(&reply)), (request, 0));
            runtime.block_on(runtime.spawn(async {})).unwrap();
        }
        assert!(metric(&fs, "geesefs_diagnostic_logs_dropped_total") > 0.);
        store
            .fail_head_once
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let failed = session.send(1, 1, b"private-flood-error\0");
        let reply = session.receive();
        assert_eq!((unique(&reply), errno(&reply)), (failed, -libc::EIO));
        runtime.block_on(runtime.spawn(async {})).unwrap();
        let success = session.send(1, 1, b"private-flood-success\0");
        let reply = session.receive();
        assert_eq!((unique(&reply), errno(&reply)), (success, 0));
        session.close();
        assert_eq!(
            metric(&fs, "geesefs_diagnostic_severe_queue_dropped_total"),
            0.
        );
        release.send(()).unwrap();
        blocker.join().unwrap();
        return;
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut child = tokio::process::Command::new(std::env::current_exe().unwrap());
    child
        .args([
            "--exact",
            "debug_saturation_preserves_a_severe_reply_without_blocking_fuse",
            "--nocapture",
        ])
        .env_clear()
        .env(CHILD, "1")
        .kill_on_drop(true);
    let output = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(20), child.output())
            .await
            .unwrap()
            .unwrap()
    });
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(output.status.success(), "{stderr}");
    let records: Vec<serde_json::Value> = stderr
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let errors: Vec<_> = records
        .iter()
        .filter(|record| record["event"] == "fuse_reply" && record["errno"] == libc::EIO)
        .collect();
    assert_eq!(
        errors.len(),
        1,
        "debug saturation must not consume the severe record's queue slot"
    );
    assert_eq!(errors[0]["level"], "error");
    assert_eq!(errors[0]["reply_sent"], true);
    assert_eq!(errors[0]["reason"], "backend_response");
    for forbidden in [
        "private-flood-success",
        "private-flood-payload",
        "private-flood-error",
    ] {
        assert!(!stderr.contains(forbidden));
    }
}

#[test]
fn severe_replies_retain_backend_status_without_debug_history() {
    const CHILD: &str = "GEESEFS_BACKEND_FAILURE_CONTEXT_CHILD";
    if let Ok(mode) = std::env::var(CHILD) {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", server.server_addr());
        let sdk = mode.starts_with("sdk_");
        let repeat_head = mode == "sdk_repeat_head";
        let body_failure = mode.starts_with("sdk_body");
        let checksummed = mode == "sdk_body_checksummed";
        let invalid = mode == "extension_invalid";
        let recovered = mode.ends_with("recovered");
        let forbidden = mode == "extension_forbidden";
        let requests = if repeat_head {
            4
        } else if forbidden || invalid {
            1
        } else if body_failure || recovered {
            2
        } else {
            3
        };
        let worker = std::thread::spawn(move || {
            for index in 0..requests {
                let request = server
                    .recv_timeout(Duration::from_secs(10))
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    request.method().as_str(),
                    if sdk && !(body_failure && index == 1) {
                        "HEAD"
                    } else {
                        "GET"
                    }
                );
                if !sdk {
                    assert!(request.url().contains("list-type=ext-v1"));
                }
                if repeat_head && index == 0 {
                    request.respond(tiny_http::Response::empty(404)).unwrap();
                    continue;
                }
                if body_failure || recovered && index == 1 {
                    let body = if sdk {
                        b"ab".to_vec()
                    } else {
                        b"<ListBucketResult><IsTruncated>false</IsTruncated></ListBucketResult>"
                            .to_vec()
                    };
                    let length = if sdk { 4 } else { body.len() };
                    let status = if body_failure && index == 1 { 206 } else { 200 };
                    let mut headers: Vec<_> = [
                        ("ETag", "\"private-etag\""),
                        ("Content-Range", "bytes 0-3/4"),
                        ("Connection", "close"),
                    ]
                    .into_iter()
                    .map(|(name, value)| tiny_http::Header::from_bytes(name, value).unwrap())
                    .collect();
                    if checksummed && index == 1 {
                        headers.push(
                            tiny_http::Header::from_bytes("x-amz-checksum-crc32", "AAAAAA==")
                                .unwrap(),
                        );
                    }
                    let _ = request.respond(
                        tiny_http::Response::new(
                            tiny_http::StatusCode(status),
                            headers,
                            std::io::Cursor::new(body),
                            Some(length),
                            None,
                        )
                        .with_chunked_threshold(usize::MAX),
                    );
                    continue;
                }
                let response = if invalid {
                    tiny_http::Response::from_string("private-invalid-xml").with_status_code(200)
                } else if !sdk && !forbidden {
                    tiny_http::Response::from_string("private-invalid-gzip-body")
                        .with_status_code(503)
                        .with_header(
                            tiny_http::Header::from_bytes("Content-Encoding", "gzip").unwrap(),
                        )
                } else {
                    tiny_http::Response::from_string(
                        "<Error><Code>private-backend-code</Code><Message>https://private-host/private-key?secret=private-secret</Message></Error>",
                    )
                    .with_status_code(if forbidden { 403 } else { 503 })
                };
                request.respond(response).unwrap();
            }
        });
        let fs = {
            let _entered = runtime.enter();
            let mut config = support::MemoryStore::config(32);
            config.metrics_listen = Some("127.0.0.1:0".parse().unwrap());
            config.diagnostic_log_level = geesefs_rs::config::DiagnosticLogLevel::Error;
            config.sdk_max_retries = 2;
            config.request_timeout = Duration::from_secs(15);
            config.http_timeout = Duration::from_secs(10);
            config.list_type = Some(geesefs_rs::config::ListType::Extended);
            if sdk {
                // Isolate the authoritative HEAD from optional parent preloading.
                config.stat_cache_ttl = Duration::ZERO;
            }
            let sdk = aws_sdk_s3::Config::builder()
                .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
                .region(aws_sdk_s3::config::Region::new("us-east-1"))
                .credentials_provider(aws_sdk_s3::config::Credentials::new(
                    "private-access",
                    "private-secret",
                    None,
                    None,
                    "test-only",
                ))
                .endpoint_url(endpoint)
                .force_path_style(true)
                .build();
            let store = geesefs_rs::backend::s3::S3::new(
                aws_sdk_s3::Client::from_conf(sdk),
                "private-bucket".into(),
                "".into(),
            )
            .unwrap()
            .with_options(&config)
            .unwrap();
            Filesystem::new(config, Arc::new(store)).unwrap()
        };
        let mut session = Session::new(fs, &runtime);
        session
            .peer
            .set_read_timeout(Some(Duration::from_secs(15)))
            .unwrap();
        let request = if sdk {
            let lookup = session.send(1, 1, b"private-key\0");
            if body_failure {
                let reply = session.receive();
                assert_eq!((unique(&reply), errno(&reply)), (lookup, 0));
                let ino = u64::from_ne_bytes(reply[16..24].try_into().unwrap());
                let handle = open(&mut session, ino);
                read_range(&mut session, ino, handle, 0, 4)
            } else {
                lookup
            }
        } else {
            session.send(27, 1, &[0; 8]);
            let reply = session.receive();
            assert_eq!(errno(&reply), 0);
            let mut body = reply[16..24].to_vec();
            body.extend_from_slice(&0u64.to_ne_bytes());
            body.extend_from_slice(&4096u32.to_ne_bytes());
            body.extend_from_slice(&[0; 20]);
            session.send(28, 1, &body)
        };
        let reply = session.receive();
        assert_eq!(
            (unique(&reply), errno(&reply)),
            (request, if recovered { 0 } else { -libc::EIO })
        );
        session.close();
        worker.join().unwrap();
        return;
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    for (mode, operation, transport, status, kind, attempts) in [
        (
            "extension_gzip",
            "list",
            "extension",
            Some(503),
            "decode",
            Some(3),
        ),
        (
            "extension_forbidden",
            "list",
            "extension",
            Some(403),
            "service",
            Some(1),
        ),
        (
            "extension_invalid",
            "list",
            "extension",
            Some(200),
            "invalid_response",
            Some(1),
        ),
        ("sdk_head", "head", "sdk", Some(503), "service", Some(3)),
        (
            "sdk_repeat_head",
            "head",
            "sdk",
            Some(503),
            "service",
            Some(3),
        ),
        ("sdk_body", "get", "sdk_body", None, "body", None),
        (
            "sdk_body_checksummed",
            "get",
            "sdk_body",
            None,
            "body",
            None,
        ),
        ("extension_recovered", "", "", None, "", None),
        ("sdk_recovered", "", "", None, "", None),
    ] {
        let mut child = tokio::process::Command::new(std::env::current_exe().unwrap());
        child
            .args([
                "--exact",
                "severe_replies_retain_backend_status_without_debug_history",
                "--nocapture",
            ])
            .env_clear()
            .env(CHILD, mode)
            .kill_on_drop(true);
        let output = runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(25), child.output())
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
        if mode.ends_with("recovered") {
            assert!(
                records.is_empty(),
                "recovered retries must not emit failures: {mode}"
            );
            continue;
        }
        assert_eq!(
            records.len(),
            1,
            "only the severe record is required: {mode}"
        );
        assert_eq!(records[0]["event"], "fuse_reply");
        assert_eq!(records[0]["errno"], libc::EIO);
        let failures = records[0]["backend_failures"]
            .as_array()
            .expect("the severe reply must carry backend context without debug history");
        assert_eq!(failures.len(), 1, "{mode}");
        assert_eq!(failures[0]["operation"], operation, "{mode}");
        assert_eq!(failures[0]["transport"], transport, "{mode}");
        assert_eq!(
            failures[0]["http_status"],
            serde_json::json!(status),
            "{mode}"
        );
        assert_eq!(failures[0]["kind"], kind, "{mode}");
        assert_eq!(
            failures[0]["attempts"],
            serde_json::json!(attempts),
            "{mode}"
        );
        assert_eq!(
            failures[0]["observations"],
            if mode == "sdk_repeat_head" { 2 } else { 1 },
            "{mode}"
        );
        assert!(failures[0]["observed_seconds"].as_f64().unwrap() >= 0.);
        assert!(
            failures[0]["observed_seconds"].as_f64().unwrap()
                <= records[0]["elapsed_seconds"].as_f64().unwrap()
        );
        for forbidden in ["private-", "http://", "https://", "X-Amz-", "Authorization"] {
            assert!(!stderr.contains(forbidden), "{mode}: leaked {forbidden}");
        }
    }
}

#[test]
fn diagnostic_records_preserve_journal_priorities_only_on_the_matching_stream() {
    const CHILD: &str = "GEESEFS_JOURNAL_PRIORITY_CHILD";
    if let Ok(mode) = std::env::var(CHILD) {
        let (runtime, store, fs) = configured_fixture(|config| {
            config.diagnostic_log_level = geesefs_rs::config::DiagnosticLogLevel::Debug;
            config.request_timeout = Duration::from_secs(5);
        });
        store.seed("private-priority-success", b"private-priority-payload");
        if mode == "matching" {
            *store.head_delay.lock().unwrap() = Duration::from_millis(1100);
        }
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *store.head_gate.lock().unwrap() = Some(gate.clone());
        let mut session = Session::new(fs.clone(), &runtime);
        let first = session.send(1, 1, b"private-priority-success\0");
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(1), store.head_started.notified())
                .await
                .unwrap();
        });
        gate.add_permits(1);
        let reply = session.receive();
        assert_eq!((unique(&reply), errno(&reply)), (first, 0));
        runtime.block_on(runtime.spawn(async {})).unwrap();
        *store.head_gate.lock().unwrap() = None;
        *store.head_delay.lock().unwrap() = Duration::ZERO;
        // A real namespace-generation conflict still returns EAGAIN/warn.
        // Capacity now waits, so it is no longer a warning fixture.
        let (started, release) = store.pause_head();
        let rejected = session.send(1, 1, b"private-priority-rejected\0");
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
        let reply = session.receive();
        assert_eq!((unique(&reply), errno(&reply)), (rejected, -libc::EAGAIN));
        store
            .fail_head_once
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let failed = session.send(1, 1, b"private-priority-error\0");
        let reply = session.receive();
        assert_eq!((unique(&reply), errno(&reply)), (failed, -libc::EIO));
        session.close();
        assert_eq!(metric(&fs, "geesefs_diagnostic_logs_dropped_total"), 0.);
        return;
    }
    use std::os::fd::OwnedFd;
    use std::os::unix::net::UnixStream;
    use tokio::io::AsyncReadExt;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    for mode in [
        "matching",
        "wrong_device",
        "wrong_inode",
        "malformed",
        "missing_separator",
        "absent",
    ] {
        let (reader, writer) = UnixStream::pair().unwrap();
        let stat = rustix::fs::fstat(&writer).unwrap();
        let mut child = tokio::process::Command::new(std::env::current_exe().unwrap());
        child
            .args([
                "--exact",
                "diagnostic_records_preserve_journal_priorities_only_on_the_matching_stream",
                "--nocapture",
            ])
            .env_clear()
            .env(CHILD, mode)
            .kill_on_drop(true)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::from(OwnedFd::from(writer)));
        let identity = match mode {
            "matching" => Some(format!("{}:{}", stat.st_dev, stat.st_ino)),
            "wrong_device" => Some(format!("{}:{}", stat.st_dev + 1, stat.st_ino)),
            "wrong_inode" => Some(format!("{}:{}", stat.st_dev, stat.st_ino + 1)),
            "malformed" => Some("not-a-device:inode".into()),
            "missing_separator" => Some("123".into()),
            "absent" => None,
            _ => unreachable!(),
        };
        if let Some(identity) = identity {
            child.env("JOURNAL_STREAM", identity);
        }
        reader.set_nonblocking(true).unwrap();
        let (status, bytes) = runtime.block_on(async {
            let mut process = child.spawn().unwrap();
            drop(child);
            let mut reader = tokio::net::UnixStream::from_std(reader).unwrap();
            let mut bytes = Vec::new();
            let (status, _) = tokio::time::timeout(Duration::from_secs(10), async {
                tokio::try_join!(process.wait(), reader.read_to_end(&mut bytes))
            })
            .await
            .unwrap()
            .unwrap();
            (status, bytes)
        });
        let stderr = String::from_utf8(bytes).unwrap();
        assert!(status.success(), "{mode}: {stderr}");
        let mut levels = std::collections::BTreeSet::new();
        for line in stderr.lines() {
            let (prefix, json) = if mode == "matching" {
                assert!(
                    line.len() >= 3 && line.starts_with('<') && line.as_bytes()[2] == b'>',
                    "journal stream must carry a severity prefix: {line}"
                );
                line.split_at(3)
            } else {
                ("", line)
            };
            let record: serde_json::Value = serde_json::from_str(json).unwrap();
            let level = record["level"].as_str().unwrap();
            if mode == "matching" {
                assert_eq!(
                    prefix,
                    match level {
                        "error" => "<3>",
                        "warn" => "<4>",
                        "info" => "<6>",
                        "debug" => "<7>",
                        _ => panic!("unknown level: {level}"),
                    }
                );
            }
            levels.insert(level.to_owned());
        }
        assert!(
            levels.contains("error") && levels.contains("warn") && levels.contains("debug"),
            "{mode}: {stderr}"
        );
        if mode == "matching" {
            assert!(
                levels.contains("info"),
                "slow reply must exercise journal info priority: {stderr}"
            );
        }
        for forbidden in [
            "private-priority-success",
            "private-priority-payload",
            "private-priority-rejected",
            "private-priority-error",
        ] {
            assert!(!stderr.contains(forbidden));
        }
    }
}

#[test]
fn debug_records_correlate_successful_request_lifecycle_and_phase_scopes() {
    const CHILD: &str = "GEESEFS_DEBUG_TRACE_CHILD";
    if let Ok(level) = std::env::var(CHILD) {
        use clap::Parser;
        let parsed = geesefs_rs::config::Config::try_parse_from([
            "geesefs-rs",
            "--diagnostic-log-level",
            &level,
            "bucket",
            "/unused",
        ])
        .expect("the requested diagnostic level must be accepted");
        let (runtime, store, fs) = configured_fixture(|config| {
            config.diagnostic_log_level = parsed.diagnostic_log_level;
        });
        store.seed("private-request-path", b"private-payload-never-log");
        let mut session = Session::new(fs.clone(), &runtime);
        let request = session.send(1, 1, b"private-request-path\0");
        let reply = session.receive();
        assert_eq!((unique(&reply), errno(&reply)), (request, 0));
        let ino = u64::from_ne_bytes(reply[16..24].try_into().unwrap());
        runtime.block_on(runtime.spawn(async {})).unwrap();
        let handle = open(&mut session, ino);
        for _ in 0..2 {
            runtime.block_on(runtime.spawn(async {})).unwrap();
            let request = read_range(&mut session, ino, handle, 0, 25);
            let reply = session.receive();
            assert_eq!((unique(&reply), errno(&reply)), (request, 0));
            assert_eq!(&reply[16..], b"private-payload-never-log");
        }
        session.close();
        assert_eq!(
            metric(
                &fs,
                "geesefs_fuse_replies_total{opcode=\"lookup\",errno=\"OK\"}"
            ),
            1.
        );
        assert_eq!(metric(&fs, "geesefs_diagnostic_logs_dropped_total"), 0.);
        return;
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    for level in ["debug", "info", "warn", "error", "off"] {
        let mut child = tokio::process::Command::new(std::env::current_exe().unwrap());
        child
            .args([
                "--exact",
                "debug_records_correlate_successful_request_lifecycle_and_phase_scopes",
                "--nocapture",
            ])
            .env_clear()
            .env(CHILD, level)
            .kill_on_drop(true);
        let output = runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(10), child.output())
                .await
                .unwrap()
                .unwrap()
        });
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(output.status.success(), "{level}: {stderr}");
        let records: Vec<serde_json::Value> = stderr
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        if level != "debug" {
            assert!(
                records.is_empty(),
                "a fast successful request is debug-only: {level}: {stderr}"
            );
            continue;
        }
        let lookup: Vec<_> = records
            .iter()
            .filter(|record| record["opcode"] == "lookup")
            .collect();
        for event in ["fuse_request", "fuse_polled", "fuse_reply", "fuse_complete"] {
            let record = lookup
                .iter()
                .find(|record| record["event"] == event)
                .unwrap_or_else(|| panic!("missing {event}: {stderr}"));
            assert_eq!(record["level"], "debug");
            assert_eq!(record["ino"], 1);
            assert!(record["request"].as_u64().unwrap() > 0);
            assert!(record["timestamp_unix_seconds"].as_f64().unwrap() > 0.);
        }
        let trace = &lookup[0]["trace_id"];
        assert!(lookup.iter().all(|record| &record["trace_id"] == trace));
        assert!(lookup.iter().any(|record| record["event"] == "phase_start"));
        assert!(
            lookup.iter().any(
                |record| record["event"] == "phase_end" && record["details"]["outcome"] == "ok"
            )
        );
        assert!(lookup.iter().any(|record| record["event"] == "fuse_reply"
            && record["errno"] == 0
            && record["reply_sent"] == true));
        for hit in [false, true] {
            assert!(
                records.iter().any(|record| record["event"] == "read_cache"
                    && record["details"]["hit"] == hit
                    && record["details"]["bytes"] == 25),
                "{stderr}"
            );
        }
        assert!(
            records
                .iter()
                .any(|record| record["event"] == "read_fill"
                    && record["details"]["role"] == "leader"),
            "{stderr}"
        );
        for forbidden in ["private-request-path", "private-payload-never-log"] {
            assert!(
                !stderr.contains(forbidden),
                "debug must not disclose request names or bytes"
            );
        }
    }
}

#[test]
fn diagnostic_logs_are_bounded_attributed_and_exclude_paths_and_backend_text() {
    const CHILD: &str = "GEESEFS_DIAGNOSTIC_LOG_CHILD";
    if let Ok(mode) = std::env::var(CHILD) {
        if mode == "blocked_stderr" || mode == "queue_full" || mode == "debug_queue_full" {
            let queue_full = mode == "queue_full";
            let debug_queue_full = mode == "debug_queue_full";
            let (runtime, store, fs) = configured_fixture(|config| {
                config.request_timeout = Duration::from_secs(5);
                if debug_queue_full {
                    config.diagnostic_log_level = geesefs_rs::config::DiagnosticLogLevel::Debug;
                }
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
            for index in 0..if debug_queue_full {
                300
            } else if queue_full {
                18
            } else {
                1
            } {
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
            } else if debug_queue_full {
                assert!(dropped > 0., "debug bursts must expose bounded-output loss");
                assert!(
                    metric(&fs, "geesefs_diagnostic_severe_queue_dropped_total") > 0.,
                    "a severe burst must also expose exhaustion of its reserved queue"
                );
                assert_eq!(
                    metric(&fs, "geesefs_diagnostic_logs_suppressed_total"),
                    0.,
                    "debug must preserve severe reply candidates without rate sampling"
                );
            }
            return;
        }
        if mode == "background" {
            read_deadline_after_publication_preserves_bytes_without_a_second_reply();
            return;
        }
        if [
            "listing",
            "listing_cancelled",
            "listing_debug",
            "listing_debug_cancelled",
        ]
        .contains(&mode.as_str())
        {
            let cancelled = mode.ends_with("cancelled");
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
                if mode.starts_with("listing_debug") {
                    config.diagnostic_log_level = geesefs_rs::config::DiagnosticLogLevel::Debug;
                }
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
            } else if mode == "slow_info" {
                config.diagnostic_log_level = geesefs_rs::config::DiagnosticLogLevel::Info;
            } else if mode == "slow_error" {
                config.diagnostic_log_level = geesefs_rs::config::DiagnosticLogLevel::Error;
            }
        });
        let mut session = Session::new(fs.clone(), &runtime);
        if mode == "slow_warn" || mode == "slow_error" || mode == "slow_info" {
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
        } else if mode == "generation" || mode == "generation_burst" {
            for index in 0..if mode == "generation_burst" { 20 } else { 1 } {
                let (started, release) = store.pause_head();
                session.send(1, 1, b"private-request-path\0");
                runtime.block_on(async {
                    tokio::time::timeout(Duration::from_secs(1), started)
                        .await
                        .unwrap()
                        .unwrap();
                    fs.create(
                        1,
                        &format!("local-generation-{index}"),
                        libc::S_IFREG | 0o644,
                        None,
                        0,
                    )
                    .await
                    .unwrap();
                });
                release.send(()).unwrap();
                assert_eq!(errno(&session.receive()), -libc::EAGAIN);
            }
        } else {
            assert!(["backend", "backend_off", "backend_warn"].contains(&mode.as_str()));
            store
                .fail_head_once
                .store(true, std::sync::atomic::Ordering::Relaxed);
            session.send(1, 1, b"private-request-path\0");
            assert_eq!(errno(&session.receive()), -libc::EIO);
        }
        session.close();
        if mode == "generation_burst" {
            // Socket delivery precedes diagnostic accounting; joined session
            // shutdown establishes completion of every generation rejection.
            assert_eq!(metric(&fs, "geesefs_diagnostic_logs_suppressed_total"), 19.);
        }
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
        ("slow_info", "fuse_reply", "ok", 0, true),
        (
            "debug_queue_full",
            "fuse_reply",
            "backend_response",
            libc::EIO,
            true,
        ),
        (
            "listing_debug",
            "fuse_reply",
            "invalid_response",
            libc::EIO,
            true,
        ),
        (
            "listing_debug_cancelled",
            "fuse_reply",
            "deadline",
            libc::ETIMEDOUT,
            true,
        ),
        (
            "generation_burst",
            "fuse_reply",
            "missing_epoch_changed",
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
        if mode == "backend_off" || mode == "slow_error" || mode == "slow_warn" {
            assert!(records.is_empty(), "{mode}: {stderr}");
            continue;
        }
        if mode == "debug_queue_full" || mode.starts_with("listing_debug") {
            assert!(
                records
                    .iter()
                    .any(|record| record["level"] == "debug" && record["event"] == "phase_start"),
                "{stderr}"
            );
            if mode == "debug_queue_full" {
                assert!(
                    records.len() >= 256,
                    "debug queue preserves a bounded burst: {}",
                    records.len()
                );
            } else {
                let attempt = records
                    .iter()
                    .find(|record| record["event"] == "s3_attempt")
                    .expect("actual extension HTTP attempt");
                assert_eq!(attempt["details"]["operation"], "list");
                assert_eq!(attempt["details"]["transport"], "extension");
                assert_eq!(attempt["details"]["retry"], false);
                let phase = records
                    .iter()
                    .find(|record| {
                        record["event"] == "phase_end" && record["details"]["stage"] == "s3_list"
                    })
                    .expect("real LIST parse failure");
                assert_eq!(
                    phase["details"]["outcome"],
                    if mode.ends_with("cancelled") {
                        "cancelled"
                    } else {
                        "error"
                    }
                );
                assert_eq!(
                    phase["details"]["reason"],
                    if mode.ends_with("cancelled") {
                        "cancelled"
                    } else {
                        "invalid_response"
                    }
                );
                assert_eq!(phase["trace_id"], attempt["trace_id"]);
                assert!(records.iter().any(|record| record["event"] == "phase_start"
                    && record["details"]["scope_id"] == phase["details"]["scope_id"]));
            }
            for forbidden in [
                "private-request-path",
                "private-backend-text",
                "disposable-access",
                "disposable-secret",
                "Authorization",
                "http://",
                "bucket",
            ] {
                assert!(
                    !stderr.contains(forbidden),
                    "debug leaked {forbidden}: {stderr}"
                );
            }
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
            if mode == "slow_info" {
                "info"
            } else if mode == "generation_burst" || mode == "generation" {
                "warn"
            } else {
                "error"
            }
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

/// Same executable on/off control. This includes fuser wire dispatch, the client,
/// runtime and a gated-store implementation, not a kernel mount or real S3.
#[test]
#[ignore = "explicit observer-cost diagnostic; not mounted or Go-relative acceptance"]
fn observer_cost_diagnostic() {
    use std::{sync::atomic::Ordering, time::Instant};
    let mode = std::env::var("GEESEFS_OBSERVER_MODE").unwrap_or_else(|_| "cached".into());
    assert!(["cached", "cold", "metadata"].contains(&mode.as_str()));
    let enabled = std::env::var("GEESEFS_OBSERVER_ENABLED").unwrap_or_else(|_| "1".into()) == "1";
    let calls: usize = std::env::var("GEESEFS_OBSERVER_CALLS")
        .unwrap_or_else(|_| "25000".into())
        .parse()
        .unwrap();
    assert!((1000..=250000).contains(&calls));
    let (runtime, store, fs) = configured_fixture(|config| {
        config.request_timeout = Duration::from_secs(10);
        config.max_requests = 128;
        config.no_diagnostics = !enabled;
        config.stat_cache_ttl = if mode == "metadata" {
            Duration::ZERO
        } else {
            Duration::from_secs(3600)
        };
        config.read_ahead = 128;
        config.read_ahead_small = 128;
        config.read_ahead_large = 128;
        config.read_ahead_parallel = 128;
    });
    const SIZE: u64 = 1 << 40;
    store.seed("virtual", &[0; 16]);
    store.virtual_size.store(SIZE, Ordering::Relaxed);
    let ino = runtime.block_on(fs.lookup(1, "virtual")).unwrap().ino;
    let mut session = Session::new(fs.clone(), &runtime);
    let handle = open(&mut session, ino);
    let mut buffer = [0u8; 8192];
    let mut operation = |session: &mut Session, call: usize| {
        let request = if mode == "metadata" {
            session.send(3, ino, &[0; 16])
        } else {
            let offset = if mode == "cold" {
                call as u64 * 128 * 1024
            } else {
                0
            };
            read_range(session, ino, handle, offset, 4096)
        };
        let size = session.peer.recv(&mut buffer).unwrap();
        let reply = &buffer[..size];
        assert_eq!((unique(reply), errno(reply)), (request, 0));
        if mode == "metadata" {
            assert_eq!(u64::from_ne_bytes(reply[40..48].try_into().unwrap()), SIZE);
        } else {
            assert_eq!(&reply[16..], &[0u8; 4096]);
        }
    };
    for call in 0..200 {
        operation(&mut session, call);
    }
    runtime.block_on(runtime.spawn(async {})).unwrap();
    let before_heads = store.head_starts.load(Ordering::Relaxed);
    let before_reads = store.read_calls.load(Ordering::Relaxed);
    let cpu_ticks = || {
        let stat = std::fs::read_to_string("/proc/self/stat").unwrap();
        let fields: Vec<_> = stat
            .rsplit_once(')')
            .unwrap()
            .1
            .split_whitespace()
            .collect();
        [
            fields[11].parse::<u64>().unwrap(),
            fields[12].parse::<u64>().unwrap(),
        ]
    };
    let before_cpu = cpu_ticks();
    let start = Instant::now();
    let mut latencies = Vec::with_capacity(calls);
    for call in 200..200 + calls {
        let start = Instant::now();
        operation(&mut session, call);
        latencies.push(start.elapsed().as_nanos());
    }
    let wall = start.elapsed().as_nanos();
    runtime.block_on(runtime.spawn(async {})).unwrap();
    let after_cpu = cpu_ticks();
    let reads = store.read_calls.load(Ordering::Relaxed) - before_reads;
    let heads = store.head_starts.load(Ordering::Relaxed) - before_heads;
    assert_eq!(reads, if mode == "cold" { calls } else { 0 });
    assert_eq!(heads, if mode == "metadata" { calls } else { 0 });
    let start = Instant::now();
    let mut scrape_bytes = 0;
    let mut series = 0;
    for _ in 0..100 {
        let text = geesefs_rs::metrics::render(&fs).unwrap();
        scrape_bytes = text.len();
        series = text.lines().filter(|line| !line.starts_with('#')).count();
        assert!(!text.contains("virtual\""));
    }
    let scrape_ns = start.elapsed().as_nanos() / 100;
    let mut sorted = latencies.clone();
    sorted.sort_unstable();
    let percentile = |percent: usize| sorted[(sorted.len() * percent).div_ceil(100) - 1];
    println!(
        "observer_cost_json={}",
        serde_json::json!({
            "mode": mode, "enabled": enabled, "calls": calls, "wall_ns": wall,
            "cpu_user_ticks": after_cpu[0] - before_cpu[0], "cpu_system_ticks": after_cpu[1] - before_cpu[1],
            "clock_ticks_per_second": rustix::param::clock_ticks_per_second(),
            "latencies_ns": latencies, "p50_ns": percentile(50), "p95_ns": percentile(95), "p99_ns": percentile(99),
            "backend_heads": heads, "backend_reads": reads, "scrape_mean_ns": scrape_ns,
            "scrape_bytes": scrape_bytes, "series": series,
            "allocator": if cfg!(feature = "system-allocator") { "system" } else { "jemalloc" },
            "process_status": std::fs::read_to_string("/proc/self/status").unwrap().lines()
                .filter(|line| line.starts_with("VmRSS:") || line.starts_with("VmHWM:")).collect::<Vec<_>>(),
            "allocation": geesefs_rs::metrics::allocation_stats(),
            "scope": "whole fixture process: client, fuser, runtime and memory store; no kernel FUSE or S3"
        })
    );
    session.close();
}
