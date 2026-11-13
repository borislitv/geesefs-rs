//! Short opt-in FUSE/SDK/real-media check, without production data or cache drops.
mod support;

use aws_sdk_s3::{
    Client,
    config::{BehaviorVersion, Credentials, Region, retry::RetryConfig},
};
use geesefs_rs::{
    backend::s3::S3,
    budget::Budget,
    disk_cache::{DiskCache, PreparedDiskCache},
    filesystem::Filesystem,
};
use std::{
    io::{Cursor, Read},
    os::unix::fs::PermissionsExt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tiny_http::{Header, Response, Server, StatusCode};

const MIB: usize = 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(10);

type Exchange = (String, String, Option<String>);

struct Objects {
    endpoint: String,
    requests: Arc<Mutex<Vec<Exchange>>>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Objects {
    fn start(payload: Vec<u8>) -> Self {
        let server = Server::http("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", server.server_addr());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let worker = std::thread::spawn(move || {
            let pressure = vec![0x71; 48 * MIB];
            while !stopped.load(Ordering::Relaxed) {
                let Some(request) = server.recv_timeout(Duration::from_millis(100)).unwrap() else {
                    continue;
                };
                let method = request.method().to_string();
                let path = request.url().to_owned();
                let requested_range = request
                    .headers()
                    .iter()
                    .find(|h| h.field.equiv("Range"))
                    .map(|header| header.value.to_string());
                captured
                    .lock()
                    .unwrap()
                    .push((method.clone(), path.clone(), requested_range));
                let object = match path.split('?').next().unwrap() {
                    "/bucket/prefix/reused.rpm" => Some(payload.as_slice()),
                    "/bucket/prefix/pressure.txt" => Some(pressure.as_slice()),
                    _ => None,
                };
                let Some(object) = object else {
                    request.respond(Response::empty(StatusCode(404))).unwrap();
                    continue;
                };
                let mut headers =
                    vec![Header::from_bytes("ETag", "\"immutable-generation\"").unwrap()];
                if method == "HEAD" {
                    request
                        .respond(
                            Response::new(
                                StatusCode(200),
                                headers,
                                Cursor::new(Vec::<u8>::new()),
                                Some(object.len()),
                                None,
                            )
                            .with_chunked_threshold(usize::MAX),
                        )
                        .unwrap();
                    continue;
                }
                assert_eq!(method, "GET");
                let range = request
                    .headers()
                    .iter()
                    .find(|h| h.field.equiv("Range"))
                    .unwrap()
                    .value
                    .as_str();
                let (start, end) = range
                    .strip_prefix("bytes=")
                    .unwrap()
                    .split_once('-')
                    .unwrap();
                let start: usize = start.parse().unwrap();
                let end: usize = end.parse().unwrap();
                assert!(start <= end && end < object.len());
                headers.push(
                    Header::from_bytes(
                        "Content-Range",
                        format!("bytes {start}-{end}/{}", object.len()),
                    )
                    .unwrap(),
                );
                let bytes = object[start..=end].to_vec();
                // Client cancellation of optional prefetch is legitimate.
                let _ = request.respond(
                    Response::new(
                        StatusCode(206),
                        headers,
                        Cursor::new(bytes),
                        Some(end - start + 1),
                        None,
                    )
                    .with_chunked_threshold(usize::MAX),
                );
            }
        });
        Self {
            endpoint,
            requests,
            stop,
            worker: Some(worker),
        }
    }
    fn count(&self, method: &str) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(actual, _, _)| actual == method)
            .count()
    }
}
impl Drop for Objects {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}

fn metric(fs: &Filesystem, name: &str) -> u64 {
    geesefs_rs::metrics::render(fs)
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{name} ")))
        .unwrap_or_else(|| panic!("missing {name}"))
        .parse()
        .unwrap()
}

