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
isolated Kubernetes fixtures. Dependencies are public crates; this package
forbids unsafe code. Reuse maintained library APIs before custom infrastructure.

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
```

The foreground binary integrates fuser, SDK-backed S3, bounded payload ownership,
paged namespace operations and generation-aware sparse/truncate/fsync writes.
The latest runtime correction resets adaptive read history when a closed stream
reopens, while retaining cached payload and active-reader history. Selected reads
improve, but the representative warm random workload still loses substantially
to both Go references. See [architecture](docs/architecture.md) and
[next work and confirmed causes](docs/next-work.md).

## Latest completed measurements — 2026-10-30

**Diagnostic results, not production acceptance.** The last complete operation
matrix contains 210 runs: 190 correct and 20 failed Go open-file fsync-recovery
runs. The table includes every healthy warm scenario, with five rotated repeats
per implementation; failures and cold results remain in the retained evidence.

Linux 6.8, real FUSE, SeaweedFS 4.51 for healthy workloads; 256 MiB read dataset,
1,024 metadata entries, 512 random 4 KiB reads, four concurrent clients and a
16 MiB writer. Each implementation has a 64 MiB payload budget, 100,000 cache
entries and 8 CPU / 8 GiB for daemon plus client. The backend has 8 CPU / 8 GiB;
the parent has 24 CPU / 16 GiB. Backend throttling was observed in sequential and
eviction runs, so these are not unqualified source-performance wins.

Values are arithmetic means of five run metrics, including **means of run
percentiles, not pooled percentiles**. RSS covers daemon plus client; its late
window averages the latter half of at most 1,000 retained samples (20 ms target
interval), not necessarily half of a long phase and never a plateau. CPU is
consumed user+system seconds per measured phase, not elapsed time or a percentage.
Small process CPU values can
round to zero; cgroup CPU is reported separately. Throughput is MiB/s except
metadata (operations/s); MiB = 2^20 bytes.

| Workload | Implementation | Throughput | p50 / p95 / p99 ms | RSS peak / late-window MiB | CPU daemon / client / cgroup s |
| --- | --- | ---: | --- | --- | --- |
| metadata | Rust | 88,144 ops/s | 0.008 / 0.010 / 0.035 | 85.7 / 85.7 | 0.000 / 0.012 / 0.015 |
| metadata | Go experimental | 47,973 ops/s | 0.008 / 0.026 / 0.300 | 100.6 / 100.6 | 0.026 / 0.020 / 0.037 |
| metadata | Go packaged | 53,645 ops/s | 0.006 / 0.022 / 0.280 | 101.1 / 101.1 | 0.020 / 0.010 / 0.029 |
| sequential | Rust | 276.46 | 0.839 / 5.285 / 13.004 | 195.4 / 182.7 | 0.668 / 0.502 / 1.179 |
| sequential | Go experimental | 138.48 | 3.523 / 17.629 / 28.914 | 350.9 / 245.8 | 6.668 / 0.646 / 7.310 |
| sequential | Go packaged | 125.19 | 3.704 / 19.197 / 33.632 | 355.9 / 243.2 | 7.414 / 0.668 / 8.084 |
| random | Rust | 1.01 | 3.482 / 5.684 / 7.428 | 154.1 / 151.2 | 0.460 / 0.388 / 0.850 |
| random | Go experimental | 7.90 | 0.123 / 0.232 / 1.026 | 164.2 / 164.2 | 0.074 / 0.190 / 0.259 |
| random | Go packaged | 8.33 | 0.122 / 0.255 / 0.897 | 163.5 / 163.4 | 0.074 / 0.172 / 0.244 |
| concurrent | Rust | 4.49 | 2.758 / 5.025 / 6.723 | 186.9 / 183.4 | 1.246 / 1.474 / 2.722 |
| concurrent | Go experimental | 4.72 | 2.540 / 4.978 / 6.812 | 185.1 / 179.6 | 1.586 / 1.428 / 3.011 |
| concurrent | Go packaged | 4.73 | 2.501 / 4.965 / 7.098 | 184.9 / 180.9 | 1.590 / 1.418 / 3.008 |
| eviction | Rust | 278.01 | 0.826 / 5.484 / 11.865 | 206.3 / 194.8 | 0.716 / 0.504 / 1.221 |
| eviction | Go experimental | 140.33 | 3.358 / 18.085 / 29.250 | 356.2 / 241.5 | 6.506 / 0.586 / 7.095 |
| eviction | Go packaged | 125.06 | 3.897 / 19.500 / 29.768 | 360.2 / 243.4 | 7.388 / 0.666 / 8.057 |
| writer/fsync | Rust | 10.04 | 83.337 / 126.114 / 126.114 | 99.8 / 98.9 | 0.328 / 0.262 / 0.589 |
| writer/fsync | Go experimental | 7.83 | 112.057 / 173.366 / 173.366 | 120.8 / 116.5 | 0.608 / 0.244 / 0.852 |
| writer/fsync | Go packaged | 7.71 | 113.278 / 169.656 / 169.656 | 120.0 / 116.3 | 0.646 / 0.268 / 0.915 |

The subsequent persistent-mount observation completed 60 mixed cycles / 360
checked workload passes per implementation, with a 1 MiB writer. Target pacing
was one cycle per 10 seconds; Rust missed that target in 58 cycles. Resources
were raised equally to a 32 CPU parent and 16 CPU backend, while daemon/client
limits and application budgets stayed unchanged. These are the same amount of
checked work over different durations, not equal-rate throughput or plateau proof.

| Implementation | Elapsed s | Late cycles / 60 | RSS peak / whole-series late-half MiB | Cgroup peak / whole-series late-half MiB | CPU daemon / client / cgroup s |
| --- | ---: | ---: | --- | --- | --- |
| Rust | 885.09 | 58 | 253.3 / 223.5 | 497.2 / 314.7 | 196.09 / 181.40 / 377.51 |
| Go experimental | 600.01 | 0 | 414.1 / 233.0 | 611.4 / 298.0 | 1088.32 / 196.47 / 1284.81 |
| Go packaged | 600.00 | 1 | 426.8 / 252.7 | 648.4 / 315.2 | 1473.94 / 188.00 / 1661.94 |

RSS is lower, but Rust is slower in the mixed run and its late-half cgroup memory
is above experimental Go. Go heap observations do not request GC; baseline-owned
GC remains enabled. Observer overhead and short-window missing heap metrics
prevent interpreting these figures as an all-metrics win.

Provenance: Rust runtime `2bf81846d4cff5fa8bdad49068f331af8e1d1818`,
release profile `5d970ff326e2c95cfad4d4020445e1e363cc29d1`, default jemalloc;
runner through `1ecc1b40d6fc6578ab7ff4cc3d8a94ee2dcfc72c`.
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
