#![forbid(unsafe_code)]
mod support;

use geesefs_rs::{filesystem::Filesystem, fuse::Fuse};
use std::{
    os::unix::net::UnixDatagram,
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

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
    config.large_read_cutoff = 0;
    config.read_ahead_large = 1024;
    config.read_ahead_parallel = 512;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    (runtime, store, fs)
}

#[test]
fn lookup_deadline_returns_etimedout_and_returns_the_request_slot() {
    let (runtime, store, fs) = fixture();
    store.seed("slow", b"independent bytes");
    *store.head_gate.lock().unwrap() = Some(Arc::new(tokio::sync::Semaphore::new(0)));
    let mut session = Session::new(fs, &runtime);
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
    let mut body = handle.to_ne_bytes().to_vec();
    body.extend_from_slice(&0u64.to_ne_bytes()); // offset
    body.extend_from_slice(&128u32.to_ne_bytes());
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
    session.close();
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
