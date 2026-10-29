# Attribute warm-read tails before changing the runtime

## Acceptance scope

The operator clarified the target on 2026-10-29: lower memory and CPU cost,
higher throughput and better p50/p95/p99 during normal operation of a long-lived
mount. Cold-start/listing measurements remain visible, but winning cold start
is not required. Reader, metadata, random/hot-set and writer/fsync compatibility
and representative performance remain required. This diagnostic is not that
complete acceptance matrix.

Higher CPU allocations are allowed, provided each implementation receives the
same allocation in its comparison. Report the resource change as such; it is not
a runtime optimization. The current implementation remains **unaccepted**.

The [numeric receipt](steady-state-headroom-results-20261029.json) retains all
216 unprofiled phases and 648,000 full-file checks, metric scopes, per-phase
samples and variance. The [eighteen flamegraphs](flamegraphs-20261029/README.md)
cover both runtime-worker configurations. All 3,040 indexed private evidence
artifacts were collected and checked against their recorded SHA256 values.

## Fixed inputs and measurement scopes

Use the same public benchmark client, four readers, 128 KiB read calls, 512 MiB
configured payload/ingress/flush budget and 100,000-entry limit. The immutable
fixture contains 3,000 files, 8,298,727,528 payload bytes and 97 directory listings.
The manifest SHA256 is
`7bd537c12c702a614b51383a28ed167c86dbf2d964696525cdf8b4c2871555a1`.
Full payload hashes and required metadata are checked, not just transferred bytes.

| Implementation | Source revision | Binary SHA256 |
| --- | --- | --- |
| Rust, System allocator | `3016433370e9d2ea7daee495106572012a70228f` | `46350f9c4c4f6db26a1fdbdea35a5ebb532e7fe7667380f88d77fe1e6a1d2cfd` |
| Go experimental | `5c0884c9e8799bbab93e2d4102fdc15e9c079e0d` | `7ad836e0bdd23ea279bb236627e9b130d11c174ae649c7582213a83e5d11a28b` |
| Go packaged | `80c214ef14f620d93407454fc76ca01dd71fa25f` | `e04492363d29985ab1f74521482a01a3c6f7fdeeed35ad1ba0faa991370f4683` |

The Rust executable was built with
`cargo build --release --locked --offline --features system-allocator`.
The Cargo allocator default remains unchanged. Tokio workers and GOMAXPROCS
remain two during CPU-capacity experiments; increasing the quota is independent
of increasing runtime worker counts. The filesystem daemon and benchmark client
share one cgroup; the synthetic S3 backend has a separate cgroup in the same
network namespace. Consequently, filesystem/client CPU capacity is not a
daemon-only allocation. Kernel page cache is not globally dropped or controlled.

## What the instrumented traces establish

CPU-clock samples at 99 Hz with DWARF callchains cover the owned daemon only.
Scheduler events, client read timestamps, thread identities and kernel syscall
boundaries establish where a client read waited. Separate FUSE request events
split queue-to-delivery from delivery-to-reply; CFS throttle/unthrottle events
cover filesystem/client, backend and parent groups.

For the Rust scheduler run, all 68,438 client read spans match kernel reads, with
zero unmatched reads. The slowest one percent comprises 685 reads and
23,780.836 ms of summed read wall time, not elapsed experiment time:

- 97.540% of that sum is inside the kernel read syscall.
- 99.365% overlaps the observed reader's off-CPU intervals.
- 73.566% is blocked before wakeup; 25.799% is ready but not scheduled.
- Post-syscall time accounts for 2.456%. Intervals shorter than 250 microseconds
  are omitted from per-read scheduler attribution.

The dominant blocked-reader stack includes `fuse_file_read_iter`,
`generic_file_read_iter`, `filemap_read`, `folio_wait_bit_common` and `io_schedule`.
It identifies waiting for FUSE-backed pages, not the specific async operation
inside the daemon. Aggregate daemon epoll/futex time includes idle workers and
cannot be assigned to the critical path of a particular request.

