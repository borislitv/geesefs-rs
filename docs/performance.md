# Performance acceptance: UNACCEPTED

The complete healthy-warm cache-policy comparison and persistent mixed-mount
observation are summarized in the [README](../README.md). Repeated warm-random
GETs disappear, but tails, concurrent reads, sustained rate and memory still lose
selected gates against Go. The mixed run establishes no memory plateau.
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
  time. The latest README uses the latter for throughput, but still reports mean
  run percentiles. Rare 30-second successful reads make the distinction material:
  candidate sequential mean rate is 238.84 MiB/s, whereas total-work/time is 115.93.
  Preserve both, all repetitions, raw outliers and independently pooled tails.
- Retain unfavorable samples, dependency failures and incomplete attempts outside
  source control. Instrumented profiles establish scoped observations, not
  acceptance timing or a percentage attribution of global p99.

## Reproduction

The release profile uses `lto = "thin"` and `codegen-units = 1`; retain these
settings with executable provenance. Linking takes longer. The matched profile
comparison uses the explicit System allocator; default builds still use jemalloc.
Do not generalize one allocator/workload's gains to every configuration.

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

The new comparison retains five rare approximately 30-second successful serial
reads in candidate Rust and twelve in its predecessor, with none in Go. The
persistent run retains eleven and sixteen respectively. P99 alone does not show
these rare stalls. Filesystem/client I/O PSI-some is substantial despite zero
quota throttling, memory pressure and reclaim; it is not proof of physical backend
disk pressure. Correlate FUSE/SDK/body waits and allocation ownership before a
runtime correction. `Filesystem::admit` has a 30-second admission timeout, but a
matching duration alone does not establish it as the cause.

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
