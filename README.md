# geesefs-rs

Experimental Linux/S3 GeeseFS replacement for readers and writers.
**Not accepted for production:** compatibility and comparative performance remain
incomplete. Do not replace a served mount with this implementation.

The goal is preserved filesystem/S3 behavior with lower memory use and faster
normal, sustained operation than both identified Go references. A language change,
passing unit tests or one favorable workload does not satisfy that goal.

## Development

Use Rust 1.96.0 or newer with rustfmt/clippy and Python 3.12+ with dependencies
from `requirements-fixture.txt`. Builds, tests and mounted benchmarks run in
isolated disposable fixtures; the operator also authorized the OpenCode VM for
the 2026-10-31 diagnostics. Dependencies are public crates; this package
forbids unsafe code. Reuse maintained library APIs before custom infrastructure.

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
```

The foreground binary integrates fuser, SDK-backed S3, bounded payload ownership,
paged namespace operations and generation-aware sparse/truncate/fsync writes.
Required admission can retain exclusive clean cache backings in revocable idle
flush capacity without increasing the payload limit; upload progress and owned
reply charges remain protected. The new policy removes repeated warm-random GETs,
but concurrent reads, tails, sustained rate and memory still fail the agreed
all-metrics goal. See [architecture](docs/architecture.md) and
[next work and confirmed causes](docs/next-work.md).

## Persistent admission correction — 2026-10-31

Native FUSE/ownership diagnostics attribute the rare 30-second read to admission
missing cache owners published after its initial reclamation sweep. Required
admission now reclaims on publication while retaining its FIFO reservation.
Directory-local epochs also prevent unrelated writes from invalidating every
fresh listing page. Gated regressions fail before correction; 218 all-feature
Rust cases, Clippy, formatting and the public-source audit pass.

The final release completes 60 persistent mixed cycles per implementation, with
1,440 checked passes and four full-byte/generation checks passing. The same work
takes 200.89 s versus prior Rust 251.35 s and Go experimental/packaged
288.61/379.29 s. Metadata active time drops 21.08 to 2.46 s; calls over 20 s drop
from one to zero. Random p99 still slightly regresses against prior Rust, and
metadata/random/writer metrics retain Go losses. These are scoped VM diagnostics,
not all-workload acceptance.

A reverse-order control runs 30 cycles at one cycle per 10 s: all four binaries
complete 300 s without lateness, and all 720 passes/four generation checks pass.
Corrected Rust uses 114.93 whole-cgroup CPU seconds versus Go 428.46/640.10, but
late cgroup memory is higher: 389.8 MiB versus 350.6/371.4. Metadata pooled p99
regresses to 1.26849 ms versus prior Rust 0.07488 and Go 0.02886/0.02510, including
late-run slow calls. This remains an open acceptance failure. See
[method, CPU/memory/tails and qualifications](docs/performance.md#native-vm-admissiondirectory-checkpoint--2026-10-31).

## Latest completed measurements — 2026-10-30

This is the earlier idle-flush cache-policy checkpoint, retained independently
from the subsequent admission/directory correction above.

**Diagnostic results, not production acceptance.** The cache-policy comparison
completes 120 healthy-warm records: five repetitions of six workloads on the new
Rust release, its unchanged predecessor and both Go references. Four implementation
orders rotate, with the first repeated once. All byte checks and 30 frozen payload
generation checks pass. The earlier 210-record cold/warm/failure matrix, including
20 Go open-file fsync-recovery failures, remains retained; this healthy-only
comparison does not replace its missing compatibility or failure gates.

Linux 6.8, real FUSE, SeaweedFS 4.51; 256 MiB payload, 1,024 metadata entries,
512 random 4 KiB reads per worker, four concurrent clients, 16 MiB writer,
64 MiB application budget and 100,000 cache entries. Each record has one matching
warmup and eight checked measured passes. All implementations use eight runtime
workers, no CPU quota, and memory maxima of 16 GiB daemon/client, 12 GiB backend
and 32 GiB parent. Go retains its default allocation-threshold/pressure GC.
One disposable bucket lives for each four-binary workload group; identical
payload/metadata and generation are verified before scoped cleanup. This backend
lifetime change is a resource configuration, not a runtime optimization.

**Throughput is total checked work divided by summed measured phase time**,
in MiB/s except metadata (operations/s). Latency, RSS and CPU columns are means
of five run metrics; **mean run percentiles are not pooled percentiles**. CPU is
consumed user+system seconds per record, not wall time. RSS includes daemon plus
client. Its late half is selected by timestamp from the complete raw series;
sampled peaks and late means are not exact peaks or plateau evidence. MiB = 2^20 bytes.

| Workload | Implementation | Throughput work/time | Mean run p50 / p95 / p99 ms | RSS peak / late-half MiB | CPU daemon / client / cgroup s |
| --- | --- | ---: | --- | --- | --- |
| metadata | Rust predecessor | 145,300 ops/s | 0.003 / 0.008 / 0.037 | 84.2 / 84.2 | 0.018 / 0.056 / 0.068 |
| metadata | Rust candidate | 151,840 ops/s | 0.003 / 0.007 / 0.023 | 85.1 / 85.0 | 0.014 / 0.052 / 0.061 |
| metadata | Go experimental | 75,041 ops/s | 0.003 / 0.013 / 0.202 | 99.4 / 99.0 | 0.084 / 0.060 / 0.149 |
| metadata | Go packaged | 78,704 ops/s | 0.003 / 0.011 / 0.213 | 100.7 / 100.2 | 0.084 / 0.060 / 0.143 |
| sequential | Rust predecessor | 42.97 | 0.805 / 4.694 / 11.868 | 215.7 / 195.1 | 5.034 / 3.930 / 8.960 |
| sequential | Rust candidate | 115.93 | 0.801 / 5.122 / 12.079 | 235.3 / 213.6 | 4.226 / 3.874 / 8.097 |
| sequential | Go experimental | 134.56 | 3.635 / 16.866 / 29.515 | 370.8 / 266.0 | 57.500 / 5.322 / 62.821 |
| sequential | Go packaged | 129.40 | 3.670 / 18.231 / 30.251 | 389.0 / 269.0 | 62.144 / 5.476 / 67.613 |
| random | Rust predecessor | 1.01 | 3.622 / 5.169 / 6.955 | 170.4 / 159.8 | 3.650 / 3.236 / 6.888 |
| random | Rust candidate | 9.07 | 0.100 / 0.189 / 0.926 | 141.1 / 140.8 | 0.408 / 1.308 / 1.712 |
| random | Go experimental | 8.80 | 0.121 / 0.203 / 0.714 | 166.3 / 165.5 | 0.534 / 1.294 / 1.831 |
| random | Go packaged | 8.88 | 0.111 / 0.201 / 0.650 | 165.7 / 165.0 | 0.480 / 1.320 / 1.804 |
| concurrent | Rust predecessor | 4.27 | 2.889 / 5.057 / 6.821 | 200.3 / 189.4 | 10.242 / 12.848 / 23.089 |
| concurrent | Rust candidate | 4.09 | 3.086 / 5.318 / 7.134 | 216.0 / 206.6 | 10.622 / 13.340 / 23.968 |
| concurrent | Go experimental | 4.41 | 2.725 / 4.953 / 6.947 | 192.8 / 185.0 | 14.620 / 12.690 / 27.314 |
| concurrent | Go packaged | 4.35 | 2.794 / 5.149 / 7.272 | 204.0 / 194.5 | 16.994 / 12.678 / 29.671 |
| eviction | Rust predecessor | 57.33 | 0.820 / 4.841 / 12.144 | 213.1 / 198.3 | 4.824 / 3.986 / 8.808 |
| eviction | Rust candidate | 86.59 | 0.799 / 4.694 / 11.848 | 231.7 / 205.0 | 4.416 / 3.908 / 8.323 |
| eviction | Go experimental | 137.24 | 3.453 / 17.368 / 28.456 | 369.2 / 265.4 | 56.336 / 5.216 / 61.549 |
| eviction | Go packaged | 132.92 | 3.575 / 18.147 / 30.113 | 373.2 / 270.5 | 60.018 / 5.218 / 65.236 |
| writer/fsync | Rust predecessor | 9.06 | 86.531 / 145.495 / 174.995 | 104.4 / 101.7 | 2.842 / 2.126 / 4.965 |
| writer/fsync | Rust candidate | 8.56 | 89.179 / 183.248 / 240.612 | 104.0 / 101.0 | 2.906 / 2.206 / 5.115 |
| writer/fsync | Go experimental | 7.01 | 119.819 / 177.852 / 427.142 | 123.1 / 115.5 | 5.132 / 2.078 / 7.210 |
| writer/fsync | Go packaged | 7.17 | 117.192 / 178.469 / 289.839 | 122.6 / 114.8 | 5.110 / 2.124 / 7.236 |

Random candidate/predecessor GET counts are 0 versus 3,380–3,389 per record.
The new random mean p99 still loses to both Go references; concurrent throughput,
RSS and cgroup memory also lose. Writer pooled p99 is 348.573 ms versus
288.321/347.198 ms for experimental/packaged Go, despite the more favorable
mean-run-p99 column. These adverse observations remain acceptance failures.

Cgroup peaks include mount startup/warmup; cgroup/file/kernel late halves use
the full raw time series. Allocator columns use sampled jemalloc allocated versus
Go HeapAlloc with different scopes/rates. Go metadata has no sufficient heap
series in all five records, and random lacks one of five; aggregates remain
missing. Missing is not zero. Shared process RSS and cgroup charges have different
ownership scopes and need not order the same way.

| Workload | Implementation | Cgroup peak / late-half MiB | File / kernel late-half MiB | Allocator peak / late-half MiB |
| --- | --- | --- | --- | --- |
| metadata | Rust predecessor | 58.4 / 58.1 | 0.0 / 2.2 | 23.0 / 23.0 |
| metadata | Rust candidate | 59.1 / 58.8 | 0.0 / 2.1 | 23.2 / 23.2 |
| metadata | Go experimental | 56.8 / 55.1 | 0.0 / 2.1 | missing / missing |
| metadata | Go packaged | 57.6 / 56.3 | 0.0 / 2.1 | missing / missing |
| sequential | Rust predecessor | 443.9 / 324.2 | 155.8 / 1.5 | 69.8 / 57.8 |
| sequential | Rust candidate | 459.8 / 314.7 | 127.5 / 1.5 | 79.9 / 59.7 |
| sequential | Go experimental | 519.7 / 358.2 | 115.4 / 2.2 | 227.6 / 144.2 |
| sequential | Go packaged | 507.2 / 359.7 | 113.0 / 2.2 | 221.1 / 159.5 |
| random | Rust predecessor | 146.9 / 135.4 | 2.4 / 1.2 | 69.3 / 68.7 |
| random | Rust candidate | 118.4 / 115.7 | 2.1 / 1.1 | 77.5 / 77.2 |
| random | Go experimental | 125.0 / 122.0 | 2.0 / 1.3 | missing / missing |
| random | Go packaged | 125.1 / 122.0 | 2.0 / 1.3 | missing / missing |
| concurrent | Rust predecessor | 189.1 / 171.9 | 8.4 / 1.6 | 70.9 / 70.1 |
| concurrent | Rust candidate | 205.0 / 189.1 | 8.4 / 1.7 | 80.7 / 79.5 |
| concurrent | Go experimental | 158.7 / 148.8 | 8.2 / 1.8 | 84.2 / 75.5 |
| concurrent | Go packaged | 169.8 / 158.5 | 8.0 / 1.8 | 91.7 / 84.4 |
| eviction | Rust predecessor | 436.6 / 319.3 | 147.5 / 1.5 | 69.9 / 57.7 |
| eviction | Rust candidate | 452.7 / 308.1 | 129.4 / 1.5 | 79.7 / 58.4 |
| eviction | Go experimental | 507.9 / 353.6 | 112.3 / 2.2 | 225.3 / 147.2 |
| eviction | Go packaged | 559.6 / 360.7 | 111.5 / 2.2 | 251.8 / 156.5 |
| writer/fsync | Rust predecessor | 98.2 / 83.7 | 9.5 / 0.9 | 23.0 / 20.8 |
| writer/fsync | Rust candidate | 97.3 / 83.0 | 9.6 / 0.9 | 23.3 / 20.9 |
| writer/fsync | Go experimental | 97.3 / 78.8 | 9.5 / 1.1 | 25.3 / 18.5 |
| writer/fsync | Go packaged | 96.8 / 78.4 | 9.5 / 1.1 | 25.3 / 18.7 |

No measured interval has quota throttling, memory PSI, reclaim or OOM. This does
not mean zero waiting: host busy ranges 12.9–51.9%, CPU PSI-some reaches 7.9%, and
filesystem/client I/O PSI-some reaches 91.8% of an interval. Both Rust binaries
have rare successful approximately 30-second reads; none occur in either Go.
Candidate sequential records span 5.55–35.72 seconds and eviction 5.39–65.82.
Its arithmetic mean sequential rate is 238.84 MiB/s, but work/time is 115.93;
all repeats and both aggregations remain retained. I/O PSI identifies waiting,
not its causal SDK, admission, FUSE or physical-backend stage.

### Persistent mixed mount

The subsequent observation completes 60 mixed cycles / 360 checked passes per
binary, 1,440 passes total, with the same limits and a 1 MiB writer. Target pacing
is one cycle per 10 seconds, with every late cycle retained. Order is packaged
Go, candidate Rust, predecessor Rust, experimental Go. This is one equal-work
observation per binary, not repeated equal-rate throughput or plateau proof.

| Implementation | Elapsed s | Late cycles / 60 | RSS peak / whole-series late-half MiB | Cgroup peak / whole-series late-half MiB | CPU daemon / client / cgroup s |
| --- | ---: | ---: | --- | --- | --- |
| Rust candidate | 709.11 | 58 | 274.0 / 232.1 | 515.8 / 296.0 | 200.61 / 194.41 / 395.02 |
| Rust predecessor | 873.78 | 54 | 261.8 / 225.4 | 509.3 / 294.7 | 206.49 / 192.32 / 398.82 |
| Go experimental | 600.00 | 0 | 412.4 / 226.9 | 600.8 / 300.1 | 1056.65 / 195.85 / 1252.50 |
| Go packaged | 600.00 | 0 | 414.0 / 244.4 | 619.0 / 300.2 | 1499.07 / 221.92 / 1720.99 |

| Implementation | File / kernel late-half MiB | Allocator peak / late-half MiB |
| --- | --- | --- |
| Rust candidate | 88.8 / 3.5 | 96.8 / 83.1 |
| Rust predecessor | 94.3 / 3.6 | 85.7 / 73.5 |
| Go experimental | 108.9 / 3.7 | 297.9 / 106.7 |
| Go packaged | 85.7 / 3.6 | 272.8 / 122.4 |

Candidate Rust completes the work sooner than its predecessor, but still takes
18.2% longer than either Go and has higher late-half RSS than experimental Go.
The candidate has 11 successful approximately 30-second serial reads; the
predecessor has 16 and both Go have zero. Memory/quota counters remain clean,
while filesystem/client I/O PSI-some is 57.9%/63.3% for candidate/predecessor.
The responsible wait path is not yet attributed. Observer overhead remains
unisolated. Go observations request no GC; baseline-owned forced-GC deltas are
607/689 for experimental/packaged Go. No plateau or all-metrics win is established.

Provenance: candidate source `35ee52c7713c2dd29e6787a81f6ced867de2c1b6`,
default jemalloc/ThinLTO, Rust 1.99.1. Predecessor runtime
`2bf81846d4cff5fa8bdad49068f331af8e1d1818`, release profile
`5d970ff326e2c95cfad4d4020445e1e363cc29d1`; frozen runner `7155df7`.
Go experimental `5c0884c9e8799bbab93e2d4102fdc15e9c079e0d`, packaged
`80c214ef14f620d93407454fc76ca01dd71fa25f`, both built with Go 1.25.9.
Exact executable hashes and reproduction boundaries are in
[performance methodology](docs/performance.md).

## Project context

- [Project brief](docs/project-brief.md) and [agent rules](AGENTS.md).
- [Change history and reasons](docs/change-history.md).
- [Architecture and ownership](docs/architecture.md), [next work](docs/next-work.md).
- [Compatibility inventory](docs/compatibility.md) and [backend options](docs/backend-options.md).
- [Dependency decisions](docs/dependency-decisions.md).
- [Testing](docs/testing.md), [performance methodology](docs/performance.md) and
  [sealed read fixture](docs/immutable-read-dataset.md).

Detailed measurements, generated graphs and execution logs belong in ignored
`benchmark-results/private/`, not in source diffs. Keep a concise explanation of
each confirmed mechanism, correction, tradeoff and remaining gap in the history.
