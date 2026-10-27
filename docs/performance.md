# Performance acceptance: UNACCEPTED

No reproducible overall memory reduction or speedup is claimed. An initial
diagnostic pair and an incomplete full-matrix attempt have measurements; passing
owner-budget tests does not establish bounded RSS or comparative acceptance.

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

## Full-matrix attempt before the sequential-read fix

The subsequent attempt at source
`830d7a41d4dc17d0f66ee50bf7ed0b827806512e` exhausted its explicit **5,400-second
deadline** after **42 of 280 results**. All 42 completed runs passed their byte or
metadata checks; the gate still failed with 243 errors, including the deadline,
incomplete matrix, missing compatibility receipts, missing Go peak/steady heap
series and measured regressions. No warm-state, writer or packaged-baseline
comparison completed. The unfinished eviction group is retained, not discarded.

These are all completed groups, with five repetitions per row except eviction
(one). Throughput is operations/s for metadata and bytes/s for other workloads;
dispersion is population standard deviation, not a confidence interval. RSS is
combined daemon/client sampled peak, averaged across repetitions.

| Cold workload | Implementation | Runs | Mean throughput ± SD | Mean seconds | Mean peak RSS, bytes |
| --- | --- | ---: | ---: | ---: | ---: |
| Metadata | Experimental Go | 5 | 6,081.78 ± 373.76 | 0.169 | 102,862,848 |
| Metadata | Rust | 5 | 751.83 ± 5.17 | 1.362 | 76,526,387 |
| Sequential | Experimental Go | 5 | 79,069,139 ± 15,091,192 | 3.547 | 223,090,278 |
| Sequential | Rust | 5 | 635,102 ± 2,452 | 422.671 | 142,893,875 |
| Random | Experimental Go | 5 | 38,667 ± 103 | 54.237 | 167,475,610 |
| Random | Rust | 5 | 42,546 ± 98 | 49.291 | 125,701,325 |
| Concurrent | Experimental Go | 5 | 38,862 ± 1,828 | 216.371 | 181,689,549 |
| Concurrent | Rust | 5 | 42,923 ± 95 | 195.435 | 128,105,677 |
| Eviction | Experimental Go | 1 | 74,564,228 | 3.600 | 223,219,712 |
| Eviction | Rust | 1 | 636,321 | 421.855 | 135,131,136 |

Metadata remained substantially slower despite bounded concurrency. Sequential
and eviction reads exposed a severe adaptive-read regression. Faster random and
concurrent groups and lower sampled RSS do not establish overall acceptance.
The [sequential-read follow-up](sequential-read-window.md) fixes cache-hit tracking
and contiguous-run history; deterministic requests fall from 139 to 5 for its
32 MiB fixture. That request count is not a post-fix mounted speed measurement.

The attempted comparison used the unchanged `bench/workloads.json`: 256 MiB
payload, 64 MiB configured payload budget, 100,000 entries, one CPU quota and
1 GiB cgroup memory maximum. The disposable backend was the local standard-listing
fixture, not an actual cloud-service validation. Provenance:

| Executable | Source revision | SHA-256 |
| --- | --- | --- |
| Experimental Go | `5c0884c9e8799bbab93e2d4102fdc15e9c079e0d` | `01ac0b9f433747661893ecba83a05c2b5d497cdc0dbf208f66937e2551a21218` |
| Packaged Go, built but not reached | `80c214ef14f620d93407454fc76ca01dd71fa25f` | `41fc45dcaf5b672303e4dad4f824f276b9137d1ef9e1676d3a5c1c55c97eb74f` |
| Rust | `830d7a41d4dc17d0f66ee50bf7ed0b827806512e` | `b170731dcf63a1c9ff43ef655195ab3ebec342624c83b3c4df4debc0418947ee` |

Toolchains: Go 1.22.2 linux/amd64 and rustc 1.99.1. Original per-run records,
memory/allocator traces and errors remain in the ignored
`benchmark-results/private/paired-full-20261026-v1/`. The dedicated unit stopped
and no owned FUSE mounts remained after the deadline.

`bench/summarize.py REPORT` includes every recorded run, explicit missing metrics,
counts, mean/min/max/dispersion and a whitelist of executable provenance.
`--table` gives a compact view. A targeted repeat can use
`--diagnostic-workload sequential` with the same full workload configuration and
unchanged limits: both baselines, states and all five repetitions still run for
that workload. Diagnostic selection never passes the full acceptance gate.

## Post-fix bounded sequential diagnostic

The repeat at committed source `980949072c02e786c213c227a4ff22122984cec7`
also exhausted its deadline, this time **1,200 seconds**. It recorded only five
completed cold experimental-baseline runs out of the selected 40; another Rust
run was interrupted and has no completed measurement. The selected workload kept
the same configuration, dataset size, effective memory/entry budgets, CPU quota
and correctness oracle. All five completed checks passed. The full gate remains
**UNACCEPTED**, with 234 errors and no complete repeated paired group.

| Implementation | Completed runs | Mean throughput ± SD, bytes/s | Mean seconds | Mean sampled peak RSS, bytes |
| --- | ---: | ---: | ---: | ---: |
| Experimental Go | 3 | 81,979,201 ± 20,523,960 | 3.496 | 220,396,203 |
| Rust after adaptive fix | 2 | 664,906 ± 14,319 | 403.906 | 138,784,768 |

These are **all completed results**, including the unmatched third Go run, not
a five-repetition paired comparison or a before/after causal estimate. The severe
mounted throughput regression remains. Correcting the deterministic adaptive
history defect did not resolve it; its remaining cause is not yet established.
No warm, packaged-baseline or writer result was obtained in this diagnostic.
The owned unit is inactive and owned FUSE mounts were removed.

Executable provenance, with the same toolchains as the preceding attempt:

| Executable | Source revision | SHA-256 |
| --- | --- | --- |
| Experimental Go | `5c0884c9e8799bbab93e2d4102fdc15e9c079e0d` | `7d7c1072ce0dacddba59f384e25f696254e3d10ba822d7b47121964cf4992729` |
| Packaged Go, built but not reached | `80c214ef14f620d93407454fc76ca01dd71fa25f` | `6b3ae76b4c494e38fa2586d0bdd0e1d53c2a9f4ce5a523264fbee3d6062b6622` |
| Rust | `980949072c02e786c213c227a4ff22122984cec7` | `511468a8d05352c8692ea26aa8b9956f7f93aedefa00a195d50a39cd3267dfd7` |

Original records and partial-run traces remain in the ignored
`benchmark-results/private/paired-sequential-20261027-v1/`. Preserve them alongside
the earlier attempt. Further performance work must explain the mounted behavior
before repeating a long matrix; library request counts cannot substitute for it.

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
- `--smoke`, diagnostic selection and `--allow-incomplete-accounting` cannot pass acceptance. Preserve
  unfavorable samples/errors; do not lower caches, change durability, omit metadata,
  force GC or invent a percentage threshold. Private receipts/credentials are not
  shared source artifacts.