The separate Rust FUSE/CFS run contains 65,449 read requests. Queue latency
p50/p95/p99 is 0.029610/0.342662/1.192247 ms; userspace-service latency is
0.129931/2.555315/15.311072 ms. A pending reply for the same file/range is
identified for 623 of the 685 tail reads. One 141.601 ms read overlaps backend
and parent throttling but no filesystem-group throttling; another request waits
43.188 ms before delivery to the daemon. There is more than one waiting stage.

The CPU-union filesystem throttle intervals overlap 84.968% of summed tail-read
wall time. This is **correlation, not an 84.968% causal contribution**: an interval
on another CPU need not prevent this request's required thread from running.
File inode/range and delivery to the owned daemon's FUSE descriptor scope the
join; the capture does not expose a separate connection identifier. Preserve
that limitation rather than treating every cgroup FUSE event as the target mount.

## CPU attribution and debug symbols

The Rust scheduler profile contains 11.868687 sampled CPU seconds and zero lost
samples. Inclusive callchain percentages and leaf costs are different scopes.
The largest leaves include kernel `memcpy_orig` (1.949495 s), kernel
`rep_movs_alternative` (1.313131 s), libc `__memmove_avx_unaligned_erms`
(0.979798 s), `lockless_pages_from_mm` (0.525253 s), and allocator leaves such as
`_int_malloc` (0.252525 s). These sampled costs do not by themselves explain a
millisecond p99 wait.

Resolve libc with the exact recorded runtime ELF/build ID, not symbols from the
analysis host. This run uses libc build ID
`066527e430a32768d82741e00b81eebb1a872294`; debug symbols are installed only in
the disposable diagnostic environment, without upgrading its runtime.
Render CPU and daemon/client off-CPU flamegraphs separately. CPU graphs count
samples; off-CPU graphs count aggregate thread nanoseconds, including idle and
background time. Neither is a request-latency flamegraph.

## Resource-capacity experiments

The initial 48-phase screen uses four configurations, with two opposite-order
mount repetitions per implementation: filesystem/client + backend CPU capacities
of 2+4, 4+4, 2+8 and 4+8. The 100 ms quota period, runtime workers and memory
limits remain unchanged. The parent CPU budget equals the sum of child budgets.
Temporary overrides affect only the owned fixture and are observed throughout;
a separate watchdog and the controller both restore the original limits.

Warm-phase means from the screen:

| CPU allocation | Implementation | Read MB/s | p99 ms | Whole FS/client CPU s | Daemon RSS peak / final-fifth MB |
| --- | --- | ---: | ---: | ---: | ---: |
| 2+4 | Rust | 623.696 | 10.856 | 27.775 | 643.912 / 643.912 |
| 2+4 | Go experimental | 586.361 | 11.284 | 29.961 | 1243.671 / 1201.284 |
| 2+4 | Go packaged | 581.042 | 10.902 | 30.012 | 1284.846 / 1212.910 |
| 4+4 | Rust | 809.749 | 7.771 | 29.348 | 642.726 / 622.663 |
| 4+4 | Go experimental | 733.226 | 6.369 | 29.746 | 1290.533 / 1239.977 |
| 4+4 | Go packaged | 705.239 | 6.114 | 30.425 | 1314.333 / 1226.888 |
| 2+8 | Rust | 597.095 | 11.787 | 29.017 | 624.912 / 624.911 |
| 2+8 | Go experimental | 568.988 | 11.442 | 30.620 | 1335.685 / 1228.729 |
| 2+8 | Go packaged | 576.090 | 11.630 | 30.448 | 1312.993 / 1214.606 |
| 4+8 | Rust | 884.386 | 7.127 | 28.440 | 625.299 / 625.299 |
| 4+8 | Go experimental | 728.441 | 6.040 | 30.377 | 1343.107 / 1187.139 |
| 4+8 | Go packaged | 730.477 | 6.092 | 29.950 | 1393.471 / 1236.243 |

