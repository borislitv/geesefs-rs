# Sealed native S3 read checkpoint (2026-10-27)

**Comparison valid for the additional explicit-metadata workload; overall acceptance false.**

All 40 phases completed in 659.123 seconds. Each phase verified 3,000 files and
8,298,727,528 bytes; all 120,000 file hashes and metadata checks passed.
Native conditional full reads and exact listings passed before and after.
The independent verifier recomputed 280 CPU windows and every memory summary.

See [methodology and reproduction](immutable-read-dataset.md) and
[the complete numeric receipt](immutable-read-results-20261027.json).

The retained preceding plain list/stat diagnostic failed 60,000 Go metadata
checks: standard S3 listings lack user metadata, which both Go references
explicitly require for these options. Their early exits are not speed wins.
The additional workload reads all four xattrs, forces GNU stat attribute
revalidation, then performs ordinary stat and complete payload reads for every
implementation. It preserves all metadata checks and cache budgets, and its
additional work remains included in whole-workload time and CPU.

Each row is the mean of five measured phases; throughput also shows population
standard deviation. MB is decimal. Fresh means a new mount/process, not cold
host or backend caches. Warm revisits the entire dataset, larger than cache.

## Payload throughput, CPU and process RSS

| Implementation | State | Read MB/s mean ± SD | Daemon RSS peak / steady MB | Client RSS peak / steady MB | Daemon user / system CPU s | Client user / system CPU s | Whole cgroup CPU s |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| go-experimental | fresh | 639.124 ± 12.823 | 1293.166 / 1202.006 | 56.889 / 53.640 | 4.714 / 8.388 | 7.159 / 6.999 | 28.085 |
| go-experimental | warm-revisit | 638.752 ± 25.833 | 1277.314 / 1206.806 | 57.250 / 53.407 | 4.080 / 7.466 | 6.761 / 7.960 | 27.138 |
| go-packaged | fresh | 632.244 ± 37.402 | 1277.608 / 1214.818 | 57.229 / 53.033 | 4.800 / 8.574 | 7.114 / 7.105 | 28.459 |
| go-packaged | warm-revisit | 621.895 ± 23.250 | 1287.438 / 1194.257 | 56.868 / 53.569 | 4.266 / 7.796 | 6.878 / 8.124 | 28.014 |
| rust | fresh | 578.372 ± 8.150 | 538.039 / 360.590 | 57.305 / 53.299 | 4.456 / 10.734 | 7.635 / 6.879 | 30.475 |
| rust | warm-revisit | 541.515 ± 21.797 | 636.854 / 360.942 | 56.999 / 53.313 | 3.836 / 10.408 | 7.825 / 8.539 | 31.456 |
| rust-pre-coherence | fresh | 604.051 ± 8.954 | 534.988 / 358.987 | 56.419 / 53.441 | 4.308 / 10.426 | 7.239 / 6.611 | 29.313 |
| rust-pre-coherence | warm-revisit | 556.979 ± 5.999 | 630.645 / 359.990 | 57.305 / 53.469 | 3.802 / 10.426 | 7.467 / 8.016 | 30.473 |

RSS peak is each phase's observed maximum, then averaged across five phases.
Steady is the mean of available samples in the final fifth, not proof of a
stationary plateau. Daemon/client RSS is individual and may double-count
shared pages; the JSON also retains a pointwise combined maximum. GNU stat's
waited-child RSS is separate; cgroup measurements include that child and the
sampler/controller. Read throughput excludes listing and explicit metadata
work; whole-workload CPU includes them. CPU seconds are counter deltas, not
elapsed time. User/system, normalized work cost and throttling remain in JSON.

## Memory ownership and backend scope

| Implementation | State | Cgroup peak / steady MB | Steady anon / file / kernel MB | Daemon PSS peak / steady MB | Allocator peak / steady MB | Backend RSS peak / steady MB | Backend user / system CPU s |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| go-experimental | fresh | 4296.545 / 4293.736 | 1278.356 / 2978.235 / 36.121 | 1287.382 / 1199.499 | missing / missing | 842.510 / 821.274 | 8.538 / 17.916 |
| go-experimental | warm-revisit | 4298.557 / 4293.642 | 1284.221 / 2970.111 / 38.307 | 1270.587 / 1204.955 | missing / missing | 804.619 / 776.340 | 7.206 / 17.710 |
| go-packaged | fresh | 4298.751 / 4293.443 | 1291.775 / 2964.369 / 36.077 | 1273.942 / 1213.953 | missing / missing | 832.584 / 808.621 | 8.722 / 18.190 |
| go-packaged | warm-revisit | 4297.310 / 4293.732 | 1271.506 / 2983.014 / 38.252 | 1278.589 / 1189.020 | missing / missing | 809.410 / 770.482 | 7.152 / 17.036 |
| rust | fresh | 4295.896 / 4227.523 | 453.700 / 3736.829 / 35.353 | 513.791 / 351.460 | 525.263 / 302.984 | 815.315 / 764.072 | 9.824 / 18.200 |
| rust | warm-revisit | 4299.113 / 4231.608 | 454.954 / 3737.321 / 37.726 | 575.785 / 406.069 | 526.710 / 291.294 | 812.737 / 773.398 | 7.502 / 18.278 |
| rust-pre-coherence | fresh | 4295.930 / 4229.683 | 452.731 / 3739.934 / 35.284 | 512.828 / 395.350 | 524.128 / 305.463 | 789.868 / 768.145 | 9.566 / 17.758 |
| rust-pre-coherence | warm-revisit | 4297.724 / 4230.481 | 452.949 / 3738.240 / 37.674 | 535.054 / 378.340 | 525.644 / 288.548 | 861.078 / 825.210 | 7.282 / 17.692 |

