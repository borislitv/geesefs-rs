#[cfg(not(feature = "system-allocator"))]
pub fn allocation_stats() -> serde_json::Value {
    use tikv_jemalloc_ctl::{epoch, stats};
    let available = epoch::advance().is_ok();
    serde_json::json!({
        "allocator": "jemalloc",
        "allocated_bytes": available.then(|| stats::allocated::read().ok()).flatten(),
        "allocated_bytes_scope": "jemalloc allocated size classes; excludes retained/free pages and external allocators",
        "allocation_counters": serde_json::Value::Null,
        "allocator_page_stats": if available { serde_json::json!({
            "provider": "tikv-jemalloc-ctl 0.7.0",
            "scope": "allocator page gauges, not process RSS; retained virtual mappings are not physical memory",
            "active_bytes": stats::active::read().ok(),
            "resident_bytes": stats::resident::read().ok(),
            "mapped_bytes": stats::mapped::read().ok(),
            "retained_virtual_bytes": stats::retained::read().ok(),
            "metadata_bytes": stats::metadata::read().ok(),
        }) } else { serde_json::Value::Null },
    })
}

#[cfg(feature = "system-allocator")]
pub fn allocation_stats() -> serde_json::Value {
    let stats = stats_alloc::INSTRUMENTED_SYSTEM.stats();
    serde_json::json!({
        "allocator": "system",
        "allocated_bytes": stats.bytes_allocated.checked_sub(stats.bytes_deallocated),
        "allocated_bytes_scope": "sampled net requested Rust global bytes; includes failed requests; non-atomic snapshot; excludes native allocations, allocator overhead and retention",
        "allocation_counters": {
            "provider": "stats_alloc 0.1.10",
            "bytes_allocated": stats.bytes_allocated,
            "bytes_deallocated": stats.bytes_deallocated,
            "allocations": stats.allocations,
            "deallocations": stats.deallocations,
            "reallocations": stats.reallocations,
            "bytes_reallocated": stats.bytes_reallocated,
        },
        "allocator_page_stats": serde_json::Value::Null,
    })
}
