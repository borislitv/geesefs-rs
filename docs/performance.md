# Performance acceptance: UNACCEPTED

The selected steady-state read fixture now favors Rust throughput and daemon RSS,
but mean p99 still loses to both Go references. Representative metadata, random,
concurrent-reader and writer/fsync workloads, long-lived memory behavior and full
compatibility remain open. Cold-start superiority is not required. See the
[change history](change-history.md) for confirmed mechanisms and corrections.

## Comparison contract

- Pin public source revisions, compiler versions, executable hashes, dataset
  manifest/generations, kernel, mount arguments and workload order.
- Compare both Go references: experimental
  `5c0884c9e8799bbab93e2d4102fdc15e9c079e0d` and packaged
  `80c214ef14f620d93407454fc76ca01dd71fa25f`. Retained Go binaries use Go 1.25.9;
  an earlier report incorrectly identified its Go 1.22.2 launcher as the compiler.
- Hold budgets, concurrency and resources equal. The current native diagnostic
  uses 512 MiB application payload, 100,000 entries, four 128 KiB readers and eight
  runtime workers; filesystem/client has 8 CPU / 16 GiB, backend 16 CPU / 12 GiB,
  parent 24 CPU / 28 GiB. Verify actual limits and pressure on every fixture.
- Rotate implementation order and repeat fresh/warm phases on immutable data.
  Verify full bytes and listings before treating a phase as comparable.
- Report throughput and p50/p95/p99, daemon/client user/system CPU, whole-cgroup
  CPU and throttling, peak/steady RSS, allocator accounting, cgroup/file/kernel
  memory separately. Missing metrics remain missing. RSS is not cgroup memory;
  final-fifth RSS is not proof of a plateau; wall time is not CPU cost.
- Retain unfavorable samples, dependency failures and incomplete attempts outside
  source control. Instrumented profiles establish scoped observations, not
  acceptance timing or a percentage attribution of global p99.

## Reproduction

Run these inside an isolated Linux/FUSE Kubernetes fixture with an explicitly
owned CPU+memory subtree, disposable storage and the declared Python dependencies:

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

## Current attribution target

Matched initial-read socket/FUSE joins locate extra Rust client work before GET
write and after required socket bytes. Header wait is similar to experimental Go
in that capture; this does not yet identify the responsible scheduler, admission,
SDK or reply stage. Measure those boundaries, inspect applicable upstream issues
and supported APIs, reproduce a concrete defect, then make a narrow correction.
Do not remove If-Match, checksum validation or bounded ownership to match a weaker
reference path. FUSE passthrough and disk-backed caching remain deferred.

Store raw results, traces, provenance and failures in ignored evidence directories.
Source documentation keeps conclusions and reasons, not per-run result tables,
expanded JSON exports, generated flamegraphs or repeated execution journals.
