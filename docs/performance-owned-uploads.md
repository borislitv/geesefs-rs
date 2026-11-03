# Ownership and upload checkpoint, 2026-11-01

**UNACCEPTED.** This checkpoint measures `d09eafc` after closed-file kernel-cache
retirement, backend-aligned directory batches and owned contiguous upload bodies.
The earlier [integrated measurements](performance-integrated.md) keep their
original binaries, resources, controller failures and missing accounting scopes.

## Design and provenance

- Rust candidate `d09eafc2f390b235563df68712556dcb51170764`, release SHA256
  `27df85c17ab173a8bde307f21e912a1feef7938ceec7bf5fbc863cae670b4e5a`.
  Rust 1.99.1, default jemalloc, ThinLTO; 85 frozen inputs match committed blobs.
- Prior Rust `67f7aec0dc78be355c431c8c72b73bd9a248378f`, SHA256
  `666de62cc08a7bf98866c1a6adf3569a90f812049da51e85b66be6b1ca7bdb49`.
- Experimental Go `5c0884c9e8799bbab93e2d4102fdc15e9c079e0d`, SHA256
  `a4b7a8c3e949b3dac86b719a740b2e9bf664474d65cefba67052c3c724a5e9dc`.
- Packaged Go `80c214ef14f620d93407454fc76ca01dd71fa25f`, SHA256
  `c0c3c0aec1d435495adef4bfefff04eefae644dab9de028ccfce09e71e8a9815`.
  Both Go binaries use Go 1.25.9 and unchanged default GC policies.
- Linux 7.0.0-38-generic, 64 EPYC Milan vCPUs; SeaweedFS 4.51. Identical 64 MiB
  payload/100,000-entry limits, eight workers, 256 MiB checked payload, 1,024 names,
  512x4 KiB random reads, concurrency4 and 1 MiB writer. Dedicated owned groups
  have unbounded CPU and memory16/12/32 GiB for filesystem+client/backend/parent.
- One fixed-order design: experimental Go, prior Rust, packaged Go, candidate.
  Each mount runs 30 cycles at one per 10 seconds, in order metadata, sequential,
  random, concurrent, eviction, writer/fsync. All four complete in300s without
  lateness:720 checked phases and four final full-byte/generation checks.
  Every writer also checks backend bytes before unlink. All process exits and
  cleanup are verified; interval throttle/reclaim/high/max/OOM deltas are zero.
- No local build, quality, audit or profile work overlaps these timings.
  Shared-host scheduling, observers and fixed ordering remain qualifications.
  Raw receipts, immutable snapshots, profiles and journals remain private/ignored.

## Throughput and latency

Metadata rates are ops/s; other rates are MiB/s. Initial means mixed cycle0,
not an isolated cold workload. Later rates divide total work by total active
phase time in cycles1-29; latency percentiles pool their complete raw calls.
Those cycles are mixed operation, not isolated warm workloads or repeated designs.

