# Integrated metadata/handle checkpoint — 2026-11-01

**UNACCEPTED.** The integrated freshness, READDIRPLUS, standard-LIST resume,
per-open history and clean-fsync corrections have been measured together. They
improve selected metadata tails and retain substantial sequential/eviction gains
against Go, but metadata/writer throughput and concurrent/equal-rate memory still
lose. This checkpoint does not establish the agreed all-workload improvement.

## Identity and measurement boundary

Candidate release SHA256 is
`4224600c17c481793621e27dbffb8bcc4d9e02c8aa18751df20d2e0417886d6c`,
Rust 1.99.1 / default jemalloc / ThinLTO. It was frozen from the integrated tree
after `f9aea43`; its manifest records every compiled and benchmark input hash,
the dirty starting checkout and successful build/verification receipts. Source
hashes are rechecked before and after controls. Documentation delivery is not a
new build. The unchanged Rust reference is `67f7aec`; both Go revisions, Go 1.25.9
and all three reference hashes are the [previous VM identities](performance.md#native-vm-admissiondirectory-checkpoint--2026-10-31).

The authorized VM runs Linux 7.0.0-38-generic, 64 AMD EPYC Milan vCPUs, about
125 GiB RAM, real FUSE and SeaweedFS 4.51. Every implementation uses eight runtime
workers, 64 MiB payload ownership, 100,000 entries, the same 256 MiB position-checked
payload, 1,024 metadata entries, 512 seeded 4 KiB random reads and four concurrent
readers. Disposable backend groups retain identical payload generations across
the four binaries and verify full bytes before cleanup. Go GC defaults are
unchanged; no benchmark-triggered GC, allocator purge or global cache drop occurs.

Builds, coverage, audits and profiling finish before these serial timings.
The VM is shared; exclusive-host isolation and zero observer overhead are not
established. Raw observations, source snapshots, incomplete attempts and reducers
are retained under ignored `benchmark-results/private/vm-20261031-integrated/`.
Private mixed orchestration is not presented as the public
[`bench/compare.py` reproduction command](performance.md#reproduction).

## Five-repeat isolated healthy workloads

There are 240 independently verified records: four binaries, cold/warm, six
healthy workloads and five repetitions, with 60 final generation/full-byte checks.
The filesystem/client quota is eight CPUs (`800000 100000`); backend and parent
have no CPU quota. Memory maxima are 16/12/32 GiB for filesystem/client, backend
and parent. Writer/fsync writes **16 MiB**. Warm records have a matching checked
warmup; implementation order rotates. Cold measurements remain separately retained.

The cold controller fails while starting the next backend after completing all
120 cold rows: its released free-port reservations selected the same filer/S3
gRPC port. The separate warm controller completes all 120 rows, then fails in
final bookkeeping after a local namespace shadows CLI arguments. Both original
controllers remain failed. Independent reduction checks the exact 240-key grid,
all 60 generations, operations, bytes, source identities and unchanged limits;
it does not relabel those failures as a clean full-matrix execution. The separate
zero-measurement warm setup failure is also retained. `writer_failure` is excluded
from this healthy comparison; the original Go 75-second EAGAIN failures remain
required [compatibility evidence](compatibility.md).

Throughput is total checked work / summed measured time, in MiB/s except metadata
operations/s. Latencies below pool all five measured runs, not their percentiles.
Memory and CPU are arithmetic means of available per-run metrics; `n` identifies
incomplete series. CPU is consumed user+system seconds, not elapsed time. MiB = 2^20
bytes. Sampled peaks and temporal latter-half means do not establish a plateau.

| Workload | Implementation | Throughput | Pooled p50 / p95 / p99 ms |
| --- | --- | ---: | --- |
| metadata | Candidate | 115473.44 ops/s | 0.00480 / 0.00841 / 0.02471 |
| metadata | Rust `67f7aec` | 153956.16 ops/s | 0.00513 / 0.00667 / 0.01900 |
| metadata | Go experimental | 94956.44 ops/s | 0.00496 / 0.00822 / 0.03123 |
| metadata | Go packaged | 117919.31 ops/s | 0.00477 / 0.00633 / 0.02096 |
| sequential | Candidate | 519.44 | 0.74455 / 1.94943 / 8.06315 |
| sequential | Rust `67f7aec` | 531.27 | 0.70411 / 1.86721 / 7.16181 |
| sequential | Go experimental | 218.23 | 2.50944 / 10.86194 / 21.72053 |
| sequential | Go packaged | 218.78 | 2.24927 / 11.29889 / 23.48452 |
| random | Candidate | 13.06 | 0.10274 / 0.14471 / 0.17612 |
| random | Rust `67f7aec` | 14.31 | 0.08013 / 0.11246 / 0.15429 |
| random | Go experimental | 13.22 | 0.09564 / 0.15541 / 0.20807 |
| random | Go packaged | 12.33 | 0.10551 / 0.15231 / 0.19263 |
| concurrent | Candidate | 9.24 | 1.58873 / 2.16917 / 2.59002 |
| concurrent | Rust `67f7aec` | 9.40 | 1.57794 / 2.11100 / 2.47460 |
| concurrent | Go experimental | 9.24 | 1.57404 / 2.16931 / 2.68460 |
| concurrent | Go packaged | 9.31 | 1.54953 / 2.17422 / 2.81081 |
| eviction | Candidate | 528.96 | 0.73121 / 1.50946 / 7.21266 |
| eviction | Rust `67f7aec` | 546.51 | 0.68677 / 1.93387 / 6.93981 |
| eviction | Go experimental | 222.36 | 2.47180 / 10.64502 / 23.06397 |
| eviction | Go packaged | 204.62 | 2.77954 / 11.47176 / 23.14621 |
| writer/fsync | Candidate | 17.33 | 49.21033 / 80.02436 / 93.08123 |
| writer/fsync | Rust `67f7aec` | 17.07 | 48.26510 / 90.39024 / 103.00418 |
| writer/fsync | Go experimental | 12.84 | 67.77703 / 107.13523 / 123.31611 |
| writer/fsync | Go packaged | 13.28 | 67.68869 / 107.61295 / 118.14476 |

| Workload | Implementation | Daemon RSS peak / late MiB | Daemon+client RSS peak / late MiB | Cgroup peak / late MiB | CPU daemon / client / cgroup s |
| --- | --- | --- | --- | --- | --- |
| metadata | Candidate | 28.42 / missing (n=0) | 82.24 / missing (n=0) | 59.36 / missing (n=0) | 0.00 / 0.01 / 0.01 |
| metadata | Rust `67f7aec` | 30.63 / 31.27 (n=3) | 84.43 / 85.04 (n=3) | 61.26 / 61.79 (n=3) | 0.00 / 0.00 / 0.01 |
| metadata | Go experimental | 46.42 / 46.63 (n=1) | 100.26 / 100.45 (n=1) | 59.06 / 59.20 (n=1) | 0.01 / 0.01 / 0.02 |
| metadata | Go packaged | 46.81 / 47.09 (n=1) | 100.62 / 100.99 (n=1) | 60.53 / 60.02 (n=1) | 0.00 / 0.00 / 0.01 |
| sequential | Candidate | 129.45 / 127.30 | 183.96 / 181.80 | 415.37 / 339.34 | 0.46 / 0.39 / 0.85 |
| sequential | Rust `67f7aec` | 131.83 / 128.11 | 186.31 / 182.60 | 418.73 / 344.64 | 0.45 / 0.39 / 0.84 |
| sequential | Go experimental | 285.83 / 183.63 | 340.38 / 238.18 | 434.68 / 370.79 | 4.93 / 0.42 / 5.35 |
| sequential | Go packaged | 282.35 / 188.95 | 336.87 / 243.47 | 439.23 / 377.67 | 4.80 / 0.41 / 5.21 |
| random | Candidate | 85.84 / 85.82 | 141.04 / 141.03 | 119.77 / 118.57 | 0.04 / 0.11 / 0.15 |
| random | Rust `67f7aec` | 86.68 / 86.67 | 141.82 / 141.82 | 120.76 / 119.41 | 0.03 / 0.11 / 0.14 |
| random | Go experimental | 109.76 / 109.74 | 164.94 / 164.92 | 127.28 / 126.02 | 0.04 / 0.10 / 0.15 |
| random | Go packaged | 109.28 / 109.28 | 164.40 / 164.40 | 127.38 / 126.40 | 0.05 / 0.12 / 0.17 |
| concurrent | Candidate | 133.44 / 131.21 | 190.06 / 187.81 | 184.10 / 177.98 | 0.77 / 0.65 / 1.42 |
| concurrent | Rust `67f7aec` | 130.27 / 127.81 | 186.81 / 184.34 | 181.14 / 174.13 | 0.78 / 0.63 / 1.40 |
| concurrent | Go experimental | 130.02 / 123.46 | 186.51 / 180.00 | 158.63 / 153.30 | 1.24 / 0.62 / 1.85 |
| concurrent | Go packaged | 130.62 / 125.73 | 187.11 / 182.25 | 159.69 / 154.98 | 1.26 / 0.62 / 1.89 |
| eviction | Candidate | 136.88 / 136.17 | 191.40 / 190.69 | 425.82 / 346.99 | 0.45 / 0.38 / 0.84 |
| eviction | Rust `67f7aec` | 129.42 / 127.86 | 183.95 / 182.40 | 417.81 / 339.63 | 0.42 / 0.39 / 0.80 |
| eviction | Go experimental | 280.06 / 181.49 | 334.65 / 236.07 | 433.89 / 372.50 | 4.69 / 0.42 / 5.10 |
| eviction | Go packaged | 288.64 / 185.58 | 343.16 / 240.09 | 449.74 / 373.04 | 5.42 / 0.43 / 5.85 |
| writer/fsync | Candidate | 42.19 / 41.12 | 101.37 / 100.30 | 94.42 / 89.20 | 0.19 / 0.11 / 0.30 |
| writer/fsync | Rust `67f7aec` | 42.29 / 41.67 | 101.50 / 100.87 | 95.51 / 89.52 | 0.18 / 0.13 / 0.31 |
| writer/fsync | Go experimental | 62.38 / 58.07 | 121.55 / 117.24 | 97.28 / 88.59 | 0.41 / 0.12 / 0.53 |
| writer/fsync | Go packaged | 62.34 / 57.96 | 121.46 / 117.08 | 97.36 / 88.73 | 0.40 / 0.12 / 0.52 |

Candidate allocated peak/late MiB is 77.27/58.10 sequential, 77.34/77.19 random,
81.02/79.83 concurrent, 78.51/60.73 eviction and 22.09/20.90 writer. Its corresponding
late file/kernel MiB is 179.85/1.50, 0.11/1.30, 11.79/1.72, 178.43/1.52 and
12.57/0.94. Go late file MiB in the same order is 164.36/0.30/11.64/166.94/12.62
experimental and 164.07/0.44/11.68/161.45/12.49 packaged; these file components
retain nonwins despite favorable total memory in several workloads.

All isolated Go heap series lack the two fully contained observations required
by policy. Candidate metadata also lacks in-window allocator/late memory samples;
eight short Rust metadata windows lack in-window budget receipts. Original daemon
observations verify the configured static budgets, but are not substituted into
those missing measured windows. These aggregates cannot pass complete accounting.
Cgroup peaks include startup/warmup; RSS and late memory use the measured window.
Rounded CPU 0.00 is not a claim that operations consume no CPU.

## Persistent mixed and observer controls

Each cycle runs metadata, sequential, random, concurrent, eviction and **1 MiB**
writer/fsync, on one persistent mount. Filesystem/client, backend and parent have
no CPU quota and retain 16/12/32 GiB maxima. Post-initial mixed cycles are not
isolated healthy-warm workloads. Two unpaced 30-cycle repetitions complete 1,440
checked phases and eight final generation/full-byte checks:

| Implementation | Elapsed s, repeats 0 / 1 | Whole-window cgroup CPU s, repeats 0 / 1 | Daemon RSS peak / late MiB, repeats 0 ; 1 | Cgroup peak / late MiB, repeats 0 ; 1 |
| --- | --- | --- | --- | --- |
| Candidate | 89.32 / 91.24 | 108.91 / 110.30 | 178.88 / 154.25 ; 184.90 / 160.28 | 477.32 / 250.96 ; 476.75 / 256.85 |
| Rust `67f7aec` | 88.29 / 84.74 | 106.45 / 103.43 | 180.26 / 153.77 ; 174.53 / 157.67 | 469.50 / 251.61 ; 469.69 / 255.24 |
| Go experimental | 132.04 / 134.34 | 409.85 / 414.38 | 316.04 / 175.16 ; 312.26 / 173.18 | 523.34 / 274.53 ; 547.45 / 272.74 |
| Go packaged | 177.66 / 175.02 | 743.65 / 733.28 | 335.63 / 202.46 ; 336.39 / 205.06 | 718.40 / 336.52 ; 774.46 / 344.92 |

Initial order is candidate, packaged Go, prior Rust, experimental Go; the second
repetition rotates that order. Candidate metadata is 60,181/52,868 ops/s with
pooled p99 0.02058/0.02137 ms, improving the prior Rust 49,516/47,258 and
0.04439/0.05518 ms, but losing both Go throughputs. Candidate writer throughput is
24.70/23.63 MiB/s versus Go experimental 30.51/29.68 and packaged 27.99/28.19;
random/concurrent and prior-Rust-relative losses remain in the retained tables.

The paced control runs 30 cycles, one per 10 seconds, in order experimental Go,
prior Rust, packaged Go, candidate. All four complete 300.00 s with zero late
cycles; 720 phases and four generation/full-byte checks pass. CPU below spans
the whole 300-second window, including inter-cycle and final pacing sleep.
Memory late means select the temporal latter half of that complete window;
sampled RSS excludes mount startup, while cgroup peak includes mount lifetime.

| Implementation | CPU daemon / client / cgroup s | Daemon RSS peak / late MiB | Client RSS peak / late MiB | Daemon+client RSS peak / late MiB | Cgroup peak / late MiB |
| --- | --- | --- | --- | --- | --- |
| Candidate | 64.07 / 49.01 / 113.08 | 175.39 / 153.13 | 62.86 / 62.77 | 238.09 / 215.90 | 482.79 / 395.21 |
| Rust `67f7aec` | 65.22 / 48.95 / 114.16 | 171.95 / 152.36 | 62.87 / 62.83 | 234.75 / 215.19 | 479.32 / 395.63 |
| Go experimental | 384.30 / 50.66 / 434.96 | 315.33 / 155.92 | 62.93 / 62.85 | 378.24 / 218.77 | 526.88 / 357.86 |
| Go packaged | 579.87 / 52.32 / 632.18 | 333.39 / 174.92 | 62.82 / 62.77 | 396.09 / 237.69 | 658.68 / 370.08 |

| Implementation | Allocator peak / late MiB | File / kernel late MiB | Metadata ops/s; pooled p50 / p95 / p99 ms | Writer MiB/s; pooled p50 / p95 / p99 ms |
| --- | --- | --- | --- | --- |
| Candidate | 87.69 / 76.01 | 198.35 / 3.39 | 23110.03; 0.00446 / 0.00736 / 0.02417 | 23.15; 20.52554 / 28.41622 / 28.66538 |
| Rust `67f7aec` | 86.66 / 75.49 | 198.31 / 3.34 | 14124.50; 0.00580 / 0.05197 / 0.06678 | 24.20; 21.19856 / 29.01317 / 34.96007 |
| Go experimental | 246.90 / 99.60 | 171.33 / 3.77 | 36403.99; 0.00486 / 0.00859 / 0.02998 | 32.74; 19.60541 / 26.10962 / 26.45983 |
| Go packaged | 273.19 / 115.46 | 155.68 / 3.82 | 42813.95; 0.00444 / 0.00730 / 0.02353 | 29.49; 19.69171 / 23.91103 / 24.77399 |

Candidate late jemalloc active/resident/mapped/retained-virtual/metadata gauges are
81.19/169.88/194.27/136.24/15.88 MiB. Allocated size classes, allocator page gauges,
virtual retention, RSS and cgroup charges have different scopes and are not
additive. This attributes retained owners without proving a leak or a plateau.

A separate observer-off 30-cycle/300-second control completes all 720 phases and
four checks, in order candidate, packaged Go, prior Rust, experimental Go. Its
memory gauges are **unobserved**, not zero. Candidate metadata p99 is 0.02746 ms
versus prior Rust 0.89183 and Go 0.02815/0.02976; throughput remains 19,993 ops/s
versus Go 37,531/36,361. Candidate writer is 22.84 MiB/s versus Go 32.78/29.94.
One on/off control and changed order do not establish zero observer overhead.

All 2,880 mixed phases and 16 final checks pass independent reduction. Recorded
filesystem/client and backend interval deltas for throttling, high/max/OOM and
scan/steal counters are zero. This is not a claim of zero CPU/I/O waiting.
Owned mounts, clients, daemons and backend processes stop successfully. The
isolated-controller errors, missing observations, per-run variance and adverse
metrics remain retained. Full compatibility, strict comparative accounting and
[whole affected-file quality](testing.md) remain open.
