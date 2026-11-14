//! Bounded-cardinality, read-only observations. No paths, credentials or S3 I/O.
mod allocator;
mod diagnostics;
mod disk_cache;
mod server;

pub use allocator::allocation_stats;
pub use diagnostics::Diagnostics;
pub(crate) use diagnostics::{
    BackendFailure, NamespaceGuard, Opcode, Stage, Trace, context, error_reason, namespace_lock,
    observe, scope,
};
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
    pub overlay_reads: Counter,
    pub source_bytes: [Counter; 4],
    pub fill_jobs: Counter,
    pub fill_errors: Counter,
    pub remote_bytes: Counter,
}

/// Requested bytes in one successful result, never its backing or prefetch span.
#[derive(Default)]
pub(crate) struct ReadBytes {
    pub l1: u64,
    pub l2: u64,
    pub remote: u64,
    pub overlay: u64,
}

impl ReadMetrics {
    pub(crate) fn cache_hit(&self, length: usize) {
        self.completed(ReadBytes {
            l1: length as u64,
            ..ReadBytes::default()
        });
    }

    pub(crate) fn completed(&self, bytes: ReadBytes) {
        let values = [bytes.l1, bytes.l2, bytes.remote, bytes.overlay];
        if values.iter().all(|&value| value == 0) {
            return;
        }
        if bytes.overlay > 0 {
            self.overlay_reads.inc();
        } else if bytes.remote > 0 {
            self.misses.inc();
        } else {
            self.hits.inc();
        }
        for (counter, value) in self.source_bytes.iter().zip(values) {
            counter.inc_by(value);
        }
    }
}

pub(crate) struct Snapshot {
    pub(crate) namespace: Option<(usize, usize, u64, u64)>,
    pub(crate) kernel_lookup_entries: u64,
    pub(crate) entry_limit: u64,
    pub(crate) cache: Option<CacheStats>,
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
            "geesefs_read_cache_capacity_evictions",
            "L1 payload/entry capacity victims; excludes loan reclamation and invalidation; not L2 demotions",
            cache.capacity_evictions,
        ),
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
    let available = Family::<Vec<(&str, &str)>, Observation>::default();
    for (component, present) in [
        ("namespace", snapshot.namespace.is_some()),
        ("read_cache", snapshot.cache.is_some()),
    ] {
        available
            .get_or_create(&vec![("component", component)])
            .set(u8::from(present) as f64);
    }
    registry.register("geesefs_metrics_snapshot_available", "Current component snapshot acquired without waiting; missing gauges are not zero occupancy", available);
    if let Some(cache) = &snapshot.cache {
        cache_metrics(registry, cache);
    }
    if let Some(namespace) = snapshot.namespace {
        namespace_metrics(registry, namespace);
    }
    independent_filesystem_metrics(registry, snapshot);
}

fn namespace_metrics(
    registry: &mut Registry,
    (entries, names, evictions, failures): (usize, usize, u64, u64),
) {
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
}

fn independent_filesystem_metrics(registry: &mut Registry, snapshot: Snapshot) {
    gauge(
        registry,
        "geesefs_namespace_kernel_lookup_entries",
        "Inodes with nonzero kernel lookup ownership, including root; not lookup reference count",
        snapshot.kernel_lookup_entries,
    );
    gauge(
        registry,
        "geesefs_namespace_entry_limit",
        "Configured namespace inode limit",
        snapshot.entry_limit,
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
    let sources = Family::<Vec<(&str, &str)>, Counter>::default();
    for (source, metric) in ["l1", "l2", "s3", "overlay"]
        .into_iter()
        .zip(&snapshot.reads.source_bytes)
    {
        sources
            .get_or_create(&vec![("source", source)])
            .inc_by(metric.get());
    }
    registry.register(
        "geesefs_read_delivered_bytes",
        "Requested bytes in successful userspace results by delivery source; excludes prefetch, failed and cancelled results; overlay may include remote materialization",
        sources,
    );
    counter(
        registry,
        "geesefs_read_cache_hit_bytes",
        "Requested bytes served from L1 or validated L2 in successful results, including cached portions of mixed reads",
        snapshot.reads.source_bytes[0].get() + snapshot.reads.source_bytes[1].get(),
    );
    counter(
        registry,
        "geesefs_read_cache_miss_bytes",
        "Requested S3 bytes in successful clean read results; excludes prefetch and writer overlays",
        snapshot.reads.source_bytes[2].get(),
    );
    for (name, help, metric) in [
        (
            "geesefs_read_cache_hits",
            "Successful nonempty reads served entirely from L1 or validated L2, including coalesced memory reuse",
            snapshot.reads.hits,
        ),
        (
            "geesefs_read_cache_misses",
            "Successful nonempty clean reads requiring any requested S3 bytes; mixed L2/S3 results count once",
            snapshot.reads.misses,
        ),
        (
            "geesefs_read_overlay_reads",
            "Successful nonempty writer-overlay results; excluded from clean cache hit/miss ratios",
            snapshot.reads.overlay_reads,
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

fn cpu_metrics(registry: &mut Registry) {
    let Ok(stat) = std::fs::read_to_string("/proc/self/stat") else {
        return;
    };
    let Some((_, fields)) = stat.rsplit_once(')') else {
        return;
    };
    let mut fields = fields.split_whitespace().skip(11);
    let ticks = fields
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .zip(fields.next().and_then(|value| value.parse::<u64>().ok()));
    let Some((user, system)) = ticks else {
        return;
    };
    let hz = rustix::param::clock_ticks_per_second() as f64;
    let cpu = Family::<Vec<(&str, &str)>, Counter<f64, AtomicU64>>::default();
    for (mode, ticks) in [("user", user), ("system", system)] {
        cpu.get_or_create(&vec![("mode", mode)])
            .inc_by(ticks as f64 / hz);
    }
    registry.register(
        "geesefs_process_cpu_seconds",
        "Daemon user/system CPU from Linux self stat; excludes other processes and unattributed cgroup work",
        cpu,
    );
}

/// OpenMetrics text accepted by Prometheus. Contended components are explicitly
/// unavailable; independent observations survive without waiting on filesystem locks.
pub fn render(fs: &Filesystem) -> Option<String> {
    let snapshot = fs.metrics_snapshot();
    let mut registry = Registry::default();
    disk_cache::register(&mut registry, fs, &snapshot);
    filesystem_metrics(&mut registry, snapshot);
    fs.diagnostics.register(&mut registry);
    memory_metrics(&mut registry);
    cpu_metrics(&mut registry);
    let mut text = String::new();
    encode(&mut text, &registry).expect("encoding into a String cannot fail");
    Some(text)
}
