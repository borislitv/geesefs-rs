# System allocator with explicit request accounting

The [allocator screen](system-allocator-screen.md) justifies a full comparison,
not a default change. Build the candidate explicitly:

```bash
cargo build --release --locked --features system-allocator
```

Ordinary builds retain jemalloc. The optional, public MIT-licensed stats_alloc
0.1.10 crate wraps `std::alloc::System` using its stable `INSTRUMENTED_SYSTEM` API.
The wrapper delegates alloc, alloc_zeroed, realloc and dealloc to the platform;
no application allocator, pool, unsafe block or malloc tuning is introduced.
Its source release is
[`be31f749696faab326bf18b5dd75eaf93010693d`](https://github.com/neoeinstein/stats_alloc/tree/be31f749696faab326bf18b5dd75eaf93010693d).
The release is from 2022, not a newly released implementation.

## Metric contract

Existing `--stats-file` samples identify the allocator and metric scope. For this
candidate, `allocated_bytes` is a sampled difference between requested allocation
and deallocation bytes, with all six raw wrapper counters retained. It is **not
exact live heap, allocator-reserved memory, process RSS or cgroup memory**:

- Counters are loaded separately and updated before platform allocation calls.
- Failed allocation/reallocation requests are included. The request balance may
  remain inflated after a handled allocation failure.
- Concurrently inconsistent negative differences become null rather than zero.
- Native allocations bypassing Rust's global allocator, allocator overhead and
  retained/free pages are outside the request balance.
- The wrapper's atomic operations are included in every candidate measurement,
  including when `--stats-file` is not supplied.

Jemalloc samples retain their size-class allocation statistic and identify that
different scope. Neither statistic substitutes for independent RSS, cgroup,
file/kernel memory and backend measurements. Do not equate these two allocator
columns or count missing Go heap data as zero.

## Verification and exact-source experiment

Kubernetes package-clean checks pass: 185 Rust cases, formatting, warning-denying
all-target/all-feature Clippy, default-feature compilation and the 338-package
public dependency audit. The existing filesystem/S3/FIFO/cancellation tests are
unchanged; they do not themselves establish the CLI allocator's mounted behavior.

The [verified native release](system-allocator-accounting-quality-20261028.json)
has SHA256 `98280f9792419af709e372950ec089ebf7ca56bb59647aa8a35e104e58477399`.
All 25 compiled inputs and their exact change set were independently reconciled.
Crapper reports run CC 30 without coverage/CRAP; three prior duplicate candidates
remain, and five main.rs mutation sites were scanned but not executed.

The exact candidate source is `b03783fcc09160e7b97f5ea816f43f89a4953df3`.
Forty phases, repetitions 100–104, complete in 699.290 s against frozen receive
jemalloc and both Go references. All 120,000 file checks, native hashes before and
after, 97 listings, 280 CPU windows and 460 artifact hashes pass. The sealed
dataset, kernel, 2 CPU/4 GiB filesystem/client quota, separate 4 CPU/6 GiB backend,
512 MiB payload, 100000 entries and four 128 KiB readers remain unchanged.
Fresh means a new mount, not controlled cold host/backend caches.

The [numeric receipt](system-allocator-accounting-results-20261028.json) retains
all selected memory/CPU scopes, dispersion and paired directions. Five-phase
means below use decimal MB; RSS is peak / final-fifth mean, not a plateau.

| Implementation/state | Read MB/s | Daemon RSS peak / final-fifth MB | Whole cgroup CPU s | Read p99 ms | Listing s |
| --- | ---: | ---: | ---: | ---: | ---: |
| Go experimental fresh | 598.813 | 1329.941 / 1203.627 | 30.628 | 10.590 | 0.484 |
| Go experimental warm-revisit | 587.619 | 1273.301 / 1189.366 | 29.836 | 10.517 | 0.031 |
| Go packaged fresh | 566.540 | 1253.992 / 1200.881 | 32.289 | 11.244 | 0.488 |
| Go packaged warm-revisit | 558.183 | 1308.093 / 1208.292 | 31.790 | 11.429 | 0.028 |
| Rust System/tracked fresh | 582.419 | 573.494 / 560.084 | 31.286 | 11.260 | 1.688 |
| Rust System/tracked warm-revisit | 565.831 | 592.055 / 580.178 | 30.839 | 11.983 | 0.027 |
| Rust jemalloc prior fresh | 567.848 | 552.663 / 474.324 | 31.835 | 11.862 | 1.502 |
| Rust jemalloc prior warm-revisit | 531.860 | 557.556 / 482.783 | 32.613 | 12.359 | 0.024 |

Against jemalloc, mean throughput improves 2.57% fresh / 6.39% warm, whole CPU
1.72% / 5.44%, and p99 5.08% / 3.04%. But favorable throughput pairs are only
2/5 and 3/5, CPU 3/5 and 3/5, p99 4/5 and 2/5. Peak RSS increases 3.77% / 6.19%
and final-fifth RSS **18.08% / 20.17%**, with no favorable final-fifth pair.
Candidate throughput population stdev is 60.896 / 65.759 MB/s, p99 stdev
1.553 / 1.962 ms. Retain this substantial dispersion; one mean is not a guarantee.

Process RSS remains substantially below Go, but experimental Go wins mean
throughput/whole CPU and both Go references win p99. Whole-cgroup memory remains
near the 4 GiB cap with different anonymous/file-cache composition; lower daemon
RSS is not a demonstrated equivalent reduction in total cgroup memory. No default
or overall acceptance decision follows from this comparison.

Actual CLI output passes 3304 sample checks across five mounts: allocator identity,
all six counters, scope, unchanged budgets and algebraic consistency. The old
uninstrumented System output fails the expected missing-counter assertion. This
checks the reporting contract, not an independent exact-heap oracle. Sampled net
requested-byte peak/final-fifth means are 515.469/282.468 MB fresh and
521.234/278.704 MB warm; their scope differs from jemalloc's allocation statistic.

## Absolute page-cost confirmation and next cause

[Nine separate profiles](system-allocator-accounting-profiles-20261028.json) pass
another 27,000 file checks and native hashes/listings. Raw periods, process CPU,
stacks and all 27 profile artifact hashes reconcile independently. Relative to
the same jemalloc source, System reduces minor faults 721,961 → 144,945 and system
CPU 9.100 → 7.483 s; user CPU rises 3.980 → 4.303 s. Selected fault-leaf time falls
0.687 → 0.141 sampled CPU s, including admitted-fragment faults 0.508 → 0.092 s.
Profile RSS peak/final-fifth instead rises 552.184/483.129 → 562.020/557.589 MB.

These are instrumented fresh mounts, not timing acceptance repeats. Capture is
95.7–96.7% for System, 96.3–96.9% for jemalloc and 92.9–93.7% for Go. The System
profiles beat Go on daemon CPU, but cannot replace the adverse matched comparison.
Unresolved libc leaves total 1.588 sampled CPU s for System versus 1.045 s for
jemalloc. Allocation shim leaves and inlining do not isolate counter overhead.
An open upstream [stats_alloc discussion](https://github.com/neoeinstein/stats_alloc/pull/9)
proposes relaxed counter ordering, but no published API provides that change.

The [same-source counter ablation](system-allocator-counter-ablation-20261028.json)
then completes twelve rotated phases, repetitions 110–112, in 225.620 s. All
36,000 file checks, native hashes/listings, 84 CPU windows and 147 artifact hashes
pass. Only the main.rs allocator wrapper/reporting branch differs from b03783f;
the exact patch and binary are retained, original source restored and package
cleaned. Bare System heap values are missing, not zero. Three-phase means:

| Counter/state | Read MB/s | Daemon RSS peak / final-fifth MB | Whole cgroup CPU s | Read p99 ms |
| --- | ---: | ---: | ---: | ---: |
| Tracked fresh | 568.845 | 584.969 / 577.099 | 31.891 | 11.752 |
| Bare fresh | 587.721 | 565.592 / 552.142 | 31.212 | 11.076 |
| Tracked warm-revisit | 557.070 | 592.953 / 575.534 | 31.242 | 11.975 |
| Bare warm-revisit | 554.453 | 589.293 / 575.967 | 31.503 | 11.713 |

Removing the wrapper improves fresh mean throughput 3.32%, CPU 2.13% and p99
5.75%, but warm throughput falls 0.47% and CPU rises 0.84%; warm final-fifth RSS
also rises 0.08%. This rejects a general counter-only explanation of the remaining
deficit. Do not remove reporting or choose a default to hide the unfavorable
repeat. Next resolve the remaining libc symbols and their callers from the
already-recorded profiles before changing another component.

Standard-LIST cold type
resolution remains constrained by fuser 0.18.0's concrete FileType API; do not
label unknown objects as regular files to remove required HEAD requests.
Overall compatibility/performance acceptance remains open.
