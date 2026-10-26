# Performance acceptance: UNACCEPTED

No reproducible overall memory reduction or speedup is claimed. One initial
diagnostic pair has measurements; passing owner-budget tests does not establish
bounded RSS or comparative performance acceptance.

## Runner and reproduction

`bench/workloads.json`, `bench/compare.py` and `scripts/bench_worker.py` build the
current Rust release and both exact public Go revisions, preserve executable
hashes and own disposable data/mounts. The matrix has five interleaved repetitions,
two baselines, cold/warm states and seven workloads (280 results). Limits and
actual mount arguments are recorded; Rust samples also report effective payload,
cache, queue, metadata-task and flush budgets.

With an already delegated CPU+memory subtree:

```bash
export GEESEFS_BENCH_CGROUP_ROOT=/path/to/owned/delegated/subtree
timeout --kill-after=30s 7240s python3 bench/compare.py \
  --go-reference "$GEESEFS_GO_REFERENCE" --workloads bench/workloads.json \
  --output benchmark-results/private/full --timeout 7200
```

Alternatively a local systemd user manager can create a dedicated owned unit:

```bash
timeout --kill-after=30s 7300s systemd-run --user --no-ask-password \
  --unit="geesefs-rs-bench-$(date -u +%Y%m%dT%H%M%S)" \
  --property="Delegate=cpu memory" --property=RuntimeMaxSec=7260s \
  --collect --wait --pipe --working-directory="$PWD" \
  --setenv="PATH=$PATH" python3 scripts/with_owned_cgroup.py \
  python3 bench/compare.py --go-reference "$GEESEFS_GO_REFERENCE" \
  --workloads bench/workloads.json --output benchmark-results/private/full \
  --timeout 7200
```

The helper verifies its dedicated unit and sole-process ownership, moves only
itself into its runner child, and enables only the explicitly delegated unit's
controllers. It never changes the hosting service/ancestor. A preflight and an
actual diagnostic run confirmed this path locally; the earlier missing-subtree
preflight remains historical evidence, no longer a current environment blocker.

## Initial diagnostic evidence

The first owned-cgroup smoke run on 2026-10-26 produced two correct cold metadata
results for 1,024 entries, 64 MiB configured payload memory, entry limit 100,000,
one CPU quota and a disposable standard-listing S3 backend:

| Metric | Experimental Go | Rust before bounded metadata concurrency |
| --- | ---: | ---: |
| Traversal throughput | 5,697.55 ops/s | 678.72 ops/s |
| Elapsed | 0.1797 s | 1.5087 s |
| Sampled combined daemon/client peak RSS | 103,170,048 B | 73,027,584 B |
| Sampled combined steady RSS | 102,404,096 B | 72,668,160 B |
| Sampled cgroup peak | 56,635,392 B | 46,170,112 B |

This is **one pair**, not a complete matrix or proof of sustained improvement.
Rust was substantially slower. Standard-LIST concrete-type resolution did serial
HEAD requests; [bounded concurrent resolution and FIFO pressure fixes](bounded-metadata-pressure.md)
followed this measurement. No post-change improvement follows merely from those
tests. Both Go source revisions were built and hashed; only the experimental
baseline participated in this initial pair. Raw original receipts are retained
under ignored `benchmark-results/private/paired-smoke-20261026-v1/`.

## Accounting and acceptance boundaries

- RSS includes daemon and fresh client, with separate PSS traces. Its peak is
  sampled every 20 ms during active work, not an assertion of an exact instantaneous
  maximum. Current cgroup peak uses kernel `memory.peak` for the owned mount lifetime
  including startup/warmup; totals, file cache and kernel memory remain separate.
  The initial pair above predates that exact cgroup-peak correction.
- Warmup executes inside the measured client cgroup. Rust jemalloc samples are
  restricted to the active workload interval; public safe allocator APIs are used
  in the normal and benchmark binary. Allocator numbers exclude other native
  allocators/kernel buffers and are not equivalent to whole RSS.
- Go currently supplies an end pprof snapshot, collected outside active timing
  without `gc=1`. Peak and steady heap time-series values remain missing and fail
  the gate; an end snapshot is never relabeled as either.
- Full compatibility receipts, actual-service PATCH/IAM/encryption, complete CLI
  parity, imported multipart layouts and mounted failure cases remain unresolved.
  The gate requires these receipts, complete paired results and unchanged effective
  budgets. Eleven Python cases exercise evidence rejection, not real performance.
- `--smoke` and `--allow-incomplete-accounting` cannot pass acceptance. Preserve
  unfavorable samples/errors; do not lower caches, change durability, omit metadata,
  force GC or invent a percentage threshold. Private receipts/credentials are not
  shared source artifacts.