Adding only backend CPU does not improve the screen. Increasing the shared
filesystem/client allocation materially reduces throttling and read tails, but
the Rust p99 still exceeds both Go references at 4+8. A second 36-phase run
reuses each mount for five warm revisits: Rust averages 843.748 MB/s and
7.168 ms p99 at 4+8, versus Go experimental 736.639/6.120 and Go packaged
696.892/6.468. These five revisits share a mount; they are not five independent
mount replications or proof of a long-term plateau.

A separate 24-phase same-capacity period ablation retains 2+4 CPU while reducing
only the filesystem period from 100 to 10 ms. Rust warm p99 improves from
11.277 to 8.924 ms, but p95 worsens from 1.908 to 4.595 ms and throughput drops
from 600.850 to 550.299 MB/s. Both Go references also trade a better p99 for a
worse p95 and throughput. This is not adopted as a runtime improvement; the
expanded fixture retains the original 100 ms period.

The operator then requested another CPU/memory increase to remove test-resource
constraints from code diagnosis. Kubernetes `pods/resize` applies 8+16 CPU and
8+12 GiB to the same containers without restarting them. The corresponding
36-phase matrix has **zero CPU throttling in every group**, but the filesystem
group reaches its memory ceiling: 448,710 `memory.events:max` increments,
63,620,992 scanned pages and 8.531322 s of memory full-pressure time across the
phase windows. No OOM occurs. A low daemon RSS does not mean the full cgroup has
spare memory: its peak reaches approximately 8,590 MB, predominantly file cache.
Therefore that run is not a resource-unconstrained code diagnostic.

The next configuration raises only filesystem/client memory to 16 GiB, leaving
CPU at 8+16 and backend memory at 12 GiB. The parent memory limit becomes 28 GiB.
Resource checks include actual kernel limits, memory events, direct reclaim,
major faults, file refaults, CPU/memory PSI and host pressure. Keep the 8 GiB
failure visible and require measured headroom before attributing remaining
delays to application code. The application payload budget remains 512 MiB.

### Verified expanded fixture

At 8+16 CPU and 16+12 GiB, all 36 phases complete with 108,000 full-file checks,
3,000 native after-run hashes and 97 native listings. Actual limits are checked
every 100 ms. The containers and fixture survive both Kubernetes resizes without
restart. Across these phase windows, CPU throttling and memory limit/OOM events
are zero in filesystem/client, backend and parent groups. Filesystem/client
`pgscan`, `pgsteal`, major faults and file refaults are also zero. Its memory
full-pressure total is 0.001587 s. Backend totals are 73 scanned pages, one major
fault and 0.000840 s memory full-pressure; do not round these observations to zero.
Ordinary CPU scheduling pressure remains measured and is not called throttling.

Each implementation has two independent mounts in opposite order and five warm
revisits per mount. Means of those ten phase measurements, decimal MB:

| Implementation | Read MB/s | p50 / p95 / p99 ms | Whole FS/client CPU s | Daemon CPU s | Daemon RSS peak / final-fifth MB |
| --- | ---: | ---: | ---: | ---: | ---: |
| Rust | 1223.916 | 0.087 / 1.320 / 4.075 | 25.196 | 10.882 | 637.297 / 636.471 |
| Go experimental | 867.072 | 0.270 / 1.216 / 4.093 | 27.996 | 12.660 | 1301.459 / 1212.483 |
| Go packaged | 844.619 | 0.277 / 1.237 / 4.281 | 28.744 | 13.158 | 1328.916 / 1222.168 |

Throughput sample standard deviations are 58.758/31.831/39.964 MB/s respectively;
p99 standard deviations are 0.299/0.267/0.358 ms. Rust's p99 difference from
experimental Go is smaller than this dispersion, not a demonstrated tail win.
Its p95 remains worse than both Go variants. Ten same-mount revisits do not
constitute ten independent mount experiments.

The whole cgroup is much larger than the daemon:

| Implementation | FS/client cgroup peak MB | File-cache peak MB | Kernel peak MB | Backend cgroup peak MB |
| --- | ---: | ---: | ---: | ---: |
| Rust | 10131.362 | 9341.542 | 58.595 | 9199.874 |
| Go experimental | 10736.205 | 9341.318 | 60.378 | 9240.982 |
| Go packaged | 10768.043 | 9341.308 | 60.552 | 9254.990 |

