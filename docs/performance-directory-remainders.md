# Directory remainder checkpoint — 2026-11-01

Status: **UNACCEPTED**. The consumed-prefix correction has a verified request and
ownership contract, but repeated mixed comparisons do not establish a joint
performance improvement. Short metadata-only gains do not override these losses.

## Sources and design

- Candidate: `eaa8a931e32337e9c8ce02dc8088a87cf424ea6f`, release SHA256
  `4ea43aeeca294e62e38b2294658a8240a0201f0a2ae0c3dd7a976130c55881db`.
- Previous Rust: `d09eafc2f390b235563df68712556dcb51170764`, SHA256
  `27df85c17ab173a8bde307f21e912a1feef7938ceec7bf5fbc863cae670b4e5a`.
  Only `src/filesystem.rs` and `src/fuse.rs` differ among runtime inputs.
- Experimental Go: `5c0884c9e8799bbab93e2d4102fdc15e9c079e0d`; packaged Go:
  `80c214ef14f620d93407454fc76ca01dd71fa25f`. Executable provenance, toolchains
  and the unchanged fixture are described in the [preceding checkpoint](performance-owned-uploads.md).
- Two repetitions, each with four separate 300-second mounts and 30 cycles paced
  at 10 seconds. Order: Go experimental / candidate / Go packaged / prior Rust;
  then candidate / Go packaged / prior Rust / Go experimental. This is a shared
  host with rotated order, not exclusive isolation or every possible order.
- Unchanged 64 MiB payload budget, 100,000-entry limit, 256 MiB checked dataset,
  1,024 metadata entries, 512 random 4 KiB reads, four concurrent readers, 1 MiB
  writer and eight workers. CPU is unbounded equally; filesystem/client, backend
  and owned parent memory limits are 16/12/32 GiB. Default Go GC and Rust jemalloc
  policies remain unchanged; observers never request forced GC.
- All 1,440 phases and eight final full-byte/generation checks pass. Every mount
  meets its schedule, exits cleanly and leaves no owned mount/PID. Interval quota,
  reclaim, high/max and OOM counters are unchanged. Builds, profiling and quality
  checks do not overlap measured windows.

Initial columns describe the first **mixed** cycle, not isolated cold workloads.
Later rates are total work divided by total active phase time for cycles 1–29;
tails are nearest-rank percentiles of their pooled calls, not averages of run
percentiles. Metadata uses ops/s; other rates use MiB/s. Writer p99 has only 29
later calls per mount and is their maximum; preserve that small-sample limitation.

Memory peaks are sampled. Late memory is the temporal latter-half mean of each
300-second window, **not a plateau**. Combined RSS is a pointwise daemon+client
sum; RSS and cgroup memory have different accounting scopes. Rust live allocator
bytes and Go heap observations are separate providers. Jemalloc page gauges
overlap and must not be added together or treated as RSS.

## Repetition 0

| Workload | Binary | Initial rate | Initial p99 ms | Later rate | Later p50 ms | Later p95 ms | Later pooled p99 ms |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| metadata | Rust eaa8a93 | 4607.59 | 0.02027 | 29013.11 | 0.00438 | 0.00733 | 0.62374 |
| metadata | Rust d09eafc | 5969.64 | 0.01973 | 35184.02 | 0.00438 | 0.00712 | 0.12543 |
| metadata | Go experimental | 4780.41 | 0.02150 | 37894.51 | 0.00485 | 0.00944 | 0.02908 |
| metadata | Go packaged | 5812.07 | 0.01906 | 39700.59 | 0.00443 | 0.00752 | 0.02488 |
| sequential | Rust eaa8a93 | 516.44 | 5.82921 | 534.57 | 0.82381 | 1.88521 | 8.13347 |
| sequential | Rust d09eafc | 548.01 | 6.54321 | 554.57 | 0.79254 | 2.28351 | 7.38993 |
| sequential | Go experimental | 215.82 | 22.19500 | 205.42 | 2.75057 | 11.48301 | 21.49821 |
| sequential | Go packaged | 187.87 | 29.19766 | 150.44 | 4.31142 | 16.65527 | 25.56387 |
| random | Rust eaa8a93 | 1.96 | 2.96783 | 1.74 | 2.03963 | 3.18518 | 3.58498 |
| random | Rust d09eafc | 1.69 | 3.52848 | 1.86 | 1.92196 | 3.01058 | 3.34671 |
| random | Go experimental | 1.86 | 3.59506 | 1.88 | 1.94849 | 2.96305 | 3.34339 |
| random | Go packaged | 1.68 | 3.31049 | 1.87 | 1.96895 | 3.00016 | 3.53292 |
| concurrent | Rust eaa8a93 | 8.38 | 3.25005 | 8.96 | 1.63729 | 2.37845 | 2.98840 |
| concurrent | Rust d09eafc | 8.47 | 3.11014 | 9.05 | 1.62005 | 2.32861 | 2.88486 |
| concurrent | Go experimental | 9.38 | 3.10514 | 8.83 | 1.61998 | 2.49401 | 3.24965 |
| concurrent | Go packaged | 8.91 | 3.07502 | 8.97 | 1.59890 | 2.37329 | 3.05681 |
| eviction | Rust eaa8a93 | 591.07 | 4.84944 | 576.82 | 0.76575 | 2.83955 | 4.77639 |
| eviction | Rust d09eafc | 559.17 | 5.61120 | 583.80 | 0.74912 | 2.66025 | 4.96739 |
| eviction | Go experimental | 194.66 | 27.66612 | 213.41 | 2.76141 | 10.86879 | 23.18412 |
| eviction | Go packaged | 199.26 | 20.62152 | 160.83 | 4.00806 | 15.40418 | 26.03190 |
| writer_fsync | Rust eaa8a93 | 23.04 | 21.22645 | 23.44 | 21.22945 | 28.28304 | 28.54605 |
| writer_fsync | Rust d09eafc | 22.59 | 20.47844 | 24.20 | 20.01973 | 26.92389 | 27.73498 |
| writer_fsync | Go experimental | 29.12 | 19.96388 | 30.93 | 20.37760 | 27.22109 | 27.98975 |
| writer_fsync | Go packaged | 26.46 | 19.53535 | 29.68 | 19.11798 | 24.82426 | 25.69212 |

