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
Detailed reply/phase diagnostics are enabled with the listener. Add
`--no-diagnostics` to retain the basic cache/namespace/process series without
request traces, phase timers or the SDK observer. Use the **same executable** with
this switch for observer-cost controls. With no listener, detailed diagnostics
are disabled; existing non-diagnostic error messages are unchanged.

`--diagnostic-log-level warn` is the normal default. Levels include more severe
events: `error` reports unexpected failures; `warn` also reports retry/admission
warnings; `info` adds slow replies and scrape-driven unfinished work; `debug`
adds detailed request/phase traces. `off` disables these records while retaining
detailed metrics. This setting controls diagnostic JSON, not existing
startup/shutdown and other legacy messages. Detailed diagnostics still require
the listener and must not be disabled with `--no-diagnostics`.

The published v0.1.4 executable predates these levels: its default is `error`,
and its `warn` includes slow/inflight records. Use `info` for that detail in a
build containing the new level contract; `debug` is not a v0.1.4 option.

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
snapshot still returns 200 with independent observations. The affected component's
series are omitted and `geesefs_metrics_snapshot_available{component="namespace"}`
or `{component="read_cache"}` is zero. No stale snapshot or false zero occupancy
is substituted. Watch both Prometheus `up` and component availability.

## Metric contract

Counters reset on daemon restart. Gauges are observations, not a globally atomic
transaction. Labels use fixed application vocabularies: payload `lane` is
`data`, `ingress`, `flush`; diagnostics add the finite labels below. No pathname,
object key, bucket, errno number, inode, client or request ID becomes a label.
There are fewer than 1,000 numeric series per instance with the current vocabulary,
independent of namespace size. Event/HTTP-attempt series appear on first use.
`geesefs_diagnostics_enabled` distinguishes detailed and basic-only observations.

Scraping never walks inode contents, refreshes TTLs, changes LRU recency, admits
payload bytes or calls S3. Detailed scrapes visit live request traces, not the
retained namespace. Process status and allocator statistics are sampled locally.
Optional unavailable allocator/process values are omitted.

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
| `geesefs_process_cpu_seconds_total{mode="user"\|"system"}` | Daemon CPU seconds from Linux self stat; system includes this process's kernel execution, not all kernel/client/cgroup work |
| `geesefs_jemalloc_allocated_bytes` | Allocated size classes in the default allocator |
| `geesefs_jemalloc_active_bytes` / `geesefs_jemalloc_resident_bytes` | Active/allocator-resident page accounting |
| `geesefs_jemalloc_mapped_bytes` / `geesefs_jemalloc_retained_virtual_bytes` | Allocator virtual mappings; retained virtual bytes are not physical memory |
| `geesefs_jemalloc_metadata_bytes` | Allocator metadata pages |
| `geesefs_system_requested_bytes` | System-allocator build only: sampled net requested Rust bytes, excluding native allocations/retention |

Namespace cardinality is not metadata heap size. Negative-entry contents and
directory page contents are not scanned. Allocator observations reuse the
`--stats-file` JSON provider; neither allocator accounting nor the payload budget
is a process-memory ceiling. Collect cgroup memory/events/pressure independently.

### FUSE reply and live work

| Metric | Meaning |
| --- | --- |
| `geesefs_fuse_reply_duration_seconds{opcode}` | Histogram from instrumented callback entry to terminal reply invocation; not total job lifetime |
| `geesefs_fuse_replies_total{opcode,errno}` | Terminal reply invocations, including rejection, timeout and abandoned-reply EIO |
| `geesefs_executor_queue_seconds` | Histogram from an admitted spawn to its first poll; excludes kernel queueing |
| `geesefs_read_post_reply_seconds` | Histogram from successful/failed READ reply to destruction of the job and its owned child observations |
| `geesefs_fuse_inflight{opcode,state}` | Live trace owners split into `reply_pending` and `after_reply` |
| `geesefs_fuse_oldest_seconds{opcode,state}` | Oldest callback age in each group; zero when none remain |
| `geesefs_diagnostic_events_total{reason}` | Conditions at their source; not a count of terminal request failures |

Exact callback coverage is `lookup`, `getattr`, `open`, `read`, `write`, `fsync`
(including fsyncdir and sync-on-close), `readdir`, `readdirplus`. Other existing
asynchronous mutations share `mutation`; their current fallback trace starts at
spawn and has no request/inode identity. `other` is reserved. Uninstrumented
synchronous callbacks and kernel page-cache hits are not in these denominators.
A reply invocation does not confirm that the kernel/client received it.

