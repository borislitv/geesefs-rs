# Classic FUSE dispatch diagnosis — 2026-11-02

**UNACCEPTED.** Plan B uses the released fuser 0.18.0 transport on Linux 7.3.1.
It does not adopt a fork, replace the SDK or change freshness, durability or cache
budgets. The target is LXC serving nginx, rsync and golist; these VM diagnostics
do not establish target-container or actual-service acceptance.

## Attribution before changes

Frozen observation-age source `1b13033` and both identified Go references complete
180 BPF-instrumented mixed phases, 108 further immutable-window BPF phases and
180 perf-instrumented phases. The authoritative BPF reduction has no unfinished
requests. CPU-clock sampling uses 499 Hz with monotonic timestamps and 8 KiB DWARF
stacks; empty unwinds are joined to raw leaf samples by tid/time, not discarded.
Unresolved symbols remain unresolved. Profile timings are not acceptance timings.

| Rust workload | Mean READ send→sent, µs | Mean READ sent→end, µs | Receive-thread CPU % | Inclusive transport-stack CPU % |
| --- | ---: | ---: | ---: | ---: |
| Sequential | 29.75 | 672.85 | 1.06 | 20.28 |
| Random | 49.23 | 1,462.32 | 5.49 | 5.74 |
| Concurrent | 37.54 | 1,223.57 | 4.12 | 5.34 |

The receive loop is mostly asleep in `fuse_dev_do_read`, not CPU-saturated.
Sequential transport CPU includes reply/copy work on Tokio threads; it is not
all receive-loop cost. Random/concurrent notification stacks account for
15.26%/16.81% of CPU, partly from earlier closes. Background invalidation spilling
into a writer phase must not be attributed to fsync without a request join.
Request sums overlap under concurrency; scheduler/off-CPU totals include idle
sleeps. Neither these totals nor inclusive CPU stacks partition elapsed wall time.

A separate 1,000-traversal metadata profile yields 1,143 warm-phase samples:
`cached_directory_page` is 21.61% exclusive/27.82% inclusive; the receive thread
is 8.14% and transport stacks 12.07%. This identifies application page validation
as substantial work, not a reason to repeat the rejected cached-observation trial.

The cached-read probe retains the 256 MiB object and unchanged 64 MiB/100,000-entry
budgets, but deliberately warms an 8 MiB subregion. Aligned 4 KiB `O_DIRECT`
`preadv` bypasses the kernel page cache. BPF checks exactly 8,192 FUSE READs per
cached phase and no daemon TCP sends; separate uncached reads produce TCP sends,
providing a positive control. All bytes are checked independently. This is a
cache-residency diagnostic, not the representative working-set acceptance test.

Across 1,834 cached-read perf samples, kernel scheduling is 40.24%, Tokio 7.14%,
application 7.09%, receive-thread CPU 29.12% and inclusive transport stacks 26.99%.
There is measurable handoff cost in this workload. The separate warm traversal
still spends 22.15% exclusive CPU in `cached_directory_page`.

## One, two and four classic receive loops

The trial uses supported `Config::n_threads` and `clone_fd`, with one identical
binary for all counts. Actual fuser thread and `/dev/fuse` fd counts are checked.
Four rotated repetitions compare retained Rust, trial 1/2/4 loops and both Go
references: 24 mounts, **864 checked phases and 24 final byte/generation checks**.
All owners stop normally; memory-limit, OOM, reclaim and quota-throttle interval
counters remain zero.

This is a short six-cycle screening design: later five cycles are post-initial
mixed work, not isolated warm work or a memory plateau. Rust/Go workers remain
eight, application budgets 64 MiB/100,000 entries, payload 256 MiB, directory
1,024 entries, random reads 512 × 4 KiB, concurrency four and writer 1 MiB.
Owned filesystem/client, backend and parent memory ceilings are 16/12/32 GiB.
There is no CPU quota or global CPU cap; affinity is the guest's online `0-63`,
not the earlier standalone C laboratory's four-CPU placement. Shared-host variance
and all four pairs remain visible in private raw evidence.

| Workload | Two/one throughput ratios, four pairs | Four/one throughput ratios, four pairs |
| --- | --- | --- |
| Metadata | 1.105, 1.008, 1.139, 1.086 | 0.826, 1.040, 1.205, 1.061 |
| Sequential | 1.128, 1.029, 0.948, 1.044 | 1.107, 1.016, 1.002, 0.954 |
| Random | 0.978, 0.942, 0.952, 0.951 | 0.844, 0.875, 0.830, 0.875 |
| Concurrent | 1.045, 0.968, 0.953, 1.037 | 1.032, 0.991, 0.955, 0.975 |
| Eviction | 1.048, 0.975, 0.931, 1.027 | 1.059, 0.975, 0.974, 1.024 |
| Writer/fsync | 1.051, 0.956, 0.862, 1.108 | 0.491, 1.104, 0.853, 0.952 |