Filesystem/client quota: 2 CPU / 4 GiB; native backend: separate 4 CPU /
6 GiB cgroup, same Pod/network namespace. Per-phase cgroup peaks here are
sampled, not the cumulative lifetime peak. PSS has a one-second target
sampling interval; RSS/cgroup memory targets 50 ms. Raw timestamps are retained.
Go allocator/heap peak and steady time series are missing, not zero.
Rust allocated-byte samples use jemalloc; they are not RSS. Near-limit file
cache means lower daemon RSS alone is not a whole-cgroup memory win.

## Latency and metadata cost

| Implementation | State | Read syscall p50 / p95 / p99 ms | File p99 ms | Listing total s | Explicit metadata roundtrip s | Whole workload wall s |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| go-experimental | fresh | 0.250 / 1.463 / 10.115 | 307.302 | 0.430 | 1.188 | 14.908 |
| go-experimental | warm-revisit | 0.251 / 1.437 / 10.101 | 303.617 | 0.033 | 0.627 | 13.980 |
| go-packaged | fresh | 0.260 / 1.497 / 10.211 | 304.291 | 0.438 | 1.183 | 15.122 |
| go-packaged | warm-revisit | 0.264 / 1.479 / 10.308 | 308.329 | 0.033 | 0.676 | 14.370 |
| rust | fresh | 0.079 / 1.336 / 18.032 | 351.583 | 1.463 | 0.597 | 16.702 |
| rust | warm-revisit | 0.084 / 1.365 / 18.242 | 389.483 | 0.020 | 0.562 | 16.233 |
| rust-pre-coherence | fresh | 0.078 / 1.306 / 17.448 | 348.686 | 1.389 | 0.610 | 16.020 |
| rust-pre-coherence | warm-revisit | 0.081 / 1.341 / 17.507 | 361.653 | 0.021 | 0.546 | 15.762 |

These are means of per-phase nearest-rank percentiles, not pooled population
percentiles. All errors and repetitions remain in the private raw evidence.

## Diagnosis and remaining gates

Rust retains substantially lower process RSS but loses payload throughput,
cold listing time, p99 and whole-workload CPU on this equivalent diagnostic.
The pre-coherence Rust executable also loses to Go; the metadata repair does
not establish a performance win or attribute the six historical EAGAIN cases.

Separate instrumented owned-daemon runs passed byte/metadata checks and
retained perf CPU profiles and syscall summaries. Those instrumented wall
times are excluded from comparison samples. The Rust profile includes
anonymous page faults, page clearing, jemalloc discard and cached-reply copies.
The syscall summaries observed 852 madvise calls for Rust versus 27 for Go;
syscall elapsed time under strace is not CPU consumed. This motivates the
separately retained unchanged-binary allocator A/B diagnostic, not a claim
that the allocator explains every regression.

Backend request metrics were unavailable: the configured endpoint refused
connections. The native fixture does not emulate actual-service extended
metadata or establish writer/fsync, random/HTTP-client, optional S3 modes,
complete CLI or full-matrix acceptance. Go heap series and broad quality
gates remain open. No production resource or object was modified.

## Unchanged-binary allocator A/B

A separate alternating three-repeat diagnostic changes only the Rust
jemalloc startup configuration to `oversize_threshold:0,dirty_decay_ms:1000,muzzy_decay_ms:0`.
The allocator confirmed the values in its startup log. Default and candidate
use the same executable, cache budgets, data, metadata work and complete byte
checks. All 36,000 additional file checks passed. These are diagnostic
settings, not a delivered default change or an accepted Go/Rust win.

| Allocator | State | Read MB/s | RSS peak / steady MB | Whole cgroup CPU s | Daemon system CPU s | Read p99 ms |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| rust-allocator-default | fresh | 578.872 | 535.216 / 355.610 | 30.695 | 10.593 | 17.383 |
| rust-allocator-default | warm-revisit | 544.254 | 667.009 / 372.053 | 31.492 | 10.547 | 17.580 |
| rust-allocator-candidate | fresh | 596.420 | 579.002 / 479.661 | 29.988 | 9.217 | 16.945 |
| rust-allocator-candidate | warm-revisit | 550.405 | 665.864 / 484.699 | 31.433 | 9.883 | 17.081 |

Reducing oversize discard lowers measured system CPU and improves fresh
throughput modestly, while raising fresh peak RSS. The warm CPU and tail
losses remain. The allocator alone therefore does not resolve the measured
deficit; changing its default is not justified as an overall acceptance fix.
