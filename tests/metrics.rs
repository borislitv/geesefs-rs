#![forbid(unsafe_code)]
mod support;

use geesefs_rs::{
    budget::Budget,
    cache::ReadCache,
    filesystem::Filesystem,
    metrics::{MetricsServer, render},
};
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use support::MemoryStore;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[global_allocator]
#[cfg(not(feature = "system-allocator"))]
static ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
#[global_allocator]
#[cfg(feature = "system-allocator")]
static ALLOCATOR: &stats_alloc::StatsAlloc<std::alloc::System> = &stats_alloc::INSTRUMENTED_SYSTEM;

fn sample(text: &str, name: &str) -> f64 {
    text.lines()
        .find_map(|line| {
            let (key, value) = line.split_once(' ')?;
            (key == name).then(|| value.parse::<f64>().unwrap())
        })
        .unwrap_or_else(|| panic!("missing sample {name} in {text}"))
}

fn fixture() -> (Arc<MemoryStore>, Arc<Filesystem>) {
    let store = MemoryStore::new();
    store.seed("private-object-name", b"abcdefgh");
    let mut config = MemoryStore::config(32);
    config.read_ahead = 4;
    config.read_ahead_small = 4;
    config.read_ahead_large = 4;
    config.read_ahead_parallel = 4;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    (store, fs)
}

#[tokio::test]
async fn cold_miss_warm_hit_and_scrapes_have_distinct_accounting() {
    let (store, fs) = fixture();
    let ino = fs.lookup(1, "private-object-name").await.unwrap().ino;
    assert_eq!(fs.read(ino, 0, 3).await.unwrap().as_slice(), b"abc");
    assert_eq!(fs.read(ino, 3, 5).await.unwrap().as_slice(), b"defgh");
    assert_eq!(fs.read(ino, 8, 3).await.unwrap().as_slice(), b"");
    let before = fs.effective_budgets();
    let text = render(&fs).unwrap();
    for (name, expected) in [
        ("geesefs_read_cache_hits_total", 1.),
        ("geesefs_read_cache_misses_total", 1.),
        ("geesefs_read_cache_hit_bytes_total", 5.),
        ("geesefs_read_cache_miss_bytes_total", 3.),
        ("geesefs_read_remote_bytes_total", 8.),
        ("geesefs_read_fill_jobs_total", 1.),
        ("geesefs_read_fill_errors_total", 0.),
        ("geesefs_read_cache_bytes", 8.),
        ("geesefs_read_cache_entries", 1.),
        ("geesefs_namespace_entries", 2.),
    ] {
        assert_eq!(sample(&text, name), expected, "{name}");
    }
    assert!(text.ends_with("# EOF\n"));
    assert!(!text.contains("private-object-name"));
    assert!(!text.contains("fixture.invalid"));
    assert!(
        sample(&text, "process_resident_memory_bytes") > 1024. * 1024.,
        "RSS must be bytes, not KiB"
    );
    let allocated = if cfg!(feature = "system-allocator") {
        "geesefs_system_requested_bytes"
    } else {
        "geesefs_jemalloc_allocated_bytes"
    };
    assert!(sample(&text, allocated) > 0.);
    render(&fs).unwrap();
    assert_eq!(store.read_calls.load(Ordering::Relaxed), 1);
    assert_eq!(store.head_starts.load(Ordering::Relaxed), 1);
    assert_eq!(store.list_calls.load(Ordering::Relaxed), 0);
    assert_eq!(
        fs.effective_budgets().data_used_bytes,
        before.data_used_bytes
    );
    assert_eq!(
        sample(&text, "geesefs_payload_used_bytes{lane=\"data\"}"),
        before.data_used_bytes as f64
    );
}

