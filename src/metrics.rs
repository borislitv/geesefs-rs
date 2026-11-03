//! Bounded-cardinality, read-only observations. No paths, credentials or S3 I/O.
mod allocator;
mod server;

pub use allocator::allocation_stats;
pub use server::MetricsServer;

use prometheus_client::{
    encoding::text::encode,
    metrics::{counter::Counter, family::Family, gauge::Gauge},
    registry::Registry,
};
use std::sync::atomic::AtomicU64;

use crate::{cache::CacheStats, filesystem::Filesystem};

#[derive(Clone, Default)]
pub(crate) struct ReadMetrics {
    pub hits: Counter,
    pub misses: Counter,
    pub hit_bytes: Counter,
    pub miss_bytes: Counter,
    pub fill_jobs: Counter,
    pub fill_errors: Counter,
    pub remote_bytes: Counter,
}

pub(crate) struct Snapshot {
    pub(crate) namespace: (usize, usize, u64, u64),
    pub(crate) entry_limit: u64,
    pub(crate) cache: CacheStats,
    pub(crate) lanes: [(&'static str, usize, usize); 3],
    pub(crate) reads: ReadMetrics,
}

type Observation = Gauge<f64, AtomicU64>;

fn gauge(registry: &mut Registry, name: &str, help: &str, value: u64) {
    let gauge = Observation::default();
    gauge.set(value as f64);
    registry.register(name, help, gauge);
}

fn counter(registry: &mut Registry, name: &str, help: &str, value: u64) {
    let counter: Counter = Counter::default();
    counter.inc_by(value);
    registry.register(name, help, counter);
}

fn cache_metrics(registry: &mut Registry, cache: &CacheStats) {
    for (name, help, value) in [
        (
            "geesefs_read_cache_bytes",
            "Retained visible payload bytes; pinned backing may outlive eviction",
            cache.bytes,
        ),
        (
            "geesefs_read_cache_capacity_bytes",
            "Retained payload policy including revocable flush loans",
            cache.capacity,
        ),
        (
            "geesefs_read_cache_entries",
            "Retained payload spans",
            cache.entries as u64,
        ),
        (
            "geesefs_read_cache_entry_limit",
            "Maximum retained payload spans",
            cache.entry_limit as u64,
        ),
        (
            "geesefs_read_cache_loaned_bytes",
            "Retained bytes charged to the idle flush lane",
            cache.loaned_bytes as u64,
        ),
    ] {
        gauge(registry, name, help, value);
    }
    for (name, help, value) in [
        (
            "geesefs_read_cache_evictions",
            "Policy and loan-reclamation evictions; excludes replacement and clear",
            cache.evictions,
        ),
        (
            "geesefs_read_cache_evicted_bytes",
            "Visible bytes removed by policy or loan reclamation, not necessarily freed",
            cache.evicted_bytes,
        ),
        (
            "geesefs_read_cache_insertions",
            "Nonempty admitted cache insertions",
            cache.insertions,
        ),
        (
            "geesefs_read_cache_replacements",
            "Old spans removed for insertion at the same generation and offset",
            cache.replacements,
        ),
        (
            "geesefs_read_cache_oversized",
            "Insertions rejected for exceeding payload policy",
            cache.oversized,
        ),
    ] {
        counter(registry, name, help, value);
    }
}

fn filesystem_metrics(registry: &mut Registry, snapshot: Snapshot) {
    cache_metrics(registry, &snapshot.cache);
    let (entries, names, evictions, failures) = snapshot.namespace;
    gauge(
        registry,
        "geesefs_namespace_entries",
        "Live namespace inodes including root, not allocated bytes",
        entries as u64,
    );
    gauge(
        registry,
        "geesefs_namespace_names",
        "Live namespace name bindings",
        names as u64,
    );
    gauge(
        registry,
        "geesefs_namespace_entry_limit",
        "Configured namespace inode limit",
        snapshot.entry_limit,
    );
    counter(
        registry,
        "geesefs_namespace_evictions",
        "Inodes removed under entry-limit pressure",
        evictions,
    );
    counter(
        registry,
        "geesefs_namespace_capacity_failures",
        "Entry-limit admission failures with no eligible victim",
        failures,
    );
    let used = Family::<Vec<(&str, &str)>, Observation>::default();
    let limits = Family::<Vec<(&str, &str)>, Observation>::default();
    for (lane, bytes, limit) in snapshot.lanes {
        used.get_or_create(&vec![("lane", lane)]).set(bytes as f64);
        limits
            .get_or_create(&vec![("lane", lane)])
            .set(limit as f64);
    }
    registry.register(
        "geesefs_payload_used_bytes",
        "Charged byte permits, including pinned and in-flight owners, not RSS",
        used,
    );
    registry.register(
        "geesefs_payload_limit_bytes",
        "Configured byte admission by lane, not a process memory limit",
        limits,
    );
    for (name, help, metric) in [
        (
            "geesefs_read_cache_hits",
            "Nonempty caller reads served by the initial cache lookup",
            snapshot.reads.hits,
        ),
        (
            "geesefs_read_cache_misses",
            "Nonempty caller reads missing initially; followers may reuse in-flight fills",
            snapshot.reads.misses,
        ),
        (
            "geesefs_read_cache_hit_bytes",
            "Requested bytes served by an initial cache hit",
            snapshot.reads.hit_bytes,
        ),
        (
            "geesefs_read_cache_miss_bytes",
            "Requested bytes missing initially, including overlapping followers",
            snapshot.reads.miss_bytes,
        ),
        (
            "geesefs_read_fill_jobs",
            "Started read-window chunk jobs, not HTTP attempts or retries",
            snapshot.reads.fill_jobs,
        ),
        (
            "geesefs_read_fill_errors",
            "Failed read-window jobs, including optional read-ahead failures",
            snapshot.reads.fill_errors,
        ),
        (
            "geesefs_read_remote_bytes",
            "Validated clean-remote chunks consumed by read windows including read-ahead; excludes overlays and wire retries",
            snapshot.reads.remote_bytes,
        ),
    ] {
        registry.register(name, help, metric);
    }
}

fn memory_metrics(registry: &mut Registry) {
    let allocation = allocation_stats();
    if let Some(allocated) = allocation["allocated_bytes"].as_u64() {
        let name = if allocation["allocator"] == "jemalloc" {
            "geesefs_jemalloc_allocated_bytes"
        } else {
            "geesefs_system_requested_bytes"
        };
        gauge(
            registry,
            name,
            "Allocator-accounted bytes; not process RSS or payload occupancy",
            allocated,
        );
    }
    for name in [
        "active",
        "resident",
        "mapped",
        "retained_virtual",
        "metadata",
    ] {
        if let Some(value) = allocation["allocator_page_stats"][format!("{name}_bytes")].as_u64() {
            gauge(
                registry,
                &format!("geesefs_jemalloc_{name}_bytes"),
                "Jemalloc page accounting; retained virtual mappings are not physical memory",
                value,
            );
        }
    }
    if let Ok(status) = std::fs::read_to_string("/proc/self/status")
        && let Some(rss) = status.lines().find_map(|line| {
            line.strip_prefix("VmRSS:")?
                .split_whitespace()
                .next()?
                .parse::<u64>()
                .ok()
        })
    {
        gauge(
            registry,
            "process_resident_memory_bytes",
            "Process RSS sampled from Linux self status",
            rss * 1024,
        );
    }
}

/// OpenMetrics text accepted by Prometheus. Contended snapshots fail rather than
/// block a filesystem operation or expose misleading zero occupancy.
pub fn render(fs: &Filesystem) -> Option<String> {
    let snapshot = fs.metrics_snapshot()?;
    let mut registry = Registry::default();
    filesystem_metrics(&mut registry, snapshot);
    memory_metrics(&mut registry);
    let mut text = String::new();
    encode(&mut text, &registry).expect("encoding into a String cannot fail");
    Some(text)
}