Errno groups are `OK`, `ENOENT`, `EAGAIN`, `ETIMEDOUT`, `EINTR`, `EIO`, `EACCES`
(also EPERM), `ESTALE`, `ENOMEM`, `ENODATA`, `OTHER`. An early READ reply can succeed
while owned read-ahead continues. A later error/deadline neither changes that
reply nor adds a second failed reply. Completed histograms alone hide unfinished
work: inspect inflight/oldest and active phases as well.

Source reasons separate `admission_full`/`closing`, `deadline_before_reply`/
`deadline_after_reply`, `s3_get_timeout`, and `read_ahead_timeout`/
`read_ahead_cancelled`/`read_ahead_error`. Namespace EAGAIN sources identify the
operation and condition, such as `lookup_moving`, `lookup_binding_busy`,
`lookup_generation_changed`, `missing_epoch_changed`, `refresh_binding_changed`,
`directory_generation_changed`, and `rename_listing_repeated`. They retain the
existing EAGAIN contract. Source events can precede a caller retry or a background
outcome; do not add them to terminal failures.

### Phase and remote-work observations

Each stage has `geesefs_stage_duration_seconds{stage}` (histogram),
`geesefs_stage_inflight{stage}` and
`geesefs_stage_completions_total{stage,outcome}`. Outcomes are `ok`, `error`, or
`cancelled` for a dropped future/guard. A returned error, including an explicit
cancellation error, is `error`; dropping its future is `cancelled`.

| Stage | Boundary and limitations |
| --- | --- |
| `cache_read` | Initial payload-cache lookup and reply assembly/copy |
| `fill_wait` | Waiting for an overlapping fill to publish/change; not an additional GET |
| `payload_admission` | Only the pressured required-admission branch, including reclamation/flush/wait; fast reservations are excluded |
| `metadata_slots` | Directory metadata concurrency admission, including select-cancelled scopes |
| `namespace_wait_sampled` / `namespace_hold_sampled` | One in 1,024 acquisitions through the filesystem helper; not all namespace transactions or a request-level distribution |
| `s3_head` / `s3_list` | Logical backend method, including SDK work/retries and LIST fallback/parsing |
| `s3_get_headers` | Conditional GET send plus response-range validation; includes SDK setup/retries, not network RTT |
| `s3_get_stream` | Body consumption, admission and delivery backpressure; not network-only time |
| `s3_first_chunk` | First publishable fragment; whole-body/checksum paths wait for complete validation, not the first TCP byte |
| `chunk_delivery` | Sending an already-admitted chunk to the bounded consumer channel |

Histogram seconds buckets are `0.00001, 0.00005, 0.0001, 0.0005, 0.001, 0.005,
0.01, 0.05, 0.1, 0.5, 1, 2.5, 5, 10, 30, 60, 120, 300, 600, +Inf`. Phase
scopes include failure/cancellation and may overlap across parallel jobs. Their
counts are not FUSE request counts, their sums need not equal reply time, and
their p99 values must not be added. Namespace sampling also has observer cost;
the histogram library uses an internal mutex, not lock-free recording.

| Metric | Meaning |
| --- | --- |
| `geesefs_s3_attempts_total{operation,transport}` | SDK before-transmit or extension HTTP send attempts, including retries; excludes presigning |
| `geesefs_s3_retries_total{operation,transport}` | Attempts after the first in an SDK execution or extension retry loop |
| `geesefs_s3_body_bytes_total` | GET bytes yielded by the SDK, including later rejected/unused bytes; not wire bytes |

Operations are `get`, `head`, `list`, `write`, `other`; transports are `sdk` and
`extension` (currently extension LIST/PATCH). A transport attempt may fail before
sending a TCP byte and is not a confirmed S3 receipt. SDK metadata and body
retries differ; the stream itself is not transparently replayed by these counters.
Body bytes and validated `geesefs_read_remote_bytes_total` have different
boundaries. Neither counts TLS framing, retransmissions or direct nginx S3 work.

### Bounded diagnostic records

At `error`, detailed diagnostics write JSON to stderr for unexpected errors,
excluding ENOENT/ENODATA and transient EAGAIN/EINTR/ESTALE. The latter are `warn`;
namespace/admission source reasons remain attached. `info` adds replies taking
at least one second and the oldest unfinished trace aged at least 30 seconds.
Background errors and READ deadlines after reply retain separate error events.
Filtering precedes formatting. Error, warning and info records each have their
own one-per-second limiter, so an info event cannot consume an error's rate slot.
`geesefs_diagnostic_logs_suppressed_total` counts rate-suppressed candidates.
Counters/histograms record all observations independently of these filters.