| Workload | Binary | Initial rate | Initial p99 ms | Later rate | Later p50 ms | Later p95 ms | Later pooled p99 ms |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| metadata | Rust d09eafc | 4288.83 | 0.01827 | 27119.27 | 0.00709 | 0.00828 | 0.04164 |
| metadata | Rust 67f7aec | 1902.86 | 0.09203 | 9025.26 | 0.00732 | 0.06045 | 0.92262 |
| metadata | Go experimental | 3875.06 | 0.03477 | 37422.29 | 0.00451 | 0.00774 | 0.02884 |
| metadata | Go packaged | 5034.82 | 0.02716 | 36150.60 | 0.00642 | 0.00758 | 0.03017 |
| sequential | Rust d09eafc | 588.23 | 5.32442 | 534.45 | 0.78499 | 1.88559 | 8.36006 |
| sequential | Rust 67f7aec | 573.98 | 5.24303 | 487.57 | 0.79908 | 2.05246 | 8.54757 |
| sequential | Go experimental | 193.86 | 25.97120 | 198.07 | 2.90589 | 11.67557 | 21.83957 |
| sequential | Go packaged | 243.33 | 17.99371 | 139.55 | 4.66428 | 17.64239 | 27.36911 |
| random | Rust d09eafc | 1.74 | 3.22693 | 1.76 | 2.05541 | 3.12687 | 3.49538 |
| random | Rust 67f7aec | 1.92 | 3.49301 | 1.87 | 1.95328 | 2.99394 | 3.34612 |
| random | Go experimental | 1.52 | 3.83273 | 1.84 | 1.95425 | 3.14299 | 3.51995 |
| random | Go packaged | 1.89 | 3.10772 | 1.90 | 1.90550 | 2.95285 | 3.55845 |
| concurrent | Rust d09eafc | 8.55 | 3.34419 | 8.84 | 1.63136 | 2.50766 | 3.12000 |
| concurrent | Rust 67f7aec | 8.54 | 3.53279 | 9.19 | 1.61148 | 2.27741 | 2.79221 |
| concurrent | Go experimental | 8.58 | 3.52614 | 8.93 | 1.62034 | 2.38863 | 2.98268 |
| concurrent | Go packaged | 8.92 | 2.88162 | 8.80 | 1.61732 | 2.48410 | 3.20431 |
| eviction | Rust d09eafc | 598.94 | 4.30238 | 578.19 | 0.76694 | 2.69009 | 4.63598 |
| eviction | Rust 67f7aec | 569.66 | 5.13965 | 587.20 | 0.73501 | 2.56162 | 4.72581 |
| eviction | Go experimental | 185.19 | 26.25251 | 206.37 | 2.82775 | 11.44576 | 24.54694 |
| eviction | Go packaged | 168.69 | 28.51720 | 151.92 | 4.24477 | 16.64872 | 26.48308 |
| writer_fsync | Rust d09eafc | 23.26 | 20.20975 | 22.04 | 24.58392 | 30.21102 | 33.65312 |
| writer_fsync | Rust 67f7aec | 20.41 | 25.41937 | 24.05 | 20.31940 | 30.27083 | 33.02332 |
| writer_fsync | Go experimental | 21.57 | 31.17307 | 31.87 | 20.21091 | 24.82518 | 26.92267 |
| writer_fsync | Go packaged | 20.42 | 28.27753 | 28.55 | 19.98662 | 27.30191 | 27.31794 |

## Memory and CPU scopes

RSS peaks are sampled in the controlled window. Combined RSS uses pointwise
daemon+client sums, not sums of independent peaks. Cgroup peaks cover the owned
mount lifetime, including startup. Late values are sample means in the temporal
latter half of300s, including offered-rate idle time: **not memory plateaus**.
Go live heap and Rust jemalloc allocated bytes have different providers; neither
equals RSS. Cgroup file/kernel and allocator gauges are not additive to RSS.

| Binary | Daemon RSS peak/late MiB | Client RSS peak/late MiB | Combined RSS peak/late MiB | Cgroup peak/late MiB | File late MiB | Kernel late MiB | Live allocator peak/late MiB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Rust d09eafc | 176.95/157.38 | 62.94/62.83 | 239.78/220.21 | 479.51/221.96 | 19.90 | 3.14 | 88.98/78.16 |
| Rust 67f7aec | 179.67/154.56 | 62.93/62.87 | 242.41/217.43 | 481.25/397.39 | 197.79 | 3.31 | 85.55/74.88 |
| Go experimental | 322.63/157.29 | 62.89/62.84 | 385.46/220.13 | 575.38/359.96 | 172.13 | 3.88 | 257.37/102.55 |
| Go packaged | 335.34/176.07 | 62.94/62.86 | 397.86/238.93 | 763.76/371.42 | 153.18 | 3.85 | 260.08/114.49 |

Candidate late jemalloc page gauges are active83.53, resident175.05, mapped201.34,
retained virtual131.94 and metadata16.01 MiB. These categories overlap and are
not physical-memory sums. Go forced-GC deltas216/257 are observed runtime activity;
the observer never requests forced GC. Neither allocator is tuned or purged.

CPU is consumed time over the whole300s window, separate from elapsed time.
One normalized cycle includes all six unchanged workload phases and observers.

| Binary | Daemon user/system CPU s | Client user/system CPU s | Cgroup CPU s | Cgroup CPU s per cycle |
| --- | ---: | ---: | ---: | ---: |
| Rust d09eafc | 29.75/41.12 | 34.16/14.45 | 119.49 | 3.9830 |
| Rust 67f7aec | 29.71/35.68 | 32.53/16.11 | 114.04 | 3.8012 |
| Go experimental | 247.28/145.62 | 34.08/17.42 | 444.41 | 14.8135 |
| Go packaged | 447.10/159.39 | 33.99/18.32 | 658.81 | 21.9603 |