#[test]
fn eviction_samples_do_not_claim_to_free_pinned_owners_or_count_replacement() {
    let budget = Budget::new(32).unwrap();
    let cache = ReadCache::with_entry_limit(8, 2);
    for (offset, bytes) in [(0, b"aaaa"), (4, b"bbbb")] {
        cache.insert(
            1,
            1,
            offset,
            budget.try_reserve(4).unwrap().copy(bytes).unwrap(),
        );
    }
    let pinned = cache.get(1, 1, 0).unwrap();
    // Make the pinned owner the eviction victim without releasing its charge.
    drop(cache.get(1, 1, 4));
    cache.insert(
        1,
        1,
        8,
        budget.try_reserve(4).unwrap().copy(b"cccc").unwrap(),
    );
    cache.insert(
        1,
        1,
        8,
        budget.try_reserve(4).unwrap().copy(b"dddd").unwrap(),
    );
    let stats = cache.try_stats().unwrap();
    assert_eq!(
        (
            stats.entries,
            stats.bytes,
            stats.capacity,
            stats.entry_limit
        ),
        (2, 8, 8, 2)
    );
    assert_eq!(
        (
            stats.evictions,
            stats.evicted_bytes,
            stats.replacements,
            stats.insertions
        ),
        (1, 4, 1, 4)
    );
    assert_eq!(pinned.as_slice(), b"aaaa");
    assert!(cache.get(1, 1, 0).is_none());
    assert_eq!(budget.used(), 12);
    cache.clear();
    assert_eq!(budget.used(), 4);
    let stats = cache.try_stats().unwrap();
    assert_eq!((stats.bytes, stats.entries, stats.evictions), (0, 0, 1));
    drop(pinned);
    assert_eq!(budget.used(), 0);
}

#[test]
fn entry_limit_and_oversized_insertions_are_not_mistaken_for_byte_capacity() {
    let budget = Budget::new(32).unwrap();
    let cache = ReadCache::with_entry_limit(16, 1);
    cache.insert(1, 1, 0, budget.try_reserve(2).unwrap().copy(b"ab").unwrap());
    cache.insert(1, 1, 2, budget.try_reserve(2).unwrap().copy(b"cd").unwrap());
    let stats = cache.try_stats().unwrap();
    assert_eq!(
        (
            stats.bytes,
            stats.capacity,
            stats.evictions,
            stats.evicted_bytes
        ),
        (2, 16, 1, 2)
    );
    cache.insert(
        1,
        1,
        2,
        budget
            .try_reserve(17)
            .unwrap()
            .copy(b"too big for cache")
            .unwrap(),
    );
    let stats = cache.try_stats().unwrap();
    assert_eq!(
        (
            stats.entries,
            stats.bytes,
            stats.oversized,
            stats.replacements
        ),
        (0, 0, 1, 1)
    );
    assert_eq!(stats.insertions, 2);
    assert_eq!(budget.used(), 0);
}

#[tokio::test]
async fn namespace_limit_pressure_is_visible_without_walking_inodes() {
    let store = MemoryStore::new();
    for key in ["one", "two", "three"] {
        store.seed(key, b"data");
    }
    let mut config = MemoryStore::config(32);
    config.entry_limit = 2;
    let fs = Filesystem::new(config, store).unwrap();
    let first = fs.lookup(1, "one").await.unwrap().ino;
    let pinned = fs.lookup(1, "one").await.unwrap();
    assert!(fs.lookup(1, "two").await.is_err());
    drop(pinned);
    let second = fs.lookup(1, "two").await.unwrap().ino;
    assert_ne!(first, second);
    let text = render(&fs).unwrap();
    assert_eq!(sample(&text, "geesefs_namespace_entries"), 2.);
    assert_eq!(sample(&text, "geesefs_namespace_names"), 2.);
    assert_eq!(sample(&text, "geesefs_namespace_entry_limit"), 2.);
    assert_eq!(
        sample(&text, "geesefs_namespace_capacity_failures_total"),
        1.
    );
    assert_eq!(sample(&text, "geesefs_namespace_evictions_total"), 1.);
}

