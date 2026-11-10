//! Constant-size snapshots only: no inode walks, cache reads or admission owners.
use super::{Observation, Snapshot, counter, gauge};
use crate::{
    backend::cache_scope::CacheBypass,
    disk_cache::{Funding, LatencyStats, State, Stats},
    filesystem::Filesystem,
};
use prometheus_client::{
    metrics::{counter::Counter, family::Family},
    registry::Registry,
};
use std::sync::atomic::AtomicU64;

pub(super) fn register(registry: &mut Registry, fs: &Filesystem, snapshot: &Snapshot) {
    let state = fs.disk_cache_state();
    let stats = fs.disk_cache_stats();
    let funding = fs.disk_cache_funding();
    states(registry, state, fs.disk_cache_bypass_reason());
    gauge(
        registry,
        "geesefs_disk_cache_stats_available",
        "An attached adapter snapshot is available; absent observations outside disabled mode are omitted, not zero occupancy",
        u64::from(stats.is_some()),
    );
    if stats.is_some() || state.is_none() {
        adapter(registry, stats.as_ref());
    }
    counter(
        registry,
        "geesefs_disk_cache_hit_bytes",
        "Validated L2 bytes delivered into the read pipeline including alignment and read-ahead, not requested client bytes",
        fs.disk_cache_read_bytes(),
    );
    let funded = funding.as_ref().map_or(0, |value| value.required_bytes);
    gauge(
        registry,
        "geesefs_disk_cache_funding_bytes",
        "Configured payload-budget deduction from effective L1; recovery or adapter bypass may remain funded; not measured heap or RSS",
        funded as u64,
    );
    let l1 = snapshot
        .lanes
        .iter()
        .filter(|(lane, _, _)| *lane != "ingress")
        .map(|(_, _, limit)| *limit)
        .sum::<usize>()
        .saturating_sub(funded);
    gauge(
        registry,
        "geesefs_effective_l1_policy_bytes",
        "Payload cache policy ceiling including revocable flush loans, after disk funding; not current ownership",
        l1 as u64,
    );
    funding_metrics(registry, funding.as_ref());
    metadata(registry, fs);
    instrumentation(registry, stats.as_ref());
}

fn states(registry: &mut Registry, state: Option<State>, bypass: Option<CacheBypass>) {
    let values = Family::<Vec<(&str, &str)>, Observation>::default();
    for (name, active) in [
        ("disabled", state.is_none()),
        ("prepared", state == Some(State::Prepared)),
        ("recovering", state == Some(State::Recovering)),
        ("ready", state == Some(State::Ready)),
        ("closing", state == Some(State::Closing)),
        ("closed", state == Some(State::Closed)),
        ("bypass", state == Some(State::Bypass)),
    ] {
        values
            .get_or_create(&vec![("state", name)])
            .set(f64::from(active));
    }
    registry.register(
        "geesefs_disk_cache_state",
        "Observed cache lifecycle state; disabled is not Ready",
        values,
    );
    let reason = match bypass {
        None => "none",
        Some(CacheBypass::UnsupportedBackend) => "unsupported_backend",
        Some(CacheBypass::OpaqueCredentials) => "opaque_credentials",
        Some(CacheBypass::TemporaryCredentials) => "temporary_credentials",
        Some(CacheBypass::OpaqueAuthentication) => "opaque_authentication",
        Some(CacheBypass::UnresolvedEndpoint) => "unresolved_endpoint",
    };
    let reasons = Family::<Vec<(&str, &str)>, Observation>::default();
    for name in [
        "none",
        "unsupported_backend",
        "opaque_credentials",
        "temporary_credentials",
        "opaque_authentication",
        "unresolved_endpoint",
    ] {
        reasons
            .get_or_create(&vec![("reason", name)])
            .set(f64::from(name == reason));
    }
    registry.register(
        "geesefs_disk_cache_bypass_reason",
        "Closed-vocabulary backend identity bypass policy, not a credential/principal label",
        reasons,
    );
}