| Binary | Daemon RSS peak/late MiB | Client RSS peak/late MiB | Combined RSS peak/late MiB | Cgroup peak/late MiB | File late MiB | Kernel late MiB | Live allocator peak/late MiB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Rust eaa8a93 | 172.58/157.18 | 62.93/62.85 | 235.43/220.04 | 478.58/222.32 | 19.78 | 3.10 | 87.96/77.34 |
| Rust d09eafc | 175.07/157.48 | 62.96/62.93 | 238.02/220.41 | 477.29/222.08 | 19.33 | 3.12 | 87.03/76.18 |
| Go experimental | 320.62/154.81 | 62.88/62.67 | 382.92/217.48 | 518.27/357.34 | 172.83 | 3.80 | 254.50/97.84 |
| Go packaged | 330.93/176.05 | 62.83/62.80 | 393.73/238.84 | 686.54/374.72 | 158.83 | 3.76 | 255.66/112.64 |

| Binary | Daemon user/system CPU s | Client user/system CPU s | Cgroup CPU s | Cgroup CPU s per cycle |
| --- | ---: | ---: | ---: | ---: |
| Rust eaa8a93 | 30.56/41.49 | 33.67/14.10 | 119.81 | 3.9937 |
| Rust d09eafc | 29.62/40.24 | 33.21/13.76 | 116.83 | 3.8943 |
| Go experimental | 238.11/140.98 | 33.50/17.16 | 429.77 | 14.3256 |
| Go packaged | 415.82/149.02 | 34.31/17.38 | 616.52 | 20.5507 |

Candidate jemalloc late active/resident/mapped/retained-virtual/metadata gauges:
82.63/175.56/201.84/133.04/16.01 MiB. Observed Go forced-GC deltas are 209/247.

## Repetition 1

| Workload | Binary | Initial rate | Initial p99 ms | Later rate | Later p50 ms | Later p95 ms | Later pooled p99 ms |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| metadata | Rust eaa8a93 | 5440.38 | 0.01819 | 36153.26 | 0.00492 | 0.00759 | 0.02991 |
| metadata | Rust d09eafc | 5292.32 | 0.01809 | 30014.80 | 0.00440 | 0.00943 | 0.80920 |
| metadata | Go experimental | 5616.84 | 0.01749 | 42474.90 | 0.00437 | 0.00734 | 0.02398 |
| metadata | Go packaged | 4942.97 | 0.02810 | 42145.34 | 0.00441 | 0.00697 | 0.02249 |
| sequential | Rust eaa8a93 | 610.60 | 4.73677 | 553.56 | 0.76591 | 1.83358 | 7.52119 |
| sequential | Rust d09eafc | 602.73 | 5.03761 | 546.63 | 0.77111 | 2.35715 | 7.90178 |
| sequential | Go experimental | 204.97 | 19.89987 | 207.46 | 2.73990 | 10.99817 | 21.01098 |
| sequential | Go packaged | 180.88 | 23.22365 | 145.40 | 4.61446 | 16.99834 | 26.03106 |
| random | Rust eaa8a93 | 1.78 | 3.52504 | 1.73 | 2.07686 | 3.18059 | 3.64434 |
| random | Rust d09eafc | 1.83 | 3.35632 | 1.92 | 1.90013 | 2.89540 | 3.27148 |
| random | Go experimental | 2.24 | 2.54685 | 1.79 | 2.06217 | 3.06846 | 3.49193 |
| random | Go packaged | 1.71 | 3.55173 | 1.84 | 1.98330 | 3.05179 | 3.70195 |
| concurrent | Rust eaa8a93 | 9.51 | 2.41575 | 9.10 | 1.63323 | 2.32069 | 2.84546 |
| concurrent | Rust d09eafc | 8.84 | 2.95505 | 9.16 | 1.60921 | 2.27601 | 2.82944 |
| concurrent | Go experimental | 9.00 | 2.96532 | 8.87 | 1.61736 | 2.43904 | 3.10835 |
| concurrent | Go packaged | 9.09 | 3.15505 | 9.08 | 1.58088 | 2.37491 | 3.13765 |
| eviction | Rust eaa8a93 | 593.24 | 4.36861 | 570.39 | 0.78125 | 2.91074 | 4.96428 |
| eviction | Rust d09eafc | 598.98 | 4.86867 | 574.87 | 0.76604 | 2.84555 | 5.21673 |
| eviction | Go experimental | 210.82 | 24.22275 | 208.38 | 2.73795 | 11.67714 | 25.02720 |
| eviction | Go packaged | 180.28 | 24.41851 | 157.90 | 4.10307 | 15.84008 | 26.22232 |
| writer_fsync | Rust eaa8a93 | 16.55 | 26.98951 | 23.83 | 21.31515 | 26.26859 | 28.45151 |
| writer_fsync | Rust d09eafc | 19.77 | 23.98277 | 23.66 | 21.63686 | 27.58873 | 29.74587 |
| writer_fsync | Go experimental | 29.89 | 19.05049 | 31.32 | 20.63920 | 24.83395 | 25.11426 |
| writer_fsync | Go packaged | 23.51 | 24.76545 | 28.29 | 20.26856 | 27.30492 | 28.19687 |

