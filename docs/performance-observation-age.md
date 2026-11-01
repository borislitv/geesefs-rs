# Metadata observation-age checkpoint — 2026-11-01

Status: **UNACCEPTED**. This source corrects cached user-metadata freshness; it
does not demonstrate the required joint performance and compatibility win.

## Correction and verification

Standard LIST contains basic object fields, not arbitrary user metadata. A new
names page previously assigned its request timestamp to cached permission data.
The age regression returns nearly 60 seconds instead of at most 15; a real native
test changes backend mode 0640 to 0600 without changing bytes/size and rejects
the old permission after the original TTL. The initial size-only probe is retained
as limited evidence because LIST can revalidate size.

Carry each selected cached/LIST/HEAD entry's actual observation time through
bounded merging and type resolution. Preserve configured TTL/budgets, exact types,
lookup ownership and newer HEAD/commit protections. Both allocator suites pass
260 Rust cases, including ten native directory contracts, plus 75 benchmark Python
cases. Three matching release/instrumented native client variants pass available
rw/ro, rsync/nginx, metadata, full-byte/Range/EOF, mmap-after-close and durability/
failure contracts. Formatting, Clippy, public-source audit and duplicate checks
pass. Whole-file CRAP fails; automated mutation has no sites for the new timestamp
assignments. The restored old assignment fails the native permission test; this
targeted proof does not close broader mutation coverage.

## Sources and unchanged control

- Candidate: `1b13033c3f161b9502c7a23870b645ca871ab2e3`; release SHA256
  `df24b6bdd42920b7989db2ed7c44855b9fc2bb1cf53292ee60ce023eb42138a8`.
- Prior Rust: `eaa8a931e32337e9c8ce02dc8088a87cf424ea6f`; SHA256
  `4ea43aeeca294e62e38b2294658a8240a0201f0a2ae0c3dd7a976130c55881db`.
  Only `src/filesystem.rs` differs among runtime inputs. The prior source has the
  now-known permission-freshness gap and is not a correctness-equivalent winner.
- Experimental/packaged Go remain `5c0884c9e8799bbab93e2d4102fdc15e9c079e0d` /
  `80c214ef14f620d93407454fc76ca01dd71fa25f`; full provenance is retained with the
  [earlier checkpoints](performance-owned-uploads.md).
- Same Linux VM, SeaweedFS 4.51, eight workers, 64 MiB payload, 100,000 entries,
  256 MiB checked dataset, 1,024 metadata entries, 512 random 4 KiB reads, four
  concurrent readers and 1 MiB writer. Equal unbounded CPU; filesystem/client,
  backend and owned parent memory limits are 16/12/32 GiB. No allocator, GC, TTL,
  dataset, cache-budget or durability tuning.
- Two repetitions, four separate 300-second mounts each, 30 cycles at 10s.
  Order: prior Rust / Go packaged / candidate / Go experimental; then Go packaged /
  candidate / Go experimental / prior Rust. All 1,440 phases and eight final
  full-byte/generation checks pass, with clean exits/owners, no late cycles and
  unchanged interval quota/reclaim/high/max/OOM counters. No build, profile or
  quality check overlaps measured windows. This remains a shared host.

Initial means the first mixed cycle, not an isolated cold workload. Later rates
use total work/time for cycles 1–29, with nearest-rank pooled call percentiles.
Metadata rates are ops/s; payload rates are MiB/s. Writer p99 is the maximum of
only 29 later calls per mount. Sampled peaks and temporal latter-half means are
not memory plateaus. Combined RSS is pointwise daemon+client; live allocator,
RSS and cgroup scopes differ. Jemalloc page gauges overlap and are not additive.

## Repetition 0