Normal levels use one library worker with a bounded lossy queue of 16 whole
records. Debug uses a 256-record ordinary lane plus a separate 16-record worker
reserved for error/warning records, so debug saturation cannot consume severe
queue slots. A severe burst can still exhaust its own reservation.
Slow stderr/journald does not wait on a FUSE worker or retain its request admission;
`geesefs_diagnostic_logs_dropped_total` counts full/disconnected-queue drops. It does
not count stderr I/O failures or records lost on abrupt exit. The retained worker
guards attempt a shutdown flush; delivery is not guaranteed. The total drop counter
aggregates both queues; `geesefs_diagnostic_severe_queue_dropped_total` separately
counts losses from the reserved debug-mode queue (zero when that queue is disabled).
At `off` no diagnostic
writer thread is created. This avoids output backpressure, not all observation cost.

When stderr's device/inode matches the documented `JOURNAL_STREAM` identity at
initialization, each whole record carries a systemd priority prefix: error 3,
warning 4, info 6 and debug 7. With the receiver's `SyslogLevelPrefix=yes` (the
systemd default), journald strips that prefix, preserves the JSON `MESSAGE` and
uses the corresponding severity/rate-limit pool. Absent, malformed or redirected
stream identities retain ordinary JSON stderr. Receiver filtering, rate suppression
and retention remain independent loss sources and are not included in the
application's dropped-record counter. Priority framing does not make either queue
durable or increase the configured receiver limits.

Records contain level, event, opcode, FUSE request/inode IDs, range, errno, bounded reason,
callback age, reply state and active/completed phase scopes. A wall-clock
`timestamp_unix_seconds`, `process_id` and daemon-local `trace_id` support ordering
and correlation; use monotonic elapsed/duration fields for timing. Paths, credentials,
URLs and arbitrary backend error text are excluded. Namespace EAGAIN carries its
source reason into the terminal record. A `fuse_background_deadline` record with
`reply_sent=true` reports a tail timeout, not a failed foreground reply.

Instrumented filesystem failures retain their class before conversion to errno:
`filesystem`, `configuration`, `unsupported`, `payload_budget`, `cancelled`,
`timeout`, `invalid_response`, `backend_transport_or_protocol`, or
`backend_response`. The last two distinguish the backend's unmodeled transport/
protocol error from a service response; they do not expose arbitrary service
codes or messages. A generic EIO alone cannot identify which dependency failed.
Read-ahead records use the same classes; their source counters retain the
`read_ahead_timeout`/`read_ahead_cancelled`/`read_ahead_error` grouping.

Error/warning records additionally carry `backend_failures`: at most five slots
for `list`, `head`, `get`, `write` and `other`. Each retains the latest failed
execution's `transport`, numeric `http_status` (or null), closed `kind`, HTTP
`attempts`, accumulated failed `observations` and monotonic `observed_seconds`
since trace creation. Successful retries do not create a failed execution.
SDK kinds distinguish `timeout`, `connector`, `response`, `service` and `other`;
extension kinds distinguish `timeout`, `connect`, `decode`, `body`, `request`,
`response_limit`, `invalid_response` and `service`. GET-body failures use
`transport=sdk_body`, `body`/`invalid_response`, with null status and attempts:
the SDK has already finalized, and its output does not retain those observations.
No arbitrary service codes, request IDs, headers or body text are added.

These are observations, not a claim that the latest backend failure caused the
terminal errno: speculative metadata work can fail and be ignored, and parallel
executions can overwrite a slot. Counts are failures, not retries; the summary is
neither an HTTP history nor complete coverage of cancellation/timeouts outside
the observed boundaries. It shares the severe record's existing sampling and
lossy queue, without depending on debug history. Other levels carry null.

Each scrape can also emit one candidate for the oldest trace aged at least 30s
(`fuse_inflight`, level `info`); the info limiter applies. This is scrape-driven unfinished-work
visibility, not a watchdog independent of the runtime. Phases show active scope
counts, accumulated completed seconds and whether any scope of that stage failed
or was dropped (`failed`, `cancelled`). These flags identify outcomes, not counts
or chronology; one stage can include both across parallel jobs. Concurrent scopes
can overlap. Normal logs are sampled evidence, not an exhaustive request journal
or HTTP-to-FUSE correlation.

#### Debug request traces