## Conclusions and remaining work

Sequential/eviction and Go-relative CPU/cgroup results are favorable. Metadata,
random, writer and some concurrent/tail metrics still lose. Candidate late daemon
and combined RSS are essentially equal to experimental Go, not robust wins;
several prior-Rust-relative CPU/memory/throughput metrics also lose. One changed
shared-host run cannot isolate the upload change from scheduling variation.

The separate native wire regression proves removal of redundant materialization
and aws-chunked encoding for covered dirty bytes, including retry/charge/activity
lifetimes. The source-only attribution below is favorable, but broader isolated/
offered-rate/observer controls and the full formal joint gate remain open.
Whole-file CRAP, surviving mutation sites, positive LINK and authorized
actual-service compatibility also remain open. No production replacement or
release is authorized by this checkpoint.

## Rotated writer-only attribution

Four paired repetitions rotate `15475fc`/candidate-v4 and `d09eafc`/candidate-v5.
Their only changed runtime input is `src/upload_body.rs`; v4 release SHA256 is
`974aeaf9d5bc63a35837cbc3c0cb495502ec33a7dca9cb000e0772de3beb8158`.
Each mount performs150 consecutive unchanged1 MiB writer operations, without
interleaved read/metadata workloads. These1,200 checked writer phases and eight
final byte/generation checks complete with clean exits, cleanup and interval
quota/reclaim/OOM counters. Observers, workers, budgets and owned resource limits
remain unchanged. This short fixed-work source attribution includes no Go runs
and is not a substitute for long-lived mixed acceptance.

| Pair | v4/v5 MiB/s | v4/v5 p50 ms | v4/v5 p95 ms | v4/v5 pooled p99 ms | v4/v5 daemon CPU s | v4/v5 cgroup CPU s |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 0 | 29.04/29.66 | 17.60104/17.22588 | 21.09325/20.09494 | 23.33374/21.95808 | 1.10/0.97 | 2.1318/1.9772 |
| 1 | 23.58/25.49 | 24.43288/21.23482 | 28.77526/27.77198 | 56.22546/29.54532 | 1.24/1.17 | 2.3227/2.1231 |
| 2 | 23.88/25.62 | 23.14079/20.30924 | 28.54384/28.00446 | 29.63085/29.12473 | 1.26/1.05 | 2.4264/2.2104 |
| 3 | 24.44/25.22 | 23.31183/21.92961 | 28.39880/27.90969 | 30.20378/28.75651 | 1.24/1.08 | 2.3220/2.2041 |

Memory uses the same scopes as above; late now means the latter half of each
short run, not a steady-state plateau. Preserve components that do not improve.

| Pair/binary | Daemon RSS peak/late MiB | Combined RSS peak/late MiB | Cgroup peak/late MiB | File/kernel late MiB | Live allocator peak/late MiB |
| --- | ---: | ---: | ---: | ---: | ---: |
| 0/v4 | 37.86/36.11 | 94.84/93.23 | 71.94/69.41 | 0.47/0.98 | 21.69/19.26 |
| 0/v5 | 31.03/29.60 | 88.02/86.62 | 64.73/61.54 | 0.63/0.95 | 20.39/18.91 |
| 1/v4 | 34.61/32.94 | 91.67/90.11 | 68.20/65.83 | 0.81/0.98 | 21.01/18.98 |
| 1/v5 | 30.20/28.78 | 87.30/85.87 | 65.68/62.56 | 0.78/0.95 | 20.48/18.87 |
| 2/v4 | 34.59/33.79 | 91.77/91.00 | 67.68/65.43 | 0.61/0.96 | 21.15/18.85 |
| 2/v5 | 30.96/28.71 | 88.12/85.91 | 64.48/60.80 | 0.60/0.97 | 20.70/18.98 |
| 3/v4 | 38.09/36.24 | 95.14/93.36 | 72.69/69.66 | 0.78/0.98 | 21.01/19.24 |
| 3/v5 | 31.50/30.42 | 88.55/87.46 | 64.96/62.02 | 0.74/0.96 | 19.51/18.90 |

Candidate throughput improves2.1-8.1% in these four pairs, with lower measured
p50/p95/p99, daemon/cgroup CPU and sampled daemon/combined RSS. Allocator/file/
kernel components are not universal wins. This supports the narrow allocation/
wire correction and does not explain the entire Go-relative mixed writer gap.