| Workload | Binary | Initial rate | Initial p99 ms | Later rate | Later p50 ms | Later p95 ms | Later pooled p99 ms |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| metadata | Rust 1b13033 | 5116.87 | 0.02111 | 22398.01 | 0.00451 | 0.01066 | 0.02902 |
| metadata | Rust eaa8a93 | 4715.07 | 0.02553 | 34311.15 | 0.00450 | 0.00763 | 0.02883 |
| metadata | Go experimental | 5378.51 | 0.02250 | 36501.78 | 0.00710 | 0.00902 | 0.03186 |
| metadata | Go packaged | 4217.03 | 0.02297 | 39688.16 | 0.00447 | 0.00762 | 0.02666 |
| sequential | Rust 1b13033 | 620.20 | 5.75943 | 572.92 | 0.73996 | 1.60034 | 7.23593 |
| sequential | Rust eaa8a93 | 603.47 | 5.03216 | 552.21 | 0.77022 | 2.03928 | 8.02426 |
| sequential | Go experimental | 185.66 | 29.56760 | 186.68 | 3.23303 | 12.14716 | 22.75736 |
| sequential | Go packaged | 185.54 | 22.59141 | 147.25 | 4.46223 | 16.46558 | 25.88992 |
| random | Rust 1b13033 | 1.61 | 3.95367 | 1.73 | 2.10583 | 3.17278 | 3.52112 |
| random | Rust eaa8a93 | 1.84 | 3.31597 | 1.86 | 1.96033 | 2.93423 | 3.34026 |
| random | Go experimental | 2.03 | 2.96412 | 1.91 | 1.87980 | 2.96161 | 3.44141 |
| random | Go packaged | 1.93 | 3.10225 | 2.01 | 1.83699 | 2.75967 | 3.33058 |
| concurrent | Rust 1b13033 | 9.16 | 2.92768 | 8.96 | 1.65293 | 2.32298 | 2.86399 |
| concurrent | Rust eaa8a93 | 8.64 | 2.83230 | 8.90 | 1.65087 | 2.36794 | 2.88029 |
| concurrent | Go experimental | 8.78 | 2.86768 | 9.01 | 1.61081 | 2.32196 | 2.95577 |
| concurrent | Go packaged | 8.93 | 2.81407 | 8.91 | 1.61145 | 2.39549 | 3.10383 |
| eviction | Rust 1b13033 | 626.06 | 4.42502 | 585.15 | 0.73426 | 2.66517 | 5.31437 |
| eviction | Rust eaa8a93 | 613.06 | 4.51351 | 588.61 | 0.73775 | 2.66546 | 4.80136 |
| eviction | Go experimental | 197.28 | 24.49447 | 207.23 | 2.83500 | 11.72985 | 23.23818 |
| eviction | Go packaged | 190.82 | 25.77216 | 154.83 | 4.13660 | 16.70628 | 26.23720 |
| writer_fsync | Rust 1b13033 | 20.50 | 22.32411 | 23.39 | 21.21606 | 28.61939 | 29.29933 |
| writer_fsync | Rust eaa8a93 | 19.79 | 20.27686 | 25.25 | 19.09738 | 25.10385 | 27.54256 |
| writer_fsync | Go experimental | 28.30 | 20.80551 | 31.02 | 20.14679 | 25.52261 | 25.65515 |
| writer_fsync | Go packaged | 27.69 | 18.98590 | 28.38 | 20.58715 | 26.87520 | 28.27837 |

| Binary | Daemon RSS peak/late MiB | Client RSS peak/late MiB | Combined RSS peak/late MiB | Cgroup peak/late MiB | File late MiB | Kernel late MiB | Live allocator peak/late MiB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Rust 1b13033 | 177.89/157.26 | 62.87/62.83 | 240.71/220.08 | 480.50/221.23 | 19.14 | 3.14 | 88.95/78.00 |
| Rust eaa8a93 | 175.67/154.49 | 63.04/63.00 | 238.69/217.49 | 479.77/218.10 | 19.60 | 3.08 | 87.52/76.46 |
| Go experimental | 308.71/157.84 | 62.91/62.86 | 371.51/220.71 | 558.14/357.43 | 170.96 | 3.83 | 249.13/99.92 |
| Go packaged | 326.92/175.03 | 62.96/62.88 | 389.80/237.91 | 749.29/375.40 | 161.67 | 3.82 | 276.71/116.92 |

| Binary | Daemon user/system CPU s | Client user/system CPU s | Cgroup CPU s | Cgroup CPU s per cycle |
| --- | ---: | ---: | ---: | ---: |
| Rust 1b13033 | 29.79/39.34 | 33.51/13.97 | 116.61 | 3.8871 |
| Rust eaa8a93 | 30.07/38.82 | 33.35/14.24 | 116.48 | 3.8826 |
| Go experimental | 251.62/147.21 | 33.55/19.14 | 451.52 | 15.0508 |
| Go packaged | 431.06/155.08 | 33.92/17.66 | 637.72 | 21.2574 |

Candidate jemalloc late active/resident/mapped/retained-virtual/metadata gauges:
83.49/174.30/200.60/140.90/16.01 MiB.

## Repetition 1