Both treatments have **zero of four joint wins** on throughput, p50/p95/p99, CPU
and peak/late memory. More loops do not fix the representative random-read gap.
fuser allocates a `16 MiB + 4 KiB` receive buffer per loop: mean late live
allocation rises from 76.24 MiB to 92.48/124.60 MiB. These allocations are outside
the application payload budget; their whole backing need not be resident.
The rejected premise is that additional receive capacity removes the small-read
cost: the original receiver mostly waits, service/reply work remains on Tokio,
and each additional receiver owns a large buffer. Do not add another receive-loop
variant without new attribution.

Means of four per-run measurements follow. RSS peaks are sampled; cgroup peaks
also include startup. Late means cover the second half of each short run.
Allocator columns mean Rust requested live bytes or Go HeapAlloc, not identical
allocator internals. CPU is consumed user+system time for the complete fixed work,
including the initial cycle; it is not elapsed time.

| Implementation | Daemon RSS peak/late MiB | Daemon+client RSS peak/late MiB | Live allocation peak/late MiB | Cgroup peak/late MiB | Late file/kernel MiB | CPU daemon/client/cgroup s |
| --- | --- | --- | --- | --- | --- | --- |
| Retained Rust | 176.65 / 154.55 | 238.76 / 217.01 | 83.60 / 76.03 | 464.63 / 243.55 | 44.75 / 3.03 | 12.24 / 8.99 / 21.23 |
| Trial, one loop | 174.44 / 152.59 | 235.33 / 214.94 | 83.35 / 76.24 | 460.34 / 239.22 | 43.75 / 3.04 | 12.42 / 9.03 / 21.45 |
| Trial, two loops | 174.24 / 152.52 | 235.83 / 214.93 | 99.39 / 92.48 | 458.05 / 240.95 | 44.47 / 3.07 | 12.50 / 9.03 / 21.53 |
| Trial, four loops | 177.26 / 155.02 | 238.19 / 217.26 | 131.51 / 124.60 | 463.38 / 237.45 | 38.84 / 3.16 | 12.56 / 9.16 / 21.73 |
| Go experimental | 309.73 / 165.84 | 371.96 / 228.40 | 232.49 / 99.80 | 486.54 / 245.36 | 56.76 / 3.53 | 46.19 / 9.71 / 55.89 |
| Go packaged | 328.36 / 181.50 | 390.75 / 244.00 | 223.12 / 118.56 | 588.49 / 268.14 | 57.77 / 3.60 | 56.28 / 9.72 / 66.01 |

The trial's one-loop random/concurrent rates are 1.857/9.083 MiB/s, with pooled
p99 3.347/2.913 ms; experimental Go is 1.854/8.735 MiB/s and 3.428/3.102 ms.
Writer throughput remains lower: 22.62 versus 30.48/26.47 MiB/s for Go. These short
same-kernel results neither supersede the earlier sustained controls nor establish
a joint correctness-equivalent Go win.

The isolated cached diagnostic repeats the 1/2/4 control separately: **144 phases
and 12 final byte/generation checks**, again zero joint wins in four pairs for
either extra-loop treatment. Cached throughput ratios for two loops are
1.087/0.979/0.949/1.004 and for four loops 0.995/0.943/0.934/0.959. One-loop pooled
p99 is 0.14690 ms versus two-loop 0.15710 ms. Additional receive loops are rejected.

## First-poll read experiment

A minimal supported-API trial polls the existing admitted read future once on the
receive thread, then moves the same pending future into Tokio. This preserves the
reply, permit, deadline, generation/TTL checks and refill state rather than adding
a second read/cache implementation. A native regression occupies both async workers
after warming direct-IO bytes: the predecessor times out waiting for the read;
the trial returns the exact bytes before those workers are released. All twelve
experimental native directory contracts pass with one, two and four receive loops.
The frozen default release also passes available rw/ro, nginx, rsync and injected
fsync failure/recovery contracts; unsupported positive LINK remains an explicit gap.

Four rotated one-loop source pairs then check **96 isolated cached/metadata
phases and eight final generations**. Only `src/fuse.rs` differs among runtime
inputs. Cached throughput ratios are 1.019/1.241/1.213/1.188; p99 improves in every
pair. The following totals pool the later five cycles of each run; latency is
pooled independently from raw per-call samples. Client byte validation is outside
individual read latency but inside phase elapsed time and client CPU.

