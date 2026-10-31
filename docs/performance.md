# Performance acceptance: UNACCEPTED

The 2026-10-31 [native admission/directory checkpoint](#native-vm-admissiondirectory-checkpoint--2026-10-31)
follows the earlier healthy-warm cache-policy comparison and persistent mixed-mount
observation summarized in the [README](../README.md). Repeated warm-random GETs
disappear and the new VM control meets its offered rate, but tails, throughput
and memory still lose selected gates against Go. Neither checkpoint establishes
a memory plateau.
Full compatibility and comparative acceptance remain open. Cold-start
superiority is not required. See [next work](next-work.md) for confirmed causes
and [change history](change-history.md) for corrections.

## Comparison contract

- Pin public source revisions, compiler versions, executable hashes, dataset
  manifest/generations, kernel, mount arguments and workload order.
- Compare both Go references: experimental
  `5c0884c9e8799bbab93e2d4102fdc15e9c079e0d` and packaged
  `80c214ef14f620d93407454fc76ca01dd71fa25f`. Retained Go binaries use Go 1.25.9;
  an earlier report incorrectly identified its Go 1.22.2 launcher as the compiler.
- Hold budgets, concurrency and resources equal. The earlier sealed diagnostic
  uses 512 MiB application payload, 100,000 entries, four 128 KiB readers and eight
  runtime workers; filesystem/client has 8 CPU / 16 GiB, backend 16 CPU / 12 GiB,
  parent 24 CPU / 28 GiB. The later operation matrix uses 64 MiB payload,
  filesystem/client 8 CPU / 8 GiB, backend 8 CPU / 8 GiB and parent 24 CPU / 16 GiB;
  backend throttling qualifies sequential/eviction results. Persistent/observer
  diagnostics raise backend/parent CPU to 16/32 without changing application or
  filesystem/client budgets. Verify actual limits and pressure on every fixture.
  The latest four-binary comparison instead uses no CPU quota, eight runtime
  workers and 16/12/32 GiB memory maxima for filesystem/client, backend and parent.
  Backend data lives for one paired workload group, with full generation checks
  before scoped bucket cleanup; no global caches are dropped.
- Rotate implementation order and repeat fresh/warm phases on immutable data.
  Verify full bytes and listings before treating a phase as comparable.
- Report throughput and p50/p95/p99, daemon/client user/system CPU, whole-cgroup
  CPU and throttling, peak/steady RSS, allocator accounting, cgroup/file/kernel
  memory separately. Missing metrics remain missing. RSS is not cgroup memory;
  final-fifth RSS is not proof of a plateau; wall time is not CPU cost.
- Keep arithmetic mean rates separate from total work divided by total measured
  time. The 2026-10-30 README tables use the latter for throughput, but report mean
  run percentiles. Rare 30-second successful reads make the distinction material:
  that candidate's sequential mean rate is 238.84 MiB/s, whereas total-work/time is
  115.93. The new VM tables use pooled percentiles. Preserve both aggregations,
  all repetitions and raw outliers.
- Retain unfavorable samples, dependency failures and incomplete attempts outside
  source control. Instrumented profiles establish scoped observations, not
  acceptance timing or a percentage attribution of global p99.

## Reproduction

The release profile uses `lto = "thin"` and `codegen-units = 1`; retain these
settings with executable provenance. Linking takes longer. The matched profile
comparison uses the explicit System allocator; default builds still use jemalloc.
Do not generalize one allocator/workload's gains to every configuration.

Run these inside an isolated Linux/FUSE fixture, in Kubernetes or on an authorized
VM, with an explicitly owned CPU+memory subtree, disposable storage and the
declared Python dependencies:

```bash
export GEESEFS_BENCH_CGROUP_ROOT=/path/to/owned/delegated/subtree
timeout --kill-after=30s 7240s python3 bench/compare.py \
  --go-reference "$GEESEFS_GO_REFERENCE" --workloads bench/workloads.json \
  --output benchmark-results/private/full --timeout 7200
python3 bench/summarize.py benchmark-results/private/full/report.json --table
```

The committed runner owns its data/mounts, records executable provenance and
checks accounting. Its full matrix includes metadata, sequential/random reads,
concurrency, eviction and writers. Diagnostic workload selection, smoke runs or
incomplete-accounting mode cannot pass the full gate. Native sealed diagnostics
use the separately described [fixture](immutable-read-dataset.md).

## Earlier attribution and remaining read-path work

The new comparison retains five rare approximately 30-second successful serial
reads in candidate Rust and twelve in its predecessor, with none in Go. The
persistent run retains eleven and sixteen respectively. P99 alone does not show
these rare stalls. Filesystem/client I/O PSI-some is substantial despite zero
quota throttling, memory pressure and reclaim; it is not proof of physical backend
disk pressure. Correlate FUSE/SDK/body waits and allocation ownership before a
runtime correction. `Filesystem::admit` has a 30-second admission timeout, but a
matching duration alone does not establish it as the cause.

The subsequent native investigation below supplies the missing ownership/FUSE
join and a deterministic failing test, then corrects the application wait. The
earlier captured counts are retained; they are not post-correction observations.

Matched initial-read socket/FUSE joins locate extra Rust client work before GET
write and after required socket bytes. Header wait is similar to experimental Go
in that capture; this does not yet identify the responsible scheduler, admission,
SDK or reply stage. Measure those boundaries, inspect applicable upstream issues
and supported APIs, reproduce a concrete defect, then make a narrow correction.
Do not remove If-Match, checksum validation or bounded ownership to match a weaker
reference path. FUSE passthrough and disk-backed caching remain deferred.

Store raw results, traces, provenance and failures in ignored evidence directories.
Source documentation keeps conclusions and reasons. The operator-requested
README checkpoint contains aggregate timing/RSS/CPU tables; per-run results,
expanded JSON exports, generated flamegraphs and execution journals stay ignored.

## Executable identities

The predecessor runtime is `2bf8184` with release profile `5d970ff`. The candidate
changes the cache policy at `35ee52c`; subsequent documentation changes do not
change compiled inputs. These are exact retained SHA256 values:

| Implementation | Executable SHA256 |
| --- | --- |
| Rust, default jemalloc / ThinLTO | `9ffb86df6efa84b989d8deeaff6fdce6789dee6af9a40478a938955a4b8b38c0` |
| Rust, idle-flush cache policy / default jemalloc / ThinLTO | `b7d1ec6dfe850c829de7ce92d509940fe49f00512e85496081821ed0b881bdf8` |
| Go experimental | `7ad836e0bdd23ea279bb236627e9b130d11c174ae649c7582213a83e5d11a28b` |
| Go packaged | `e04492363d29985ab1f74521482a01a3c6f7fdeeed35ad1ba0faa991370f4683` |

Retained reports are `profiles-acceptance-native-matrix-v4-headroom/report.json`
and `profiles-acceptance-long-v1/report.json`, with whole-series memory analysis
in `profiles-acceptance-long-v1/whole-series-memory-summary.json` for the older
checkpoint. The new reports are
`profiles-proven-compare-loans-v4-isolated/report.json` and
`profiles-proven-long-loans-v1/report.json`; independent outputs are
`comparison-summary.json`, `whole-series-memory-summary.json` and
`final-analysis.json` in their respective result directories. They include
source/build identities, raw latencies, interval resource counters, failures and
sampled allocator/process/cgroup memory. Private fixture orchestration is not a
committed portable reproduction command; `bench/compare.py` above provides the
public runner, but is not a claim to reproduce the private mixed/ablation harness.

The idle-flush candidate is source commit `35ee52c7713c2dd29e6787a81f6ced867de2c1b6`,
built with Rust 1.99.1 / LLVM 23.1.1. All 25 compiled inputs match that commit.
Its System-allocator comparison build has SHA256
`f5e0273f8fe1b29e5f5f881c5915094c716e8d45b653734cc58258631e5f4c0b`;
building that feature is not a mounted performance result.

## Heap observation boundary

Go heap accounting uses one-second observations of `runtime.MemStats.HeapAlloc`
through `/debug/pprof/heap?debug=1`, without a forced-GC parameter. A separate
bounded observer retains request timings and counters; its server work remains
charged to Go. Only fully in-window observations contribute. Fewer than two
samples leave peak/steady heap missing rather than substituting a final snapshot.
These are sampled peaks and late-half means, not exact peaks or plateau evidence.
Very short workloads therefore still need longer representative measurements;
observer overhead must be checked before interpreting a performance comparison.
The original Go buffer pool itself invokes `debug.FreeOSMemory` at its unchanged
allocation threshold and under pressure. Retain the resulting `NumForcedGC` delta;
the prohibition concerns benchmark-triggered collection, not changing the
reference's own policy or rejecting an otherwise valid reference observation.

The earlier native screening, CPU/off-CPU profiles and first partial cache-policy
comparison passed Go `--gc-interval 0`. That disables its default 250 MiB
allocation-triggered collection threshold, although pressure GC remains enabled.
Those runs establish configured diagnostics, not default-policy comparative
acceptance. The complete pre-loan operation matrix and persistent observation did
not override that threshold. Subsequent candidate comparisons remove the override
and require `GOGC` and `GOMEMLIMIT` to be unset. Observer-triggered collection and
reference-owned collection remain distinct.

## Native VM admission/directory checkpoint — 2026-10-31

This is a distinct diagnostic design on the operator-authorized OpenCode VM:
Linux 7.0.0-38-generic, 64 AMD EPYC Milan vCPUs, about 125 GiB RAM, loopback
SeaweedFS 4.51 and real FUSE. Each persistent cycle runs metadata, sequential,
random, concurrent, eviction and writer/fsync checks. Payload is 256 MiB with
1,024 metadata entries; random uses 512 seeded 4 KiB reads per worker, concurrent
has four workers, and writer/fsync writes **1 MiB**, not the earlier 16 MiB.
All four implementations use a 64 MiB application budget, 100,000 cache entries,
eight runtime workers, no CPU quota and 16/12/32 GiB memory maxima for
daemon/client, backend and parent. Go GC defaults are unchanged; its own forced
collections are distinct from benchmark-triggered GC, which is not requested.

Final runtime source is `67f7aec0dc78be355c431c8c72b73bd9a248378f`, Rust 1.99.1,
default jemalloc/ThinLTO. New VM builds have their own executable identities:

| Implementation | Executable SHA256 |
| --- | --- |
| Rust before, runtime `35ee52c` / checkout `69eb0a7` | `6a6ca9c379c63b17c41c3fc2e1515d3e4ba38a9c524ac0c4e4ce04577325ea58` |
| Rust corrected, runtime `67f7aec` | `666de62cc08a7bf98866c1a6adf3569a90f812049da51e85b66be6b1ca7bdb49` |
| Go experimental `5c0884c9`, Go 1.25.9 | `a4b7a8c3e949b3dac86b719a740b2e9bf664474d65cefba67052c3c724a5e9dc` |
| Go packaged `80c214ef`, Go 1.25.9 | `c0c3c0aec1d435495adef4bfefff04eefae644dab9de028ccfce09e71e8a9815` |

### Fixed-work control

Order is prior Rust, corrected Rust, Go experimental, Go packaged, each on a fresh
mount with 60 cycles. All 1,440 passes and four final payload generation/full-byte
checks pass. Elapsed includes controller intervals, but excludes mount startup;
CPU sums the active phase counter windows. Peaks are sampled. Late means select
the temporal latter half of the complete raw active series, not a bounded ring
buffer. These are not a memory plateau or equal-rate comparison. MiB = 2^20 bytes.

| Implementation | Elapsed s | Phase CPU daemon / client / cgroup s | Whole-cgroup CPU s / s per cycle | Daemon RSS peak / late MiB | Daemon+client RSS peak / late MiB | Cgroup peak / late MiB |
| --- | ---: | --- | --- | --- | --- | --- |
| Rust before | 251.35 | 128.89 / 98.06 / 227.01 | 227.28 / 3.788 | 184.6 / 156.8 | 247.5 / 219.8 | 497.3 / 319.8 |
| Rust corrected | 200.89 | 121.27 / 97.74 / 219.07 | 219.34 / 3.656 | 180.3 / 156.4 | 243.1 / 219.2 | 476.6 / 263.6 |
| Go experimental | 288.61 | 766.20 / 102.45 / 868.67 | 868.91 / 14.482 | 316.6 / 174.3 | 379.6 / 237.2 | 547.2 / 279.4 |
| Go packaged | 379.29 | 1437.55 / 103.46 / 1540.95 | 1541.56 / 25.693 | 344.4 / 203.5 | 407.2 / 266.3 | 821.5 / 346.4 |

| Implementation | File / kernel late MiB | Allocator peak / late MiB |
| --- | --- | --- |
| Rust before | 118.4 / 3.24 | 89.1 / 80.0 |
| Rust corrected | 62.3 / 3.15 | 86.1 / 78.2 |
| Go experimental | 71.5 / 3.65 | 243.8 / 113.8 |
| Go packaged | 81.6 / 3.66 | 278.5 / 139.3 |

Allocator means jemalloc allocated or Go HeapAlloc, with different scopes and
50 ms / 1 s sampling. Client late RSS is about 62.8–63.0 MiB. Most of the cgroup
decrease versus prior Rust is file-cache occupancy; daemon late RSS barely changes.
Do not describe that decrease as a demonstrated application-memory improvement.
Go experimental/packaged observed `NumForcedGC` deltas are 422/548; these are
the reference's own unchanged policy, not observer-requested collections.

Throughput below is total work divided by summed phase time, in MiB/s except
metadata operations/s. Tails are independently pooled p50/p95/p99 over all 60
cycles, including one initial cycle. Cold phase times and warm-only rates remain
in the retained summaries; these pooled percentiles are not mean run percentiles.

| Workload | Implementation | Throughput | Pooled p50 / p95 / p99 ms |
| --- | --- | ---: | --- |
| metadata | Rust before | 2,915 ops/s | 0.00481 / 0.04754 / 0.10648 |
| metadata | Rust corrected | 25,006 ops/s | 0.00487 / 0.05318 / 0.07130 |
| metadata | Go experimental | 51,748 ops/s | 0.00428 / 0.00716 / 0.02195 |
| metadata | Go packaged | 45,338 ops/s | 0.00435 / 0.00757 / 0.02605 |
| sequential | Rust before | 503.66 | 0.767 / 1.706 / 7.710 |
| sequential | Rust corrected | 507.22 | 0.763 / 1.775 / 7.624 |
| sequential | Go experimental | 200.16 | 2.856 / 11.871 / 22.304 |
| sequential | Go packaged | 125.61 | 5.601 / 20.227 / 29.825 |
| random | Rust before | 1.843 | 1.999 / 3.003 / 3.422 |
| random | Rust corrected | 1.820 | 1.992 / 3.047 / 3.455 |
| random | Go experimental | 1.940 | 1.886 / 2.876 / 3.296 |
| random | Go packaged | 1.889 | 1.921 / 2.913 / 3.539 |
| concurrent | Rust before | 8.957 | 1.637 / 2.417 / 2.998 |
| concurrent | Rust corrected | 9.137 | 1.603 / 2.352 / 2.923 |
| concurrent | Go experimental | 8.941 | 1.603 / 2.384 / 3.016 |
| concurrent | Go packaged | 8.937 | 1.580 / 2.458 / 3.264 |
| eviction | Rust before | 264.70 | 0.777 / 3.078 / 5.527 |
| eviction | Rust corrected | 557.96 | 0.771 / 2.920 / 5.495 |
| eviction | Go experimental | 205.29 | 2.865 / 11.699 / 23.109 |
| eviction | Go packaged | 131.05 | 5.322 / 19.793 / 31.123 |
| writer/fsync | Rust before | 21.498 | 21.622 / 30.180 / 129.636 |
| writer/fsync | Rust corrected | 21.226 | 22.240 / 30.672 / 31.709 |
| writer/fsync | Go experimental | 30.157 | 20.524 / 27.389 / 30.421 |
| writer/fsync | Go packaged | 26.529 | 20.795 / 29.130 / 30.514 |

Prior Rust retains one 30,051.716 ms read; corrected Rust has no call over 20 s.
Serial maxima are 87.695/22.671 ms for corrected sequential/eviction; random max
is 62.331 ms. Metadata maximum, including initial listing, is 419.906 ms.
Random throughput/p99 and metadata p50/p95 slightly regress versus prior Rust;
metadata, random and writer measurements retain losses to Go. Average elapsed
does not compensate for them.

Filesystem/client counters show no throttling, reclaim or OOM. Backend counters
show 48 scanned/stolen pages during the prior-Rust interval and zero for the
other three intervals; retain this resource qualifier. Bounded quality-tool
checks overlap part of that predecessor interval on the shared VM. No exclusive
host, independently zero observer overhead or repeatable all-metric win is claimed.
The separate instrumented 60-cycle capture, intermediate candidate controls and
partial reverse-order attempt are retained, not pooled with this final binary.
The partial control stops before Rust worker startup because of an orchestration
script indentation error, not a filesystem failure.

### Equal-offered-rate control

Reverse order is Go packaged, Go experimental, corrected Rust, prior Rust: 30
cycles each, scheduled one cycle every 10 s. All 720 passes and four final
full-byte/generation checks pass. Every implementation takes 300.00 s with zero
late cycles and no call over 20 s. The absence of the rare stall in prior Rust at
this offered rate is retained; it is not additional evidence of a fix's effect.

Phase CPU below sums the active phase windows. Whole-cgroup CPU also includes
inter-cycle idle/background work; its normalization is seconds per mixed cycle,
not per homogeneous operation. Raw windows retain user/system CPU separately for
daemon, client and cgroup. Memory spans first through last phase, including the
inter-cycle gaps but excluding the final pacing sleep; late means use that
window's temporal latter half. Client late RSS is 62.8–62.9 MiB.

| Implementation | Elapsed s | Phase CPU daemon / client / cgroup s | Whole-cgroup CPU s / s per cycle | Daemon RSS peak / late MiB | Daemon+client RSS peak / late MiB | Cgroup peak / late MiB |
| --- | ---: | --- | --- | --- | --- | --- |
| Rust before | 300.00 | 63.28 / 49.24 / 112.53 | 115.18 / 3.839 | 175.9 / 152.5 | 238.6 / 215.3 | 484.1 / 394.1 |
| Rust corrected | 300.00 | 63.03 / 49.20 / 112.24 | 114.93 / 3.831 | 181.8 / 151.6 | 244.7 / 214.5 | 478.5 / 389.8 |
| Go experimental | 300.00 | 374.86 / 50.40 / 425.27 | 428.46 / 14.282 | 310.8 / 153.7 | 373.6 / 216.5 | 529.3 / 350.6 |
| Go packaged | 300.00 | 585.28 / 52.10 / 637.44 | 640.10 / 21.337 | 341.1 / 175.2 | 404.0 / 238.1 | 644.7 / 371.4 |

| Implementation | File / kernel late MiB | Allocator peak / late MiB |
| --- | --- | --- |
| Rust before | 197.1 / 3.33 | 88.3 / 76.5 |
| Rust corrected | 193.8 / 3.37 | 86.7 / 76.2 |
| Go experimental | 167.4 / 3.74 | 243.6 / 95.7 |
| Go packaged | 155.3 / 3.78 | 270.3 / 115.2 |

The same throughput/pooled-tail definition applies, over these 30 cycles:

| Workload | Implementation | Throughput | Pooled p50 / p95 / p99 ms |
| --- | --- | ---: | --- |
| metadata | Rust before | 3,193 ops/s | 0.00575 / 0.05434 / 0.07488 |
| metadata | Rust corrected | 9,020 ops/s | 0.00692 / 0.06640 / 1.26849 |
| metadata | Go experimental | 33,795 ops/s | 0.00464 / 0.00761 / 0.02886 |
| metadata | Go packaged | 31,387 ops/s | 0.00434 / 0.00756 / 0.02510 |
| sequential | Rust before | 509.58 | 0.743 / 1.797 / 7.914 |
| sequential | Rust corrected | 516.02 | 0.730 / 1.687 / 7.706 |
| sequential | Go experimental | 203.51 | 2.765 / 11.543 / 23.422 |
| sequential | Go packaged | 145.90 | 4.466 / 16.673 / 25.711 |
| random | Rust before | 1.842 | 2.020 / 2.972 / 3.336 |
| random | Rust corrected | 1.778 | 2.047 / 3.094 / 3.435 |
| random | Go experimental | 1.856 | 1.984 / 2.951 / 3.341 |
| random | Go packaged | 1.937 | 1.878 / 2.854 / 3.321 |
| concurrent | Rust before | 8.931 | 1.631 / 2.379 / 3.016 |
| concurrent | Rust corrected | 8.949 | 1.619 / 2.475 / 3.120 |
| concurrent | Go experimental | 9.103 | 1.591 / 2.373 / 3.061 |
| concurrent | Go packaged | 8.778 | 1.614 / 2.483 / 3.194 |
| eviction | Rust before | 583.26 | 0.724 / 2.738 / 5.205 |
| eviction | Rust corrected | 570.22 | 0.752 / 2.826 / 5.071 |
| eviction | Go experimental | 212.36 | 2.678 / 11.508 / 23.726 |
| eviction | Go packaged | 156.90 | 4.217 / 15.868 / 25.503 |
| writer/fsync | Rust before | 25.204 | 18.900 / 22.949 / 28.959 |
| writer/fsync | Rust corrected | 24.121 | 19.938 / 29.471 / 30.300 |
| writer/fsync | Go experimental | 29.824 | 21.616 / 27.795 / 28.372 |
| writer/fsync | Go packaged | 25.983 | 22.322 / 29.353 / 36.077 |

Metadata active time improves 9.62 to 3.41 s, but its pooled p99 worsens from
0.07488 to 1.26849 ms. In corrected Rust, cycle 28 alone has 345 metadata calls
above 1 ms; this is a late-run observation, not just cold startup. Its mechanism
still needs attribution. Random, concurrent and writer tails also retain losses.
Corrected Rust's late cgroup memory exceeds both Go references despite lower
sampled peak and late daemon RSS. Its peak daemon RSS also increases versus prior
Rust. No throttling, reclaim or OOM occurs in filesystem/client or backend.
Go experimental/packaged observed `NumForcedGC` deltas are 212/260.

These are two differently paced, fixed-order controls, not independent repetitions
of one design or an exclusive-host/observer-free experiment. They show useful
corrections and adverse measurements, not repeatable all-metric acceptance.
Both controllers exit successfully; their owned mounts/processes/backend stop.
Private evidence is under `vm-20261031/`: `final-work-v1/report.json` and
`final-rate-v1/report.json`, raw memory/allocator/CPU windows, independently derived
summaries, source/executable identity checks and cleanup receipts. As with earlier
private mixed harnesses, the controller is retained locally rather than claimed
to be the public `bench/compare.py` command above.