| Binary | Daemon RSS peak/late MiB | Client RSS peak/late MiB | Combined RSS peak/late MiB | Cgroup peak/late MiB | File late MiB | Kernel late MiB | Live allocator peak/late MiB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Rust eaa8a93 | 176.50/153.49 | 62.97/62.92 | 239.43/216.41 | 478.64/217.54 | 19.86 | 3.12 | 87.05/76.14 |
| Rust d09eafc | 176.78/153.79 | 62.85/62.81 | 239.46/216.60 | 477.32/217.74 | 18.89 | 3.12 | 86.58/75.18 |
| Go experimental | 318.79/154.14 | 62.73/62.69 | 381.45/216.83 | 520.37/354.38 | 170.15 | 3.75 | 247.16/92.61 |
| Go packaged | 329.60/176.36 | 62.88/62.82 | 392.46/239.18 | 683.19/374.57 | 159.00 | 3.76 | 258.51/112.55 |

| Binary | Daemon user/system CPU s | Client user/system CPU s | Cgroup CPU s | Cgroup CPU s per cycle |
| --- | ---: | ---: | ---: | ---: |
| Rust eaa8a93 | 30.30/40.76 | 33.05/13.79 | 117.90 | 3.9299 |
| Rust d09eafc | 29.81/39.58 | 32.93/13.84 | 116.16 | 3.8718 |
| Go experimental | 235.96/142.35 | 33.64/17.25 | 429.20 | 14.3067 |
| Go packaged | 432.81/152.02 | 33.74/17.60 | 636.17 | 21.2056 |

Candidate jemalloc late active/resident/mapped/retained-virtual/metadata gauges:
81.46/170.66/196.93/140.07/16.02 MiB. Observed Go forced-GC deltas are 208/249.

## Mechanism and remaining gates

The namespace-pressure regression first observes two LISTs rather than one after
an already-emitted inode is reclaimed. Returning only the remaining page avoids
that request and repeated prefix validation/copies, without pinning inodes or
extending freshness. Stale-type and inserted-prefix tests reject deliberate
freshness and continuation faults. Both allocator suites pass 258 Rust cases and
75 benchmark Python cases; three matching release/instrumented native client
variants pass. The selected partition mutation is killed. New helpers have CRAP
1; affected directory orchestration remains at 7 or below, but the whole-file
CRAP gate and broader mutation/compatibility gates still fail or remain open.

Four rotated 1,000-traversal metadata-only source pairs check 8,192,000 entries and
eight final dataset generations. Throughput improves 7.0–13.3%; some tails and
memory metrics lose. The separate supported READDIRPLUS_AUTO experiment fails
three of four throughput comparisons and all four pooled-p99 comparisons, so its
runtime/test changes are discarded and its evidence retained.

The repeated mixed results above are decisive for scope: candidate metadata rate
and p99 improve against prior Rust in one repetition and lose in the other.
Random throughput and whole-cgroup CPU lose against prior Rust in both. Candidate
cgroup memory and large-read speed beat Go, but process RSS is not a reproducible
win over experimental Go, and metadata/writer throughput still loses. Retain all
losses; neither the short pairs nor the best mixed repetition passes acceptance.
Attribute the remaining metadata bursts and writer request amplification before
another source change. Actual-service, positive LINK and other compatibility
requirements remain in [next work](next-work.md).