| Workload | Pre-inline rate | First-poll rate | Pre-inline p50/p95/p99 ms | First-poll p50/p95/p99 ms | Daemon CPU/work, before/after |
| --- | ---: | ---: | --- | --- | --- |
| Cached 4 KiB reads | 18.527 MiB/s | 21.505 MiB/s | 0.05945 / 0.11082 / 0.147609 | 0.02712 / 0.04626 / 0.07153 | 0.013328 / 0.004422 s/MiB |
| Warm metadata | 145,701 ops/s | 146,183 ops/s | 0.00348 / 0.00438 / 0.01762 | 0.00346 / 0.00455 / 0.01817 | 0.002274 / 0.002190 s/1,000 ops |

| Isolated source | Daemon RSS peak/late MiB | Daemon+client RSS peak/late MiB | Live allocation peak/late MiB | Cgroup peak/late MiB | Late file/kernel MiB | CPU daemon/client/cgroup s |
| --- | --- | --- | --- | --- | --- | --- |
| Pre-inline | 49.18 / 47.14 | 83.42 / 81.68 | 39.00 / 37.27 | 62.06 / 60.78 | 0.28 / 2.23 | 3.44 / 10.00 / 13.43 |
| First-poll | 48.04 / 45.89 | 82.29 / 80.36 | 38.86 / 38.00 | 60.93 / 58.57 | 0.25 / 2.23 | 1.73 / 10.27 / 12.00 |

This does **not** pass the joint gate: metadata p95 loses in all four pairs,
late live allocation increases in all four, and RSS/other memory losses remain
in individual pairs. Lower pooled RSS does not establish reproducibility or a
plateau. A genuine cache-hit mechanism improvement alone is insufficient.

The separate representative source-pair control completes **576 phases and 16
final byte/generation checks**, rotating pre-inline Rust, first-poll Rust and both
Go references through four repetitions. Sources, one-loop topology, normal cleanup
and zero adverse interval counters are independently checked. The same short
six-cycle design and unchanged budgets apply; these are not sustained-run results.

| Workload | Pre-inline rate | First-poll rate | Pre-inline p50/p95/p99 ms | First-poll p50/p95/p99 ms | Go experimental rate / p99 ms | Go packaged rate / p99 ms |
| --- | ---: | ---: | --- | --- | --- | --- |
| Metadata | 121,221 ops/s | 118,173 ops/s | 0.00351 / 0.00759 / 0.01987 | 0.00357 / 0.00728 / 0.02053 | 134,281 / 0.01867 | 124,110 / 0.02037 |
| Sequential | 648.887 MiB/s | 650.323 MiB/s | 0.547949 / 1.307279 / 7.237953 | 0.547219 / 1.212989 / 7.447253 | 363.729 / 15.115836 | 324.623 / 13.563497 |
| Random | 1.852 MiB/s | 1.919 MiB/s | 1.907878 / 3.001867 / 3.401317 | 1.868388 / 2.957258 / 3.353737 | 1.970 / 3.355307 | 1.883 / 3.666167 |
| Concurrent | 8.998 MiB/s | 9.159 MiB/s | 1.628988 / 2.349488 / 2.904277 | 1.620929 / 2.270188 / 2.751657 | 9.009 / 2.904887 | 8.651 / 3.244287 |
| Eviction | 655.399 MiB/s | 650.220 MiB/s | 0.541819 / 1.840848 / 6.136644 | 0.546939 / 1.848068 / 6.110604 | 351.640 / 19.558681 | 320.127 / 16.604354 |
| Writer/fsync | 23.976 MiB/s | 22.538 MiB/s | 19.274892 / 28.133513 / 32.450569 | 20.121640 / 28.639292 / 47.968844 | 30.846 / 28.790632 | 28.041 / 26.197145 |

| Mixed source | Daemon RSS peak/late MiB | Daemon+client RSS peak/late MiB | Live allocation peak/late MiB | Cgroup peak/late MiB | Late file/kernel MiB | CPU daemon/client/cgroup s |
| --- | --- | --- | --- | --- | --- | --- |
| Pre-inline | 174.32 / 155.16 | 236.06 / 217.48 | 83.28 / 76.09 | 459.82 / 243.15 | 44.27 / 3.02 | 12.17 / 9.21 / 21.38 |
| First-poll | 174.10 / 156.84 | 235.38 / 219.03 | 83.53 / 76.06 | 462.09 / 244.08 | 44.33 / 3.02 | 12.08 / 9.01 / 21.09 |
| Go experimental | 312.71 / 165.99 | 374.43 / 228.54 | 208.54 / 101.94 | 486.72 / 248.04 | 57.46 / 3.48 | 45.99 / 9.63 / 55.61 |
| Go packaged | 329.57 / 180.70 | 392.23 / 243.14 | 243.39 / 119.04 | 574.16 / 267.44 | 57.27 / 3.57 | 55.95 / 9.74 / 65.70 |