Select `--diagnostic-log-level debug` for a bounded diagnostic window. Debug
mode bypasses the one-per-second limiters for every record, including error/warn/info
replies; it retains the bounded lossy output queue. This is deliberately more
expensive: formatting and enqueue attempts occur
on instrumented request threads. It is not the normal operating mode or a
performance-acceptance configuration. Watch the dropped-record counter; full
queues, output failures and abrupt exit can leave incomplete traces, including
missing severe records. This is not a durable audit log.

- `fuse_request`, `fuse_polled`, `fuse_reply`, `fuse_complete` distinguish callback
  entry, executor queue delay, terminal reply and the end of owned background work.
  Fast successful replies and ordinary ENOENT/ENODATA are debug-only.
- `phase_start` / `phase_end` have `details.stage` and a unique daemon-local
  `details.scope_id`, so parallel scopes of the same stage can be paired.
  Completion includes duration, `ok`/`error`/`cancelled`, and a bounded error class.
  Existing namespace-lock sampling remains one in 1,024 acquisitions.
- `read_cache` reports the initial cache hit/miss and EOF-clipped range;
  `read_fill` reports leader/follower ownership and the read-ahead window.
- `s3_attempt` records operation, transport and retry status; `s3_body_chunk`
  records yielded byte counts. `diagnostic_event` retains bounded source reasons.
  These events inherit an instrumented FUSE context; standalone/background backend
  calls without that context still contribute metrics, not fabricated request IDs.

No raw paths, object names, payloads, URLs, authorization headers, arbitrary error
messages or stack-memory dumps are added at debug. Coverage is the instrumented
callbacks/stages above, not every syscall, SDK-internal span or kernel/page-cache
read. Events within one trace can be concurrent; output order is FIFO within each
lane, not across the ordinary/severe lanes or a total order across threads. Use
timestamps and IDs for correlation. No OpenTelemetry exporter or new global tracing
subscriber is installed.

## Attributing Go-relative regressions