fn adapter(registry: &mut Registry, stats: Option<&Stats>) {
    for (name, help, value) in [
        (
            "geesefs_disk_cache_active_jobs",
            "Admitted active adapter read/spill jobs; not a queued-only count",
            stats.map_or(0, |s| s.active_jobs),
        ),
        (
            "geesefs_disk_cache_copy_owned_bytes",
            "Current admitted spill-copy allocation ownership; not cumulative copied traffic",
            stats.map_or(0, |s| s.copied_bytes),
        ),
        (
            "geesefs_disk_cache_index_entries",
            "Current guarded record-index entries; not measured index heap bytes",
            stats.map_or(0, |s| s.index_entries),
        ),
        (
            "geesefs_disk_cache_recent_entries",
            "Current bounded approximate-recency hints; not measured heap bytes",
            stats.map_or(0, |s| s.recent_entries),
        ),
    ] {
        gauge(registry, name, help, value as u64);
    }
    for (name, help, value) in [
        (
            "geesefs_disk_cache_submission_bytes",
            "Engine submission ownership including queued and active serialization/publication, not queued-only payload bytes",
            stats.map_or(0, |s| s.submission_bytes),
        ),
        (
            "geesefs_disk_cache_submission_items",
            "Engine submissions including queued and active publication, not the adapter job count",
            stats.map_or(0, |s| s.submission_items),
        ),
    ] {
        let scoped = Family::<Vec<(&str, &str)>, Observation>::default();
        scoped
            .get_or_create(&vec![("scope", "queued_and_active")])
            .set(value as f64);
        registry.register(name, help, scoped);
    }
    for (name, help, value) in [
        (
            "geesefs_disk_cache_admitted_demotions",
            "Funded exact-visible-range copies admitted this session; excludes index hints and rejected admission; not publication",
            stats.map_or(0, |s| s.admitted_demotions),
        ),
        (
            "geesefs_disk_cache_engine_admitted_demotions",
            "Store enqueue acceptances this session, not publication or durability acknowledgements",
            stats.map_or(0, |s| s.engine_admitted_demotions),
        ),
        (
            "geesefs_disk_cache_spill_no_space",
            "Completed enqueue NoSpace rejections, not storage errors",
            stats.map_or(0, |s| s.spill_no_space),
        ),
        (
            "geesefs_disk_cache_spill_errors",
            "Admitted spill enqueue or guarded-drain errors, excluding NoSpace",
            stats.map_or(0, |s| s.spill_errors),
        ),
        (
            "geesefs_disk_cache_region_reclaims_started",
            "Actual engine region picker callbacks; not completed recycling, per-record evictions or reclaimed bytes",
            stats.map_or(0, |s| s.region_reclaims_started),
        ),
        (
            "geesefs_disk_cache_read_operation_hits",
            "Worker-completed validated copied blocks, including results not delivered after caller cancellation",
            stats.map_or(0, |s| s.read_operation_hits),
        ),
        (
            "geesefs_disk_cache_read_operation_misses",
            "Worker-completed loads without a usable block or surfaced load error; independent of caller delivery",
            stats.map_or(0, |s| s.read_operation_misses),
        ),
        (
            "geesefs_disk_cache_read_operation_errors",
            "Worker-completed surfaced load errors, independent of caller cancellation",
            stats.map_or(0, |s| s.read_operation_errors),
        ),
        (
            "geesefs_disk_cache_hits",
            "Caller-delivered adapter block-read hits this session; pipeline generation checks may still reject a result; not worker-completed hits",
            stats.map_or(0, |s| s.hits),
        ),
        (
            "geesefs_disk_cache_misses",
            "Caller-delivered adapter block-read misses this session; not worker-completed misses or S3 request count",
            stats.map_or(0, |s| s.misses),
        ),
        (
            "geesefs_disk_cache_dropped",
            "Best-effort demotion drops/rejections including loan/size/copy/job/submission refusal; not capacity evictions",
            stats.map_or(0, |s| s.drops),
        ),
        (
            "geesefs_disk_cache_errors",
            "Adapter storage/read/decode errors, not every backend or hardware I/O error",
            stats.map_or(0, |s| s.errors),
        ),
        (
            "geesefs_disk_cache_io_read_bytes",
            "Engine-accounted read bytes; not physical device bytes or validated hit bytes",
            stats.map_or(0, |s| s.io_read_bytes),
        ),
        (
            "geesefs_disk_cache_io_write_bytes",
            "Engine-accounted write bytes including record overhead; not physical device bytes or admitted demotions",
            stats.map_or(0, |s| s.io_write_bytes),
        ),
    ] {
        counter(registry, name, help, value);
    }
    operation_latency(
        registry,
        "read",
        stats.map_or(LatencyStats::default(), |s| s.read_operations),
    );
    operation_latency(
        registry,
        "spill",
        stats.map_or(LatencyStats::default(), |s| s.spill_operations),
    );
    startup(registry, stats);
    let allocated = stats.and_then(|s| s.opened_file_allocated_bytes);
    gauge(
        registry,
        "geesefs_disk_cache_opened_file_observation_available",
        "Captured once-at-open file block observation is available; not live occupancy",
        u64::from(allocated.is_some()),
    );
    if let Some(bytes) = allocated {
        gauge(
            registry,
            "geesefs_disk_cache_opened_file_allocated_bytes",
            "Filesystem st_blocks times 512 captured once at open; not live occupancy, device allocation or cache residency",
            bytes,
        );
    }
}

fn operation_latency(registry: &mut Registry, operation: &str, value: LatencyStats) {
    counter(
        registry,
        &format!("geesefs_disk_cache_{operation}_operations"),
        "Completed job-admitted operations this session, including owned completion after caller cancellation; not rejected admission",
        value.count,
    );
    let total = Counter::<f64, AtomicU64>::default();
    total.inc_by(value.total_duration_ns as f64 / 1e9);
    registry.register(format!("geesefs_disk_cache_{operation}_operation_duration_seconds"), "Monotonic completed-operation duration including task queue and I/O; spill includes copying and joined drain, not per-record fsync; no percentile distribution", total);
    let max = Observation::default();
    max.set(value.max_duration_ns as f64 / 1e9);
    registry.register(format!("geesefs_disk_cache_{operation}_operation_duration_seconds_max"), "Maximum completed-operation duration this session, not a percentile or device service time", max);
}