#[test]
#[ignore = "explicit disposable Linux FUSE and rsync fixture; under a minute"]
fn native_rsync_reuses_complete_l2_victims_without_extra_s3_requests() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();
    let payload: Vec<_> = (1..=16u8).flat_map(|byte| vec![byte; 128 * 1024]).collect();
    let objects = Objects::start(payload.clone());
    let directory = tempfile::tempdir().unwrap();
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let output = tempfile::tempdir().unwrap();
    let configured_mount = tempfile::tempdir().unwrap();
    let mut config = support::MemoryStore::config(32);
    config.mountpoint = configured_mount.path().into();
    config.entry_limit = 100_000;
    config.read_ahead = 2048;
    config.read_ahead_small = 2048;
    config.read_ahead_large = 2048;
    config.read_ahead_parallel = 2048;
    config.disk_cache_dir = Some(directory.path().into());
    config.disk_cache_max_size = Some(128 * MIB as u64);
    config.disk_cache_regex = Some(r"\.rpm$".parse().unwrap());
    config.no_detect = true;
    config.metrics_listen = Some("127.0.0.1:0".parse().unwrap());
    let store = runtime.block_on(async {
        let credentials = Credentials::new(
            "disposable-access",
            "disposable-secret",
            None,
            None,
            "test-only",
        );
        let sdk = aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new("us-east-1"))
            .endpoint_url(&objects.endpoint)
            .force_path_style(true)
            .credentials_provider(credentials.clone())
            .retry_config(RetryConfig::standard().with_max_attempts(1))
            .build();
        S3::new(Client::from_conf(sdk), "bucket".into(), "prefix".into())
            .unwrap()
            .with_static_credentials(credentials)
            .with_options(&config)
            .unwrap()
            .initialize()
            .await
            .unwrap()
    });
    runtime.block_on(async {
        let media = DiskCache::open(
            PreparedDiskCache::prepare(directory.path(), 128 * MIB as u64).unwrap(),
            &Budget::new(32 * MIB).unwrap(),
        )
        .await
        .unwrap();
        media.close(DEADLINE).await.unwrap();
    });
    for phase in 0..2 {
        let fs = runtime.block_on(async {
            let fs = Filesystem::new(config.clone(), Arc::new(store.clone())).unwrap();
            fs.attach_disk_cache(
                PreparedDiskCache::prepare(directory.path(), 128 * MIB as u64).unwrap(),
            )
            .await
            .unwrap();
            fs
        });
        let gets = objects.count("GET");
        let heads = objects.count("HEAD");
        support::native::mounted(fs.clone(), |mount| {
            assert_eq!(std::fs::read(mount.join("reused.rpm")).unwrap(), payload);
            let after_read = objects.requests.lock().unwrap().len();
            let copied = std::process::Command::new("timeout")
                .args([
                    "--signal=TERM",
                    "--kill-after=2",
                    "10s",
                    "rsync",
                    "--checksum",
                    "--no-whole-file",
                ])
                .arg(mount.join("reused.rpm"))
                .arg(output.path())
                .output()
                .unwrap();
            assert!(
                copied.status.success(),
                "{}",
                String::from_utf8_lossy(&copied.stderr)
            );
            assert_eq!(
                std::fs::read(output.path().join("reused.rpm")).unwrap(),
                payload
            );
            assert_eq!(
                objects.requests.lock().unwrap().len(),
                after_read,
                "warm reads/rsync must not issue GET, HEAD or LIST within TTL"
            );
            if phase == 0 {
                let mut pressure = std::fs::File::open(mount.join("pressure.txt")).unwrap();
                let mut buffer = [0; 128 * 1024];
                let mut total = 0;
                loop {
                    let count = pressure.read(&mut buffer).unwrap();
                    if count == 0 {
                        break;
                    }
                    assert_eq!(&buffer[..count], &vec![0x71; count]);
                    total += count;
                }
                assert_eq!(total, 48 * MIB);
            } else {
                assert_eq!(
                    objects.count("GET"),
                    gets,
                    "empty-L1 remount must read every requested payload byte from L2"
                );
                assert_eq!(
                    objects.count("HEAD") - heads,
                    1,
                    "one current-generation HEAD, not one per block"
                );
                assert_eq!(
                    metric(&fs, "geesefs_read_delivered_bytes_total{source=\"s3\"}"),
                    0
                );
                let l2 = metric(&fs, "geesefs_read_delivered_bytes_total{source=\"l2\"}");
                let l1 = metric(&fs, "geesefs_read_delivered_bytes_total{source=\"l1\"}");
                assert!(l2 > 0, "the fresh reader must actually deliver L2 bytes");
                // L2 read-ahead promotes the remaining records to L1 before
                // later FUSE callbacks request them. Count each requested byte
                // at its actual delivery boundary, not at its historical origin.
                // KEEP_CACHE is unset on open. The explicit full read and rsync
                // --checksum each request one full pass; only the first needs L2.
                assert_eq!(l1 + l2, 2 * payload.len() as u64);
                assert_eq!(fs.disk_cache_read_bytes(), payload.len() as u64);
                assert_eq!(metric(&fs, "geesefs_read_cache_misses_total"), 0);
                for method in ["get", "head"] {
                    assert_eq!(
                        metric(
                            &fs,
                            &format!(
                                "geesefs_s3_requests_total{{operation=\"{method}\",transport=\"sdk\"}}"
                            )
                        ),
                        objects.count(&method.to_uppercase()) as u64,
                        "execution counters must match requests actually received by the S3 fixture"
                    );
                }
                assert_eq!(
                    metric(&fs, "geesefs_s3_body_bytes_total"),
                    (50 * MIB) as u64
                );
            }
        });
        if phase == 0 {
            eprintln!(
                "cold source exchanges: {:?}",
                objects.requests.lock().unwrap()
            );
            assert_eq!(
                objects.count("GET"),
                25,
                "each 2-MiB source window is fetched only once"
            );
        }
    }
}