Use Go 0.43.9 at the [declared reference revision](project-brief.md#target-and-reference).
Pin source/binary hashes, object generations, machine/kernel, CPU/cgroup limits,
concurrency, payload/entry budgets, metadata TTL/negative-cache policy and cold/warm
state. Collect instrumented diagnostics separately from uninstrumented acceptance.
Use at least eight independent mount pairs and same-executable A/A controls for
[comparative acceptance](performance-methodology.md); local fixture processes do
not substitute for independent mounts.

Collect per-instance series at 30s, externally measured client throughput/latency,
nginx request/upstream/cache status and rsync errors over the same sustained window.
Include p50/p95/p99, observations, failures and >30/>60/>120s counts. Preserve raw
private records outside source; completion timestamps and client backpressure
matter. Ordinary file reads do not propagate an HTTP request ID into FUSE.
Direct nginx-to-S3 and golist paths need their own logs; FUSE timings cannot explain
them or client delivery/kernel queueing by themselves.

Start with the following hypotheses:

- High executor delay or `admission_full`: dispatch/admission pressure, rather
  than assuming every EAGAIN came from S3. Namespace source events identify a
  different retry mechanism.
- High sampled namespace waits/holds, `cache_read`, `fill_wait`, or pressured
  admission with ordinary S3 phases: investigate local locks, copies, pinned
  owners and payload pressure using matching CPU/off-CPU profiles.
- High GET headers or HEAD/LIST duration with retry growth: inspect SDK/transport
  setup, dependency errors and upstream telemetry. Header time is not RTT.
- Long streams/first-fragment delay versus delivery/admission pressure: distinguish
  backend consumption from local backpressure; inspect unfinished traces as well
  as completed samples.
- More attempts or SDK/validated bytes per equivalent client work: inspect TTL
  refreshes, cache reuse, read-ahead and retries. Requested-byte counters include
  overlapping callers; amplification is not an exact measurement of wasted bytes.
- Higher CPU/RSS at unchanged work: normalize daemon/client/cgroup CPU by completed
  operations and delivered bytes, inspect profiles, allocator/RSS, payload permits,
  metadata cardinality, cgroup file/kernel memory and throttling independently.
  Do not force Go GC, equate wall time with CPU, or treat payload capacity as RSS.

Example PromQL, keeping instance and opcode separate:

```promql
histogram_quantile(0.99,
  sum by (instance, opcode, le) (rate(geesefs_fuse_reply_duration_seconds_bucket[15m])))
sum by (instance, opcode) (increase(geesefs_fuse_reply_duration_seconds_count[15m]))
sum by (instance, opcode, errno) (rate(geesefs_fuse_replies_total{errno!~"OK|ENOENT|ENODATA"}[15m]))
histogram_quantile(0.99,
  sum by (instance, stage, le) (rate(geesefs_stage_duration_seconds_bucket[15m])))
geesefs_fuse_oldest_seconds{state="reply_pending"}
sum by (instance, mode) (rate(geesefs_process_cpu_seconds_total[15m]))
```

Always retain the histogram count alongside quantiles; low counts, missing
component snapshots and buckets wider than the measured effect are inconclusive.
A histogram quantile is interpolated, not an exact client percentile. Equivalent
cache budgets/configuration and repeated paired controls are required before
attributing a Go-relative gap or claiming that it is fixed.

### Observer-cost fixtures

The ignored diagnostics reuse real fuser parsing/replies over a UnixDatagram and
the SDK over localhost HTTP, with byte checks and unchanged budgets:

```bash
timeout 2400s cargo +1.96.0 test --locked --release --no-run --test fuse_deadline_review --test backend_contract
GEESEFS_OBSERVER_MODE=cached GEESEFS_OBSERVER_ENABLED=1 GEESEFS_OBSERVER_CALLS=100000 \
  timeout 120s cargo +1.96.0 test --locked --release --test fuse_deadline_review observer_cost_diagnostic -- --ignored --exact --nocapture
GEESEFS_OBSERVER_ENABLED=1 GEESEFS_OBSERVER_CALLS=5000 \
  timeout 120s cargo +1.96.0 test --locked --release --test backend_contract sdk_observer_cost_diagnostic -- --ignored --exact --nocapture
```

Repeat with `GEESEFS_OBSERVER_ENABLED=0`, retaining the same executable hash, and
use interleaved off/on/off repetitions. Fuser modes `cold` and `metadata` exercise
disjoint uncached virtual ranges and expired metadata through the memory store;
the SDK fixture separately covers real HTTP/signing/body observation. Output
includes raw latencies and whole-fixture CPU ticks (including client/server), not
daemon-only CPU. Fuser also reports RSS/high-water mark, allocator state and
100-scrape mean/series/bytes. The SDK fixture uses the test harness's system
allocator; fuser uses the default production allocator unless built otherwise.
These fixtures omit kernel FUSE, production RTT/TLS and sustained multi-client
traffic. Their results diagnose observer cost, not Go superiority or acceptance.

Eight release-fixture process blocks for the 0.1.3 diagnostic/logger source on
Linux 6.17/x86-64, with randomized
off-a/on/off-b arms, give the following geometric on/off effects. Brackets are
95% block-bootstrap intervals; controls use the same executable within each
fixture. Calls per process are 100,000 cached/metadata, 10,000 cold and 5,000 SDK,
after separate warmup. Raw samples, source and executable manifests are retained
outside source.

| Fixture | Wall/call change | Whole-process CPU/call change | p99 change |
| --- | --- | --- | --- |
| Cached read | +4.5% [-1.3, 12.6] | +5.8% [-0.01, 14.2] | +1.9% [-1.4, 5.1] |
| Uncached virtual read | +7.9% [4.6, 11.6] | +7.7% [4.3, 11.4] | +8.0% [2.1, 14.7] |
| Expired metadata | +3.4% [-0.2, 8.8] | +4.2% [0.8, 9.2] | +2.6% [0.1, 5.7] |
| SDK/localhost HTTP | -7.4% [-23.9, 12.2] | -5.7% [-17.0, 6.3] | -19.8% [-73.4, 94.4] |

Detailed observation is not free. Median wall/call changes from 70.7 to 72.9 us
cached, 94.4 to 102.7 us uncached and 67.8 to 68.6 us for metadata. These absolute
medians and paired geometric effects summarize different statistics. A/A wall
intervals are broad: cached [-12.3, 1.5]%, cold [-2.8, 8.4]%, metadata [-5.0, 2.1]%,
and SDK [-25.1, 0.8]%. SDK tails are especially unstable; negative point estimates
are not evidence of a speedup. The uncached/metadata overhead remains visible,
not a zero-latency guarantee. Cached RSS median grows from 18,892 to 19,246 KiB,
effect +1.83% [1.37, 2.24], not a large-mount bound. The cached direct-render median
grows from 0.255 to 0.448 ms, 40 to 769 numeric series and about 6.5 to 58.8 kB.
This excludes HTTP scrape costs and does not measure sustained error-record load.
Keep `--no-diagnostics` available for basic-only controls and collect production
profiles separately rather than treating these fixture results as a release
performance claim.

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