These are means of separate phase peaks; components need not peak simultaneously.
File cache includes the workload and diagnostic fixture files. RSS and final-fifth
values remain scoped process measurements, not total memory or a plateau proof.
The expanded run preserves all source hashes; its gains are not attributed to a
Rust code change. Remaining p95, representative workloads and compatibility gates
keep the project unaccepted.

### Runtime worker count is a separate intervention

The same expanded fixture also completes 36 phases with Tokio workers and
GOMAXPROCS both set to eight instead of two, retaining four client readers.
Each implementation again has two independent mounts and ten warm phases:

| Implementation, eight workers | Read MB/s | p50 / p95 / p99 ms | Whole FS/client CPU s | Daemon CPU s | Daemon RSS peak / final-fifth MB |
| --- | ---: | ---: | ---: | ---: | ---: |
| Rust | 1256.099 | 0.086 / 1.299 / 4.117 | 28.596 | 14.131 | 803.203 / 800.760 |
| Go experimental | 1070.054 | 0.207 / 1.050 / 3.500 | 29.079 | 14.370 | 1310.009 / 1221.100 |
| Go packaged | 1017.035 | 0.221 / 1.104 / 3.681 | 31.045 | 15.884 | 1334.537 / 1233.747 |

All three groups again have zero CPU throttling and memory limit events.
Filesystem/client scan, steal, major fault, file-refault and memory full-pressure
deltas are zero. Backend records one major fault, 26 file refaults and 0.000107 s
of memory full-pressure; scan and steal remain zero.

More workers do not explain away Rust's remaining tail: its p99 does not improve,
while both Go references improve. Relative to the two-worker matrix, Rust's
daemon CPU and retained RSS increase materially for a small throughput change.
These matrices run consecutively, not as a randomized interleaved worker sweep;
retain that temporal limitation along with all samples. Rust still uses less
process memory and has higher throughput, but p95/p99 do not win at eight
workers. Do not choose only the two-worker Go result when presenting Rust at
eight workers, or silently label final-fifth RSS a stable heap plateau.

The eight-worker trace reaches the same stage-level conclusion: 97.415% of
summed Rust tail-read wall time is blocked before wakeup; only 0.409% is ready
but not scheduled. Of 685 tail reads, 683 have an identified pending FUSE reply.
Queue p99 is 0.415203 ms and userspace-service p99 4.596528 ms, with no observed
CFS throttle interval. Source-level attribution still needs a join from that
service interval to byte admission, fill publication and S3 header/body waits.

### Profiles after removing quota and memory ceilings

The separate instrumented run uses the verified 16 GiB filesystem configuration.
It joins all 68,438 client reads per implementation with zero unmatched reads,
and 65,438 FUSE reads per implementation. This capture additionally filters the
FUSE request's superblock device against the owned mount, rather than relying
only on the cgroup and matching inode. No CFS throttle interval is observed.

For Rust's 685 tail reads, 96.434% of summed read wall time is blocked before
wakeup and 0.499% is ready but not scheduled. A same-file/range pending FUSE reply
is identified for 684 of those reads. Queue p99 is 0.599924 ms; userspace-service
p99 is 4.239506 ms. The longest observed read takes 39.850 ms, including 39.791 ms
between delivery and reply, with no quota overlap. This localizes the next
investigation to request service, including backend/admission/fill waits; it does
not identify a particular async wait as the cause.

The resolved Rust CPU profile has 12.383838 sampled CPU seconds, zero lost samples
and these exclusive leaf categories with caller evidence:

| Category | Sampled CPU s |
| --- | ---: |
| Kernel FUSE reply copy (`fuse_copy_page` / `fuse_dev_do_write`) | 2.393939 |
| Kernel TCP receive copy (`tcp_recvmsg` / `skb_copy_datagram_iter`) | 1.565657 |
| Admitted SDK receive-fragment copy (`CopyBuf` / `S3::read_range_chunks_inner`) | 1.101010 |
| Other kernel leaves | 4.585859 |
| Rust executable leaves, including dependencies | 1.919192 |
| libc allocator leaves | 0.434343 |
| Other libc copies | 0.161616 |
| libc buffer-growth copies | 0.020202 |
| libc HTTP/SDK copies | 0.020202 |
| Kernel FUSE request receive copy | 0.010101 |
| Other libc leaves | 0.121212 |
| Other/unresolved leaves | 0.050505 |

Samples representing 7.404040 CPU seconds contain an unknown frame somewhere in
the callchain, often at its root. This is a callchain-completeness limitation,
not 7.404040 seconds of unknown leaf cost. Retain truncated stacks, and do not
claim an exhaustive optimization proof. The admitted copy is measurable CPU
work; this table alone does not make it the cause of the blocked-read tail.

## The YANET2 optimization clue

Sergey Kacheev's public [issue 2898](https://github.com/yanet-platform/yanet2/issues/2898)
and [PR 2902](https://github.com/yanet-platform/yanet2/pull/2902), at head
`2f6ea3e6dd03b6194e2c21b586039046235ec3a7`, describe out-of-line Rust SDK
accessors across crates. Their machine-code and packet microbenchmark comparison
found Rust-only LTO removed that gap; cross-language helper inlining added no
measurable benefit. The PR is a closed, unmerged proof of concept, not a merged
GeeseFS optimization.

This project's Cargo profile does not explicitly enable cross-crate LTO. The
measured executable is nevertheless a release build, and source inspection did
not find hot-path debug logging. No current evidence attributes GeeseFS's read
tail to the YANET2 accessor mechanism. Debug-symbol availability improves
attribution; it does not imply debug assertions or debug optimization settings.
Keep LTO as a measured candidate if the actual callchains/machine code justify it.

## Tool failures and attribution limits

- The first quiescence parser omitted the `SeaweedFS_` metric prefix and returned
  empty counters. Its quiescence claim is invalid; retain its timing data under
  that limitation.
- An older static bpftrace lacked tracepoint typedefs and later crashed printing
  stack-key aggregates. The replacement is official bpftrace 0.27.1, SHA256
  `c8d570302725bd1a3ed2f23bfa8d83dd78bde95daa98856279343052db5799c9`.
- An intermediate capture mixed namespace-local `pid`/`tid` with scheduler
  init-namespace thread IDs and emitted helper errors/lost events. Reject that
  capture. Explicit `pid(init)`/`tid(init)` and thread-ID mapping fix the scope;
  the subsequent captures have no such errors or reported losses.
- The off-CPU role classifier initially included client/controller threads in
  its daemon summary. Use the actual daemon's main-thread identity to select
  its process, and retain the earlier summary as superseded evidence.
- Observer overhead shares the test resources. Instrumented timings cannot be
  substituted for the historical matched comparison or for unprofiled acceptance.
- Per-GET/body/admission/cache activity is not yet joined to each FUSE request.
  Remaining userspace-service waits and the full representative workload matrix
  are still open; FUSE passthrough and disk-backed caching remain deferred.

## Public primary sources

- [Linux 6.8 CFS bandwidth](https://github.com/torvalds/linux/blob/v6.8/Documentation/scheduler/sched-bwc.rst)
  and [cgroup v2 cpu.max](https://github.com/torvalds/linux/blob/v6.8/Documentation/admin-guide/cgroup-v2.rst).
- [Linux 6.8 FUSE request lifecycle](https://github.com/torvalds/linux/blob/v6.8/fs/fuse/dev.c).
- [bpftrace 0.27.1 namespace tests](https://github.com/bpftrace/bpftrace/blob/v0.27.1/tests/runtime/pid_namespace).
- [FlameGraph](https://github.com/brendangregg/FlameGraph/tree/41fee1f99f9276008b7cd112fca19dc3ea84ac32),
  pinned for stack collapse and SVG rendering; no fork of those tools is added.