#[tokio::test]
async fn failed_remote_fill_counts_error_without_fabricating_bytes() {
    let (store, fs) = fixture();
    let ino = fs.lookup(1, "private-object-name").await.unwrap().ino;
    store.objects.lock().unwrap().clear();
    assert!(fs.read(ino, 0, 3).await.is_err());
    let text = render(&fs).unwrap();
    assert_eq!(sample(&text, "geesefs_read_fill_jobs_total"), 1.);
    assert_eq!(sample(&text, "geesefs_read_fill_errors_total"), 1.);
    assert_eq!(sample(&text, "geesefs_read_remote_bytes_total"), 0.);
    assert_eq!(sample(&text, "geesefs_read_cache_bytes"), 0.);
    assert_eq!(
        sample(&text, "geesefs_payload_used_bytes{lane=\"data\"}"),
        0.
    );
}

#[tokio::test]
async fn http_scrape_routes_contention_bind_errors_and_shutdown_are_observable() {
    let (store, fs) = fixture();
    let server = MetricsServer::bind("127.0.0.1:0".parse().unwrap(), fs.clone())
        .await
        .unwrap();
    let address = server.address();
    assert!(MetricsServer::bind(address, fs.clone()).await.is_err());
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let url = format!("http://{address}/metrics");
    let response = client.get(&url).send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers()["content-type"],
        "application/openmetrics-text; version=1.0.0; charset=utf-8"
    );
    let text = response.text().await.unwrap();
    assert_eq!(sample(&text, "geesefs_namespace_entries"), 1.);
    assert!(text.ends_with("# EOF\n"));
    assert_eq!(client.post(&url).send().await.unwrap().status(), 405);
    assert_eq!(
        client
            .get(format!("http://{address}/other"))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    let (release, wait) = std::sync::mpsc::channel();
    let (entered, held) = tokio::sync::oneshot::channel();
    let locked_fs = fs.clone();
    let owner = std::thread::spawn(move || {
        let _namespace = locked_fs.namespace.lock().unwrap();
        entered.send(()).unwrap();
        wait.recv_timeout(Duration::from_secs(5)).unwrap();
    });
    held.await.unwrap();
    let busy = client.get(&url).send().await.unwrap().status();
    release.send(()).unwrap();
    owner.join().unwrap();
    assert_eq!(busy, 503);
    assert_eq!(client.get(&url).send().await.unwrap().status(), 200);
    assert_eq!(store.read_calls.load(Ordering::Relaxed), 0);
    assert_eq!(store.head_starts.load(Ordering::Relaxed), 0);
    assert_eq!(store.list_calls.load(Ordering::Relaxed), 0);
    server.shutdown().await;
    assert!(tokio::net::TcpStream::connect(address).await.is_err());
}

#[tokio::test]
async fn incomplete_http_connection_has_a_deadline_and_does_not_block_another_scrape() {
    let (_, fs) = fixture();
    let server = MetricsServer::bind("127.0.0.1:0".parse().unwrap(), fs)
        .await
        .unwrap();
    let mut stalled = tokio::net::TcpStream::connect(server.address())
        .await
        .unwrap();
    stalled
        .write_all(b"GET /metrics HTTP/1.1\r\nHost:")
        .await
        .unwrap();
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    assert_eq!(
        client
            .get(format!("http://{}/metrics", server.address()))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    let mut body = Vec::new();
    tokio::time::timeout(Duration::from_secs(6), stalled.read_to_end(&mut body))
        .await
        .expect("incomplete connection must close within five seconds")
        .unwrap();
    let mut active = tokio::net::TcpStream::connect(server.address())
        .await
        .unwrap();
    active
        .write_all(b"GET /metrics HTTP/1.1\r\nHost:")
        .await
        .unwrap();
    server.shutdown().await;
    let closed = tokio::time::timeout(Duration::from_secs(1), active.read_to_end(&mut body))
        .await
        .expect("shutdown must release active sockets");
    assert!(
        closed.is_ok()
            || closed.is_err_and(|error| error.kind() == std::io::ErrorKind::ConnectionReset)
    );
}
