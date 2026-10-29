# Confirmed cause: read-ahead does not advance on cache hits

## Finding

The frozen Rust implementation prefetches chunks within its current read window,
but starts the next window only after a demand read misses the cache. A cached
read returns before the future-range planner. The reader therefore encounters
the next GET's response latency at the window boundary.

Go checks the forward-looking range on every read, including cache hits, and
starts missing future ranges before their first bytes are consumed. This is a
confirmed avoidable source of Rust refill latency. It does not explain every
first-object delay or establish that every optimization opportunity is exhausted.

The [numeric evidence](read-ahead-refill-causality-20261029.json) contains the
direct socket/FUSE joins, selective-delay experiment, backend-worker control,
metric scopes, all phase results and limitations. Overall acceptance remains
**UNACCEPTED**; identifying the mechanism is not an implemented runtime fix.

## Exact source path

The runtime remains source `3016433370e9d2ea7daee495106572012a70228f`, System-feature
binary SHA256 `46350f9c4c4f6db26a1fdbdea35a5ebb532e7fe7667380f88d77fe1e6a1d2cfd`.
All 25 compiled inputs still match that build.

- [`Filesystem::read_inner`](https://github.com/borislitv/geesefs-rs/blob/3016433370e9d2ea7daee495106572012a70228f/src/filesystem.rs#L866) returns `Ok(Some(block))`
  immediately on the first cache hit. The second cache check around line 915 also
  returns. Window admission and GET task creation are below those returns,
  around lines 931–1008. Updating `Pattern` on every read does not itself launch
  another window.
- In the experimental Go reference,
  [`FileHandle.ReadFile`](https://github.com/yandex-cloud/geesefs/blob/5c0884c9e8799bbab93e2d4102fdc15e9c079e0d/core/file.go#L590)
  calls `getReadAhead`, `trackRead` and `CheckLoadRange` before returning buffers.
  [`LoadRange`](https://github.com/yandex-cloud/geesefs/blob/5c0884c9e8799bbab93e2d4102fdc15e9c079e0d/core/file.go#L294)
  examines `GetHoles(offset, raSize)`;
  [`loadFromServer`](https://github.com/yandex-cloud/geesefs/blob/5c0884c9e8799bbab93e2d4102fdc15e9c079e0d/core/file.go#L243)
  merges/splits missing ranges and starts their reads. Required cached bytes do
  not have to wait for the future range to finish. The inspected public file's
  Git blob SHA1 is `65e781349cc0f51b01373d1d2dc78d1262ecaf42`.
- The two compiled Go references independently exhibit early refill requests in
  the trace; the conclusion is not based only on reading one reference source.

The timing sequence at a missed Rust window boundary is:

```text
consumer read -> cache miss -> GET send -> response headers/body -> FUSE reply
```

The Go refill is already requested, and its required socket bytes received,
before the consumer enters that boundary read. Both implementations still do
real S3 I/O; the difference is where that I/O lies relative to demand.

## Direct trace without a proxy

Use the original fixed executables, immutable 3,000-file / 8,298,727,528-byte
fixture, four readers and 128 KiB reads. Filesystem/client resources are
8 CPU / 16 GiB; backend resources are 16 CPU / 12 GiB. Filesystem runtimes have
eight workers, and this capture explicitly sets backend GOMAXPROCS to 16.

Scoped BPF events record outgoing GET headers, completed socket reads, HTTP
response headers, FUSE queue/delivery/completion and client syscall spans.
HTTP request/response ranges, successful sends, header lengths and cumulative
body bytes are checked per connection. All captured GET bodies are complete.
The FUSE superblock device identifies the owned mount. Every implementation has
68,438 matched client reads with zero unmatched syscalls and 65,438 FUSE reads.
No CFS throttle interval is observed.

For a noninitial GET, identify the client read consuming its first byte and when
the daemon's socket reader has received the bytes needed by that read. Exclude
requests with ambiguous overlapping covering GETs; retain their counts.

| Implementation | Unambiguous noninitial GETs | Sent after consuming read began | Required socket bytes ready before read | Median request lead ms | Median consuming-read ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| Rust | 659 | **484** | 162 | **-0.184** | **1.734** |
| Go experimental | 884 | **0** | 884 | **15.576** | **0.239** |
| Go packaged | 884 | **0** | 884 | **15.441** | **0.238** |

Negative lead means the consuming read has already begun. Rust has two ambiguous
GETs excluded from this table; both Go references have none. The median required
socket-byte lead is -1.497 ms for Rust versus +14.026/+13.998 ms for Go. These are
conditional refill measurements, not whole-workload latency percentiles.

The Rust tail join identifies a same-range pending FUSE reply and covering GET
for 684/685 tail reads. Of their summed userspace-service time, 11.940% precedes
the observed GET, 75.939% lies between GET and response headers, 6.298% waits for
the remaining required socket bytes and 5.823% follows socket receipt. These
are stage boundaries, not a claim that refill scheduling causes 75.939% of all
latency. Initial-object GETs are included in that tail population. The Go stage
joins cover 577/685 and 583/685 tail reads; unmatched dependencies remain explicit.

## Controlled intervention: delay only refill requests

An aiohttp 3.12.15 adapter forwards the same read-only fixture on loopback,
preserving request signing headers, Range/If-Match, response headers and payload.
Both control and treatment use this adapter. It adds a configured 5 ms delay
before forwarding GETs whose Range starts above zero; initial GETs are unchanged.
Actual delay intervals are measured because event-loop scheduling can extend
the configured sleep. Repeated order is control, treatment, treatment, control,
with rotated implementation order and fresh/warm phases.

All 24 phases pass 72,000 full-file checks. Native after-run verification checks
all 3,000 files. Proxy logs contain no request exceptions. The consuming-read
join excludes overlapping GETs and uses per-read monotonic timestamps and byte
offsets, not aggregate percentile subtraction.

For the two **warm treatment** phases per implementation:

| Implementation | Noninitial GETs | Requested after consuming read began | Actual inserted interval sum ms | Inserted interval overlapping the consuming read ms |
| --- | ---: | ---: | ---: | ---: |
| Rust | 1272 | **932** | 8237.901 | **5828.685** |
| Go experimental | 1768 | **0** | 13981.770 | **0** |
| Go packaged | 1768 | **0** | 13146.039 | **0** |

There are no ambiguous requests in these warm treatment groups. Every Go
request is made before consumption and the proxy has sent its first body chunk
before that read begins. Rust exposes the inserted delay at 932 demand boundaries;
the direct trace above independently confirms the late socket-byte availability
without the proxy. The sums are across requests, not elapsed experiment time.

This intervention demonstrates why delaying a refill becomes a reader stall in
Rust while Go can overlap it with already available data. It does **not** claim
that all Go reads or whole-workload timings are unaffected by the adapter.
Proxy copies/CPU and noticeable temporal drift change absolute timings, so this
matrix is not performance acceptance and is not used to estimate a universal
percentage of production p99. All adverse control/treatment values remain in JSON.

## Backend-worker hypothesis checked independently

The original fixture explicitly forced SeaweedFS `GOMAXPROCS=4` even after its
CPU quota was increased. The earlier headroom report removed CFS/memory ceilings
but did not remove that internal backend setting. A replacement fixture makes
the setting explicit and verifies it in the live process environment.

Run 4 -> 16 -> 16 -> 4 backend workers with identical quotas, filesystem workers,
budgets, binaries and sealed generations. Each step restarts the backend against
the same read-only data and verifies native hashes before becoming ready.
All 48 phases pass 144,000 full-file checks. Warm means (six phases per cell),
decimal MB; final-fifth RSS is not a memory plateau:

| Backend workers | Implementation | MB/s | p95 / p99 ms | FS/client CPU s | Backend read CPU s | Daemon RSS peak / final-fifth MB |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| 4 | Rust | 1516.099 | 1.067 / 3.141 | 23.860 | 15.122 | 845.971 / 838.008 |
| 4 | Go experimental | 1215.310 | 0.910 / 2.836 | 26.154 | 17.237 | 1319.264 / 1220.647 |
| 4 | Go packaged | 1172.936 | 0.947 / 3.060 | 27.109 | 17.844 | 1348.418 / 1223.916 |
| 16 | Rust | 1449.484 | 1.215 / 3.096 | 24.503 | 19.034 | 824.480 / 817.389 |
| 16 | Go experimental | 1123.449 | 1.005 / 2.918 | 27.460 | 21.998 | 1310.250 / 1216.199 |
| 16 | Go packaged | 1149.558 | 1.016 / 2.946 | 26.441 | 21.231 | 1343.624 / 1227.853 |

More backend workers do not remove the Rust p95/p99 gap and consume more backend
CPU. The hypothesis that the hidden worker setting explains the remaining gap
is not supported by this control. Both the subsequent direct trace and the
selective-delay experiment explicitly use sixteen backend workers.

## Other observations and data retention

Rust stores `Pattern` on the inode; Go's `NewFileHandle` starts fresh history per
handle. This can change the first read-ahead window on reopening. That source
difference is recorded, but is not independently established as the cause here.
An earlier window-size A/B was interrupted by the original fixture's configured
PID1 alarm; its partial console results are not a complete accepted experiment.
The original fixture's later raw network data could not be recovered through
its default kubelet emptyDir paths. The authoritative network experiment above
is a new, complete run with collected raw evidence on the replacement fixture.

The replacement's 961 immutable artifacts are collected and hash-verified.
Backend-worker and selective-delay matrices cover 216,000 complete file checks;
the six network-profile phases add 18,000, separately labeled. All controlled
phase CPU throttling is zero, and filesystem/client and backend memory-limit
events remain zero. Process RSS, whole cgroup, file/kernel memory and separate
backend CPU scopes are retained in the numeric receipt.

Independent export validation checks 40,137 numeric cells and interval-derived
values, 2,427 direct refill dependencies, all table rows, relative links and
public-source boundaries. After that verification the backend stops normally
through its controller's completion marker. Both exact owned diagnostic Pods
are deleted with normal grace periods and verified absent. The numeric resource
proof is the earlier collection-time snapshot, not a claim that a fixture is
still running. Private lifecycle receipts retain the identity guards and readback.

## Repair target

Advance bounded read-ahead on cached sequential reads, planning only missing
future ranges before the current window is exhausted. Preserve immediate
publication of required cached bytes, full backing charges, FIFO admission,
generation/If-Match/EOF/checksum barriers, cancellation and request ownership.
Optional prefetch must not starve a required read or silently relax byte limits.

The public regression should observe a next-range GET starting while a cached
read can still complete, before consumption reaches the window boundary, and
verify deduplication, pressure, cancellation and concurrent-reader behavior.
That runtime change and its matched performance verification are subsequent
work. This report establishes a concrete causal mechanism; the broader
representative reader/writer matrix and production acceptance remain open.