| Workload | Binary | Initial rate | Initial p99 ms | Later rate | Later p50 ms | Later p95 ms | Later pooled p99 ms |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| metadata | Rust 1b13033 | 4014.83 | 0.02321 | 20238.66 | 0.00712 | 0.00805 | 0.03583 |
| metadata | Rust eaa8a93 | 5361.08 | 0.02182 | 34626.70 | 0.00460 | 0.00759 | 0.03491 |
| metadata | Go experimental | 5873.56 | 0.02593 | 42407.25 | 0.00443 | 0.00741 | 0.02530 |
| metadata | Go packaged | 5118.70 | 0.02322 | 36803.14 | 0.00452 | 0.00825 | 0.02891 |
| sequential | Rust 1b13033 | 549.04 | 6.53674 | 518.69 | 0.84956 | 2.17057 | 8.57331 |
| sequential | Rust eaa8a93 | 617.52 | 4.56333 | 572.86 | 0.75542 | 1.59912 | 7.20981 |
| sequential | Go experimental | 199.46 | 23.78372 | 201.35 | 2.85462 | 11.24840 | 22.20160 |
| sequential | Go packaged | 215.47 | 18.91781 | 148.30 | 4.52562 | 16.24569 | 25.26058 |
| random | Rust 1b13033 | 1.57 | 3.59165 | 1.79 | 2.02881 | 3.05749 | 3.39511 |
| random | Rust eaa8a93 | 1.76 | 3.56619 | 1.89 | 1.93791 | 2.94131 | 3.31318 |
| random | Go experimental | 2.16 | 2.52405 | 1.97 | 1.86438 | 2.79711 | 3.26228 |
| random | Go packaged | 1.64 | 3.44807 | 1.74 | 2.09947 | 3.18375 | 3.72052 |
| concurrent | Rust 1b13033 | 9.18 | 2.83191 | 9.32 | 1.58652 | 2.24210 | 2.84418 |
| concurrent | Rust eaa8a93 | 9.16 | 2.92018 | 9.00 | 1.63324 | 2.39156 | 2.95575 |
| concurrent | Go experimental | 8.74 | 2.87203 | 9.00 | 1.61906 | 2.30975 | 2.86197 |
| concurrent | Go packaged | 8.31 | 3.43029 | 8.90 | 1.60123 | 2.43122 | 3.19166 |
| eviction | Rust 1b13033 | 625.45 | 3.89092 | 594.06 | 0.72746 | 2.68910 | 4.74444 |
| eviction | Rust eaa8a93 | 598.82 | 4.46068 | 586.02 | 0.74426 | 2.73029 | 5.05429 |
| eviction | Go experimental | 204.39 | 33.13510 | 205.18 | 2.81740 | 11.82921 | 22.93987 |
| eviction | Go packaged | 191.88 | 18.10271 | 156.70 | 4.31651 | 15.93391 | 24.70306 |
| writer_fsync | Rust 1b13033 | 18.69 | 24.81447 | 26.13 | 18.47955 | 25.15467 | 27.04644 |
| writer_fsync | Rust eaa8a93 | 24.24 | 17.85818 | 24.50 | 19.32435 | 28.21941 | 28.41903 |
| writer_fsync | Go experimental | 30.04 | 18.84493 | 31.44 | 19.87307 | 27.09889 | 28.56811 |
| writer_fsync | Go packaged | 27.95 | 18.55056 | 28.85 | 19.72324 | 24.07139 | 26.40947 |

| Binary | Daemon RSS peak/late MiB | Client RSS peak/late MiB | Combined RSS peak/late MiB | Cgroup peak/late MiB | File late MiB | Kernel late MiB | Live allocator peak/late MiB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Rust 1b13033 | 175.52/159.03 | 62.94/62.85 | 238.43/221.88 | 481.30/223.29 | 19.52 | 3.09 | 87.92/77.09 |
| Rust eaa8a93 | 193.46/156.23 | 63.09/63.03 | 256.54/219.26 | 501.76/220.40 | 19.59 | 3.11 | 86.36/75.32 |
| Go experimental | 319.30/158.07 | 62.87/62.79 | 382.15/220.86 | 582.68/359.34 | 172.62 | 3.73 | 246.17/101.24 |
| Go packaged | 332.80/171.07 | 62.91/62.87 | 395.67/233.94 | 671.85/369.25 | 158.54 | 3.69 | 255.28/109.92 |

| Binary | Daemon user/system CPU s | Client user/system CPU s | Cgroup CPU s | Cgroup CPU s per cycle |
| --- | ---: | ---: | ---: | ---: |
| Rust 1b13033 | 30.08/40.38 | 33.20/14.08 | 117.74 | 3.9248 |
| Rust eaa8a93 | 29.56/38.74 | 32.53/13.94 | 114.78 | 3.8261 |
| Go experimental | 243.60/144.07 | 33.60/17.23 | 438.52 | 14.6172 |
| Go packaged | 426.21/152.62 | 34.44/17.67 | 630.96 | 21.0320 |

Candidate jemalloc late active/resident/mapped/retained-virtual/metadata gauges:
82.27/176.00/202.28/121.88/16.01 MiB.

## Remaining work

Metadata and writer rates still lose to both Go references. Process RSS is not a
reproducible win over experimental Go; prior-Rust-relative CPU, late RSS/cgroup,
allocator and random metrics retain losses. Correctness must not be exchanged
for the predecessor's lower metadata-refresh cost.

Separate checked 14-cycle instrumented traces retain HEAD bursts at expiry and
five HEADs/two LISTs per writer, plus PUT/GET/DELETE. Standard LIST cannot justify
refreshing absent permission metadata. Attribute the remaining request/work cost
without extending TTL, changing negative-cache policy or dropping post-commit
verification. The formal joint performance, actual-service, positive LINK and
remaining compatibility/quality gates in [next work](next-work.md) stay open.
