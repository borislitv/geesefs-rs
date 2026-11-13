//! Real buffered FUSE reads must wait for capacity, not become failed kernel pages.
mod support;

use geesefs_rs::filesystem::Filesystem;
use std::{
    fs::File,
    io::{self, Read},
    os::unix::{fs::FileExt, net::UnixStream},
    sync::{Arc, atomic::Ordering, mpsc},
    time::{Duration, Instant},
};
use support::{MemoryStore, native::mounted};

const OFFSET: u64 = 256 * 1024;
const LENGTH: usize = 4096;

#[derive(Clone, Copy, Debug)]
enum ReadKind {
    Buffered,
    Sendfile,
}

fn transfer(file: &File, kind: ReadKind) -> io::Result<Vec<u8>> {
    let mut bytes = vec![0; LENGTH];
    match kind {
        ReadKind::Buffered => file.read_exact_at(&mut bytes, OFFSET)?,
        ReadKind::Sendfile => {
            let (mut receiver, sender) = UnixStream::pair()?;
            receiver.set_read_timeout(Some(Duration::from_secs(5)))?;
            sender.set_write_timeout(Some(Duration::from_secs(5)))?;
            let mut offset = OFFSET;
            while offset < OFFSET + LENGTH as u64 {
                let remaining = (OFFSET + LENGTH as u64 - offset) as usize;
                if rustix::fs::sendfile(&sender, file, Some(&mut offset), remaining)? == 0 {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
            }
            receiver.read_exact(&mut bytes)?;
        }
    }
    Ok(bytes)
}

fn metric(fs: &Filesystem, name: &str) -> f64 {
    geesefs_rs::metrics::render(fs)
        .unwrap()
        .lines()
        .find_map(|line| {
            let (key, value) = line.rsplit_once(' ')?;
            (key == name).then(|| value.parse().unwrap())
        })
        .unwrap_or(0.)
}

fn wait_for(message: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !condition() {
        assert!(Instant::now() < deadline, "{message}");
        std::thread::yield_now();
    }
}

struct ReleaseOnDrop(Arc<tokio::sync::Semaphore>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.add_permits(1);
    }
}

fn cached_read_waits_for_actual_owner(kind: ReadKind) {
    let store = MemoryStore::new();
    let mut body = vec![0x19; 1024 * 1024];
    body[OFFSET as usize..OFFSET as usize + LENGTH].fill(0xa5);
    store.seed("payload", &body);
    let tail = Arc::new(tokio::sync::Semaphore::new(0));
    store
        .read_range_gates
        .lock()
        .unwrap()
        .insert((512 * 1024, 512 * 1024), tail.clone());
    let mut config = MemoryStore::config(32);
    config.max_requests = 1;
    config.large_read_cutoff = 0;
    config.read_ahead_large = 1024;
    config.read_ahead_parallel = 512;
    config.request_timeout = Duration::from_secs(10);
    config.shutdown_timeout = Duration::from_secs(10);
    config.metrics_listen = Some("127.0.0.1:0".parse().unwrap());
    let fs = Filesystem::new(config, store.clone()).unwrap();
    mounted(fs.clone(), |path| {
        std::thread::scope(|scope| {
            let release = ReleaseOnDrop(tail);
            let file = File::open(path.join("payload")).unwrap();
            // Prevent kernel speculative reads from populating the second page.
            // Application read-ahead remains enabled and owns the stalled tail.
            rustix::fs::fadvise(&file, 0, None, rustix::fs::Advice::Random).unwrap();
            let mut first = [0; LENGTH];
            file.read_exact_at(&mut first, 0).unwrap();
            assert_eq!(first, [0x19; LENGTH]);
            wait_for("read-ahead did not reach its gated tail", || {
                store.read_calls.load(Ordering::SeqCst) == 2
                    && metric(
                        &fs,
                        "geesefs_fuse_inflight{opcode=\"read\",state=\"after_reply\"}",
                    ) == 1.
            });
            let (finished, result) = mpsc::channel();
            let worker = scope.spawn(move || finished.send(transfer(&file, kind)).unwrap());
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut completed = None;
            let waited = loop {
                if let Ok(outcome) = result.try_recv() {
                    completed = Some(outcome);
                    break false;
                }
                if metric(
                    &fs,
                    "geesefs_diagnostic_events_total{reason=\"admission_wait\"}",
                ) > 0.
                {
                    break true;
                }
                assert!(
                    Instant::now() < deadline,
                    "second read was neither completed nor waiting"
                );
                std::thread::yield_now();
            };
            assert_eq!(store.read_calls.load(Ordering::SeqCst), 2);
            drop(release);
            let outcome =
                completed.unwrap_or_else(|| result.recv_timeout(Duration::from_secs(3)).unwrap());
            worker.join().unwrap();
            let actual = outcome.unwrap_or_else(|error| {
                panic!("{kind:?}: capacity pressure broke a valid cached read: {error}")
            });
            assert_eq!(actual, [0xa5; LENGTH]);
            assert!(
                waited,
                "the post-reply owner must retain its only request slot until completion"
            );
            assert_eq!(
                metric(
                    &fs,
                    "geesefs_fuse_replies_total{opcode=\"read\",errno=\"EAGAIN\"}"
                ),
                0.
            );
        });
    });
}

#[test]
#[ignore = "requires an explicitly authorized Linux FUSE fixture"]
fn native_buffered_read_waits_for_post_reply_admission() {
    cached_read_waits_for_actual_owner(ReadKind::Buffered);
}

#[test]
#[ignore = "requires an explicitly authorized Linux FUSE fixture"]
fn native_sendfile_waits_for_post_reply_admission() {
    cached_read_waits_for_actual_owner(ReadKind::Sendfile);
}
