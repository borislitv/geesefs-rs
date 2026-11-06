//! In-process namespace diagnostic, not mounted filesystem acceptance.
mod support;

use geesefs_rs::{filesystem::Filesystem, inode::Inode};
use serde_json::json;
use std::{
    sync::{Barrier, atomic::Ordering},
    time::{Duration, Instant},
};
use support::MemoryStore;

fn memory_kib() -> serde_json::Value {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let field = |name: &str| -> u64 {
        status
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap()
    };
    json!({"rss": field("VmRSS:"), "hwm": field("VmHWM:")})
}

fn cpu_ticks() -> (u64, u64) {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap();
    let fields: Vec<_> = stat[stat.rfind(')').unwrap() + 2..]
        .split_whitespace()
        .collect();
    (fields[11].parse().unwrap(), fields[12].parse().unwrap())
}

fn latency(samples: Vec<u128>) -> serde_json::Value {
    let mut sorted = samples.clone();
    sorted.sort_unstable();
    let percentile = |p: usize| sorted[(sorted.len() * p).div_ceil(100) - 1];
    json!({"unit": "ns", "samples": samples, "p50": percentile(50),
        "p95": percentile(95), "p99": percentile(99)})
}

#[test]
#[ignore = "explicit large-namespace diagnostic; no FUSE or performance-acceptance claim"]
fn namespace_scale_diagnostic() {
    let entries: usize = std::env::var("GEESEFS_SCALE_ENTRIES")
        .unwrap_or_else(|_| "100000".into())
        .parse()
        .unwrap();
    assert!((1_000..=500_000).contains(&entries));
    let store = MemoryStore::new();
    store.seed("tree/anchor", b"anchor");
    let mut config = MemoryStore::config(32);
    config.entry_limit = (entries + 16) as u64;
    config.stat_cache_ttl = Duration::ZERO;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    assert_eq!(rt.block_on(fs.lookup(1, "tree")).unwrap().ino, 2);
    // Model a previously populated clean cache, without including S3 fixture
    // allocation or population work in the measurements. Both revisions use
    // identical inode state and canonical name bindings.
    {
        let mut ns = fs.namespace.lock().unwrap();
        for index in 0..entries {
            let key = format!("tree/deep/file-{index:08}");
            ns.install(&key, |ino| {
                Inode::new(ino, key.clone(), false, None, &fs.config)
            })
            .unwrap();
        }
    }
    let memory_before = memory_kib();
    let cpu_before = cpu_ticks();
    let mut idle = Vec::new();
    let mut dirty = Vec::new();
    let mut lookup = Vec::new();
    let mut listing = Vec::new();
    rt.block_on(async {
        for _ in 0..32 {
            let start = Instant::now();
            fs.flush_all().await.unwrap();
            idle.push(start.elapsed().as_nanos());
        }
        let file = fs
            .create(1, "dirty", libc::S_IFREG | 0o644, None, 0)
            .await
            .unwrap();
        for index in 0u64..32 {
            let bytes = index.to_le_bytes();
            fs.write(file.ino, 0, &bytes).await.unwrap();
            let start = Instant::now();
            fs.flush_all().await.unwrap();
            dirty.push(start.elapsed().as_nanos());
            assert_eq!(store.bytes("dirty"), bytes);
        }
        for _ in 0..32 {
            let start = Instant::now();
            assert_eq!(fs.lookup(1, "tree").await.unwrap().ino, 2);
            lookup.push(start.elapsed().as_nanos());
            let start = Instant::now();
            let page = fs.directory_chunk(1, "", 1).await.unwrap();
            listing.push(start.elapsed().as_nanos());
            assert_eq!(
                page.iter()
                    .map(|entry| entry.name.as_str())
                    .collect::<Vec<_>>(),
                ["dirty"]
            );
        }
    });
    let workers = 8;
    let calls_per_worker = 25_000;
    let barrier = Barrier::new(workers + 1);
    let resolve_ns = std::thread::scope(|scope| {
        let threads: Vec<_> = (0..workers)
            .map(|worker| {
                let fs = &fs;
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    for call in 0..calls_per_worker {
                        let ino = 3 + ((worker * 997 + call) % entries) as u64;
                        assert_eq!(std::hint::black_box(fs.node(ino).unwrap()).ino, ino);
                    }
                })
            })
            .collect();
        barrier.wait();
        let start = Instant::now();
        for thread in threads {
            thread.join().unwrap();
        }
        start.elapsed().as_nanos()
    });
    let cpu_after = cpu_ticks();
    assert_eq!(store.put_calls.load(Ordering::Relaxed), 32);
    assert_eq!(store.delete_calls.load(Ordering::Relaxed), 0);
    assert_eq!(store.read_calls.load(Ordering::Relaxed), 0);
    println!(
        "namespace_scale_json={}",
        json!({
            "entries": entries, "allocator": "system (test harness)",
            "memory_before_kib": memory_before, "memory_after_kib": memory_kib(),
            "cpu_user_ticks": cpu_after.0 - cpu_before.0,
            "cpu_system_ticks": cpu_after.1 - cpu_before.1,
            "idle_flush": latency(idle), "dirty_flush": latency(dirty),
            "directory_lookup": latency(lookup), "root_listing": latency(listing),
            "resolver": {"workers": workers, "calls": workers * calls_per_worker, "wall_ns": resolve_ns},
            "backend": {"puts": 32, "deletes": 0, "reads": 0,
                "heads": store.head_keys.lock().unwrap().len(),
                "lists": store.list_calls.load(Ordering::Relaxed)}
        })
    );
}