There are **zero joint wins in four mixed pairs**. Random throughput improves
1.029/1.011/1.043/1.059 times, but metadata loses in three pairs and writer
throughput in two (ratios 1.024/0.975/1.074/0.748). Pooled writer p99 increases
32.45→47.97 ms; mean late RSS increases 155.16→156.84 MiB. These observations do
not identify the retained owner's cause, but they reject adoption under the
existing gate. Do not infer a whole-filesystem win from removing one handoff.

Discard the first-poll implementation and its experiment-only regression, along
with the multi-loop CLI. Keep the confirmed rename correction and its concurrent
behavior test. All other runtime inputs return to the observation-age baseline;
future S3/setup/retained-owner work needs new attribution, not another dispatch
variant under the same failed premise.

## Negotiated classic transport limits

The retained diagnostic INIT syscall pair records kernel protocol 7.46 and fuser
7.40, `max_write=1,048,576`, `max_readahead=131,072`, and `max_pages=256` with
`FUSE_MAX_PAGES` enabled. `max_background=16` and `congestion_threshold=12` are
distinct from application `max_requests=64`. The per-loop 16 MiB allocation is
also not the negotiated write limit. No transport limit is enlarged to obtain
the reported rates, and traced native-contract timings are not benchmark data.

## Confirmed rename deadlock

The new native concurrency regression hangs with both one and two loops on the
pre-fix source. Four clients interleave listing/reading with create, fsync, rename
and unlink, then verify dirty-close bytes after unmount. Kernel stacks place a
Tokio worker in `fuse_reverse_inval_entry`: redundant synchronous post-rename
notifications wait for a parent-directory operation while holding the notifier
mutex and occupying an executor thread.

Linux [rename/dentry handling](https://github.com/torvalds/linux/blob/v7.3/fs/fuse/dir.c)
already maintains these dentries. The
[libfuse notification contract](https://github.com/libfuse/libfuse/blob/fuse-3.18.3/include/fuse_lowlevel.h)
warns against related operation/lock paths, and its
[passthrough rename](https://github.com/libfuse/libfuse/blob/fuse-3.18.3/example/passthrough_ll.c)
only sends the reply. Remove both extra entry notifications; do not add a queue
or remove the separately required last-close inode invalidation. All eleven native
directory/concurrency contracts then pass independently with one, two and four
loops. This is a correctness fix, not a measured speed claim.

## Evidence and limits

The retained rename-only runtime is `0e44bd84f0a6498afd852339a41cacccf2381972`,
release SHA256 `9949ad9f8041397d5ca7583dbb163af621de51228472901e1ca17d37f5de0200`.
Its own 261-case allocator suites, eleven native contracts, matching instrumented
and release clients pass; [quality/coverage and compatibility limits](testing.md)
remain explicit. These checks are not performance acceptance of the discarded trials.

Observation-age baseline SHA256 is
`df24b6bdd42920b7989db2ed7c44855b9fc2bb1cf53292ee60ce023eb42138a8`;
the same-binary loop trial is
`88e1c55fd771ff30a24de462a93dcd1c872d63275669c87165aefcf6a5db5fc4`.
The first-poll trial is
`7f425139778f61cbe864d4f7e851d40f00a5b905d1677252af96e72e0bffa303`.
Versioned inputs, exact commands, original failures, per-phase raw latency arrays,
CPU/work and full memory series are retained under ignored evidence groups
`attribution-{bpf,perf}-v1`, `attribution-bpf-v2`, `attribution-metadata-perf-v1`,
`classic-loops-pairs-v1`, `cached-loops-pairs-v2`,
`cached-loops-{bpf-control,perf}-v1`, `inline-cached-pairs-v1` and
`inline-mixed-pairs-v1`. Detailed execution
and results belong there and in the Tracker report, not in source diffs.

Long-lived memory, actual LXC/nginx/rsync/golist operation, actual-service IAM/SSE,
full Linux/S3 compatibility, whole-file CRAP and broad mutation acceptance remain
open. Plan A still requires a released upstream transport and verified LXC policy;
a measured cache-hit transport share above 10% does not satisfy those other gates.
