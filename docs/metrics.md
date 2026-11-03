# Prometheus observations

The optional listener serves OpenMetrics 1.0 text at `GET /metrics`, accepted by
Prometheus. Enable it explicitly when starting a new mount:

```bash
geesefs-rs -f --metrics-listen 127.0.0.1:9469 bucket /mountpoint
curl --max-time 3 http://127.0.0.1:9469/metrics
```

The default is disabled. The address must be numeric; IPv6 uses `[::1]:9469`.
Port zero selects an ephemeral port, reported on stderr. A bind error fails
startup before mounting. The listener stops with the daemon. For fstab helper
normalization, include `--metrics-listen=127.0.0.1:9469` among the `-o` options.

The endpoint has no authentication or TLS. Use loopback, or an explicitly protected
management listener/reverse proxy when collecting remotely. It exports no object
keys, mount paths, bucket names, credentials or per-client labels.

For a Prometheus agent on the same host:

```yaml
scrape_configs:
  - job_name: geesefs
    scrape_interval: 30s
    scrape_timeout: 3s
    static_configs:
      - targets: ['127.0.0.1:9469']
```

There are at most two active HTTP connections, with a five-second total connection
deadline, 16 headers and an 8 KiB request buffer. Connections close after the
response. Unknown paths return 404, other methods 405. A contended namespace/cache
snapshot returns 503 rather than waiting behind filesystem work or publishing
false zeroes. Watch Prometheus `up` for failed scrapes.

## Metric contract

Counters reset on daemon restart. Gauges are observations, not a globally atomic
transaction. The only metric label is `lane`, with three fixed values: `data`,
`ingress`, `flush`. Scraping never walks inode contents, refreshes TTLs, changes LRU
recency, admits payload bytes or calls S3. Process status and allocator statistics
are sampled locally. Optional unavailable allocator/process values are omitted.

### Payload cache

| Metric | Meaning |
| --- | --- |
| `geesefs_read_cache_bytes` | Retained visible span bytes; not total charged backing storage |
| `geesefs_read_cache_capacity_bytes` | Retention policy including revocable idle-flush loans |
| `geesefs_read_cache_entries` / `geesefs_read_cache_entry_limit` | Retained spans and their entry cap |
| `geesefs_read_cache_loaned_bytes` | Retained visible bytes currently charged to the flush lane |
| `geesefs_read_cache_evictions_total` | Byte/entry/admission-pressure and loan-reclamation evictions |
| `geesefs_read_cache_evicted_bytes_total` | Visible bytes removed by those evictions, not necessarily physically freed |
| `geesefs_read_cache_insertions_total` | Nonempty admitted span insertions |
| `geesefs_read_cache_replacements_total` | Old spans removed for the same inode/generation/offset |
| `geesefs_read_cache_oversized_total` | Insertions rejected for exceeding retention capacity |
| `geesefs_payload_used_bytes{lane="…"}` / `geesefs_payload_limit_bytes{lane="…"}` | Charged permits and hard byte admission per lane, including pinned/in-flight owners |

Explicit cache clearing and same-key replacements are not policy evictions.
An evicted block can remain charged while a reader owns its backing. Byte admission,
retention policy, span count and process RSS are different quantities.

### Read observations

| Metric | Meaning |
| --- | --- |
| `geesefs_read_cache_hits_total` / `geesefs_read_cache_misses_total` | Initial cache result for each nonempty admitted caller read |
| `geesefs_read_cache_hit_bytes_total` / `geesefs_read_cache_miss_bytes_total` | EOF-clipped requested bytes in each initial result |
| `geesefs_read_fill_jobs_total` | Started read-window chunk jobs, including optional read-ahead and local overlays |
| `geesefs_read_fill_errors_total` | Failed chunk jobs; includes optional read-ahead failures |
| `geesefs_read_remote_bytes_total` | Validated clean-remote chunks consumed by read windows, including read-ahead |

An initial miss remains a miss when it subsequently reuses another in-flight fill;
internal rechecks are not additional requests. EOF/empty reads and requests rejected
before cache admission are excluded. Remote bytes exclude overlays, SDK retry/wire
overhead and metadata/list/write operations. Jobs are not HTTP attempts, completed
HTTP responses or a universal S3 request counter. Cancellation is not fabricated
as a backend error. Valid prefixes can remain counted after a late tail failure.

These observe filesystem reads reaching userspace. Kernel page-cache and nginx
cache hits can bypass this path; this is not an HTTP client hit ratio.

### Namespace and process memory

| Metric | Meaning |
| --- | --- |
| `geesefs_namespace_entries` / `geesefs_namespace_names` | Live inode/name-map lengths, including root |
| `geesefs_namespace_entry_limit` | Configured inode limit |
| `geesefs_namespace_evictions_total` | Inodes removed under entry-limit pressure |
| `geesefs_namespace_capacity_failures_total` | Inode admission failures without an eligible victim |
| `process_resident_memory_bytes` | Linux process RSS, not whole-cgroup memory |
| `geesefs_jemalloc_allocated_bytes` | Allocated size classes in the default allocator |
| `geesefs_jemalloc_active_bytes` / `geesefs_jemalloc_resident_bytes` | Active/allocator-resident page accounting |
| `geesefs_jemalloc_mapped_bytes` / `geesefs_jemalloc_retained_virtual_bytes` | Allocator virtual mappings; retained virtual bytes are not physical memory |
| `geesefs_jemalloc_metadata_bytes` | Allocator metadata pages |
| `geesefs_system_requested_bytes` | System-allocator build only: sampled net requested Rust bytes, excluding native allocations/retention |

Namespace cardinality is not metadata heap size. Negative-entry contents and
directory page contents are not scanned. Allocator observations reuse the
`--stats-file` JSON provider; neither allocator accounting nor the payload budget
is a process-memory ceiling. Collect cgroup memory/events/pressure independently.

## Deciding whether a larger cache helps

Collect a representative sustained workload at the existing limits first. Inspect
the following per instance, not a mixture of unrelated mounts:

```promql
geesefs_read_cache_bytes / geesefs_read_cache_capacity_bytes

rate(geesefs_read_cache_hit_bytes_total[15m])
/
clamp_min(rate(geesefs_read_cache_hit_bytes_total[15m]) + rate(geesefs_read_cache_miss_bytes_total[15m]), 1)

rate(geesefs_read_cache_evicted_bytes_total[15m])

geesefs_read_cache_entries / geesefs_read_cache_entry_limit

geesefs_payload_used_bytes / geesefs_payload_limit_bytes

rate(geesefs_read_remote_bytes_total[15m])
```

A full cache, recurring misses and substantial evictions justify an experiment,
not an automatic increase: sequential one-pass reads can churn at any size.
Distinguish span-count pressure, byte admission, revoked flush loans and metadata
pressure. More payload bytes will not correct an entry-count limit, TTL refresh,
backend errors, allocator retention or unrelated metadata growth.

Compare unchanged-budget and larger-budget windows with equivalent traffic,
generations, startup/steady state and independent repetitions. Require improved
cache reuse/backend traffic and user-visible tails/throughput without unacceptable
RSS/cgroup pressure. These counters alone do not estimate the whole working set
or prove a benefit from doubling memory. Existing binaries without this listener
provide no historical hit/miss/occupancy series; deployment and collection remain
separate from adding the feature.