fn startup(registry: &mut Registry, stats: Option<&Stats>) {
    let phases = stats.map_or(Default::default(), |s| s.startup);
    let counts = Family::<Vec<(&str, &str)>, Counter>::default();
    let totals = Family::<Vec<(&str, &str)>, Counter<f64, AtomicU64>>::default();
    let maxima = Family::<Vec<(&str, &str)>, Observation>::default();
    for (phase, value) in [
        ("startup", phases.startup),
        ("allocation", phases.allocation),
        ("recovery", phases.recovery),
    ] {
        let labels = vec![("phase", phase)];
        counts.get_or_create(&labels).inc_by(value.count);
        totals
            .get_or_create(&labels)
            .inc_by(value.total_duration_ns as f64 / 1e9);
        maxima
            .get_or_create(&labels)
            .set(value.max_duration_ns as f64 / 1e9);
    }
    registry.register("geesefs_disk_cache_open_phase_completions", "Completed singleton open phases for the attached session, not retries; unavailable without attached snapshot", counts);
    registry.register("geesefs_disk_cache_open_phase_duration_seconds", "Monotonic completed open phases: startup excludes prepare; allocation includes queue/validation/preallocation/sync; recovery is subsequent engine construction, not scan-only", totals);
    registry.register(
        "geesefs_disk_cache_open_phase_duration_seconds_max",
        "Maximum completed singleton phase duration, equal to its total; no latency quantiles",
        maxima,
    );
}

fn funding_metrics(registry: &mut Registry, funding: Option<&Funding>) {
    gauge(
        registry,
        "geesefs_disk_cache_managed_capacity_bytes",
        "Configured engine-managed capacity, not current physical allocated storage or occupancy",
        funding.map_or(0, |f| f.capacity) as u64,
    );
    gauge(
        registry,
        "geesefs_disk_cache_index_entry_limit",
        "Funded guarded index entry ceiling, not current occupancy",
        funding.map_or(0, |f| f.index_entries) as u64,
    );
    gauge(
        registry,
        "geesefs_disk_cache_recent_entry_limit",
        "Funded recency-hint entry ceiling, not current occupancy",
        funding.map_or(0, |f| f.recent_entries) as u64,
    );
    let components = Family::<Vec<(&str, &str)>, Observation>::default();
    for (component, value) in [
        (
            "fixed",
            funding.map_or(0, |f| f.fixed_bytes - f.recovery_bytes),
        ),
        ("recovery", funding.map_or(0, |f| f.recovery_bytes)),
        ("metadata", funding.map_or(0, |f| f.metadata_bytes)),
        ("copy", funding.map_or(0, |f| f.copy_bytes)),
        ("read_work", funding.map_or(0, |f| f.read_work_bytes)),
        ("submission", funding.map_or(0, |f| f.submission_bytes)),
    ] {
        components
            .get_or_create(&vec![("component", component)])
            .set(value as f64);
    }
    registry.register("geesefs_disk_cache_component_funding_bytes",
        "Nonoverlapping planned budget allowances summing to total funding; not measured heap or current allocation", components);
}

fn metadata(registry: &mut Registry, fs: &Filesystem) {
    let usage = fs.metadata_usage();
    gauge(
        registry,
        "geesefs_metadata_limit_enabled",
        "Application metadata charging enabled; not an RSS bound",
        u64::from(usage.limit.is_some()),
    );
    gauge(
        registry,
        "geesefs_metadata_limit_bytes",
        "Configured application metadata admission limit, zero when disabled; external overhead excluded",
        usage.limit.unwrap_or(0) as u64,
    );
    gauge(
        registry,
        "geesefs_metadata_charged_bytes",
        "Current application owner charges; excludes external overhead from SDK/parsers/allocator rounding; not measured heap",
        usage.charged as u64,
    );
    let available = Family::<Vec<(&str, &str)>, Observation>::default();
    available
        .get_or_create(&vec![("observation", "external_overhead_bytes")])
        .set(0.);
    registry.register(
        "geesefs_metadata_instrumentation_available",
        "Measured metadata observations not available from logical admission counters",
        available,
    );
}

fn instrumentation(registry: &mut Registry, stats: Option<&Stats>) {
    let available = Family::<Vec<(&str, &str)>, Observation>::default();
    for observation in [
        "capacity_evictions",
        "allocated_bytes",
        "index_heap_bytes",
        "latency_quantiles",
    ] {
        available
            .get_or_create(&vec![("observation", observation)])
            .set(0.);
    }
    for observation in [
        "demotions",
        "read_latency",
        "write_latency",
        "recovery_duration",
        "region_reclaims_started",
    ] {
        available
            .get_or_create(&vec![("observation", observation)])
            .set(f64::from(stats.is_some()));
    }
    registry.register("geesefs_disk_cache_instrumentation_available",
        "Observation family available from attached snapshot; allocated_bytes means live allocation (unsupported), not the separately captured once-at-open st_blocks observation; latency aggregates do not provide quantiles", available);
}