#[test]
#[ignore = "explicit concurrent warm-cache diagnostic; no FUSE acceptance claim"]
fn cache_scale_diagnostic() {
    let fragment_kib: usize = std::env::var("GEESEFS_CACHE_FRAGMENT_KIB")
        .unwrap_or_else(|_| "64".into())
        .parse()
        .unwrap();
    assert!([64, 1024].contains(&fragment_kib));
    let workers = 8;
    let calls = 256;
    let length = 1024 * 1024;
    let store = MemoryStore::new();
    let mut config = MemoryStore::config(64);
    config.read_ahead = 1024;
    config.read_ahead_small = 1024;
    config.read_ahead_large = 1024;
    config.read_ahead_parallel = fragment_kib;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut files = Vec::new();
    for worker in 0..workers {
        let name = format!("file-{worker}");
        let expected = vec![worker as u8; length];
        store.seed(&name, &expected);
        let file = rt.block_on(fs.lookup(1, &name)).unwrap();
        assert_eq!(
            rt.block_on(fs.read(file.ino, 0, length))
                .unwrap()
                .as_slice(),
            expected
        );
        files.push((file, expected));
    }
    let reads_before = store.read_calls.load(Ordering::Relaxed);
    let cpu_before = cpu_ticks();
    let barrier = Barrier::new(workers + 1);
    let (wall_ns, latencies) = std::thread::scope(|scope| {
        let threads: Vec<_> = files
            .iter()
            .map(|(file, expected)| {
                let fs = &fs;
                let barrier = &barrier;
                scope.spawn(move || {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .unwrap();
                    barrier.wait();
                    let mut samples = Vec::new();
                    for _ in 0..calls {
                        let start = Instant::now();
                        let block = rt.block_on(fs.read(file.ino, 0, length)).unwrap();
                        samples.push(start.elapsed().as_nanos());
                        assert_eq!(block.as_slice(), expected);
                    }
                    latency(samples)
                })
            })
            .collect();
        barrier.wait();
        let start = Instant::now();
        let samples: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        (start.elapsed().as_nanos(), samples)
    });
    let cpu_after = cpu_ticks();
    assert_eq!(store.read_calls.load(Ordering::Relaxed), reads_before);
    assert_eq!(store.put_calls.load(Ordering::Relaxed), 0);
    println!(
        "cache_scale_json={}",
        json!({
            "fragment_kib": fragment_kib, "workers": workers, "calls_per_worker": calls,
            "bytes_per_call": length, "wall_ns": wall_ns, "reader_latencies": latencies,
            "cpu_user_ticks": cpu_after.0 - cpu_before.0,
            "cpu_system_ticks": cpu_after.1 - cpu_before.1,
            "memory_after_kib": memory_kib(), "backend_reads": reads_before,
            "allocator": "system (test harness)"
        })
    );
}
