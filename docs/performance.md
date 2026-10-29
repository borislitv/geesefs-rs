# Performance acceptance: UNACCEPTED

The [bounded cache-hit refill candidate](read-ahead-refill.md) passes its
failure-sensitive behavior and full library checks. Its matched native
performance verification is in progress; the measurements below describe
the frozen prior implementation rather than an unmeasured candidate win.

The latest [read-ahead refill diagnosis](read-ahead-refill-causality.md) confirms
that Rust's cache-hit early return postpones the next window's GET until a demand
miss. Direct socket/FUSE joins and selective noninitial-GET delay independently
establish avoidable refill waits. The [numeric receipt](read-ahead-refill-causality-20261029.json)
retains 72 controlled phases / 216,000 full-file checks and separate network
profiles / 18,000 checks, including backend-worker controls and explicit
incomplete-data limits. This is source-level causal attribution, not a runtime
repair, an estimate of every p99 cause or full performance acceptance.

The preceding [steady-state diagnosis](steady-state-tail-diagnosis.md) and
[numeric receipt](steady-state-headroom-results-20261029.json) supersede the
historical measurements below for current resource-sensitivity findings. They
retain 216 unprofiled phases, 648,000 full-file checks and eighteen separately
scoped [flamegraphs](flamegraphs-20261029/README.md). Expanded CPU/memory removes
quota throttling and filesystem/client reclaim. Rust's throughput and process
memory improve relative to both Go references, but p95/p99 remain worse with
eight runtime workers. No overall acceptance or runtime-code improvement is
claimed. Cold startup is reported separately and is no longer a required win.

No overall memory/speed acceptance is claimed. Selected local sequential/eviction
matrices complete correctly with higher group mean throughput, but cold tail
regressions remain. A separate real-object Kubernetes diagnostic completes all
18 phases correctly and shows **lower Rust throughput** than both Go references.
Lower daemon RSS is not a whole-cgroup memory win under file-cache pressure.
The full matrix and compatibility remain incomplete. Favorable and unfavorable
attempts are retained below; CPU cost is now measured separately from wall time.

The subsequent [sealed native S3 checkpoint](immutable-read-results-20261027.md)
completes 40 equivalent explicit-metadata phases: 120,000 complete file checks,
unchanged generations, exact listings and 280 independently recomputed CPU windows.
Rust still loses throughput, cold listing time, p99 and CPU while retaining lower
daemon RSS. The original plain standard-listing Go metadata failures remain
separate. Profiles, syscall counts and an unchanged-binary allocator A/B identify
partial allocation cost without claiming that tuning alone fixes the regressions.

The [matched charged-view comparison](cached-read-views.md#matched-native-comparison)
also completes forty phases and 120,000 full file checks. Its exact-copy baseline
is retained in the same run. Views modestly improve group mean throughput/p99
against that copy binary, but still lose to Go on throughput, CPU and p99; steady
RSS and system CPU are not uniformly improved. The numeric receipt retains both
RSS and CPU with allocator, client, cgroup and separate native-backend scope.

The [requested-range reply comparison](requested-read-reply.md#matched-native-result)
adds forty more correct phases. Its behavior fix lets a requested result precede
unrelated parallel read-ahead, but it does not resolve throughput, CPU, p99 or
whole-memory losses. The separate
[complete-GET buffering follow-up](streamed-read-body.md) has failure-sensitive
real HTTP and filesystem prefix tests. Its forty-phase matched repeat is
byte-correct and reduces p99 versus the exact prior Rust, but throughput, CPU,
steady RSS and p50/p95 regress. Both Go references still win throughput, CPU and
p99. The complete result is retained as a tradeoff, not overall acceptance.

The [cached-fragment lookup repeat](cached-fragment-lookup.md) improves mean
throughput/CPU/p99 against that exact body-prefix prior, with mixed warm pairs
and slightly higher RSS. The subsequent
[borrowed miss-reply](borrowed-read-reply.md) and
[initialized-fragment](initialized-read-fragment.md) repeats each add forty
equivalent phases and 120,000 passing file checks. Reusing existing immutable
views and Tokio copy_buf/take removes redundant application copy/zeroing, but
neither repeat demonstrates a reproducible overall speed/CPU/RSS/tail win.
Both Go references remain faster. All 920 indexed artifacts, 560 CPU windows,
both sets of compiled-input commits and the adverse paired results are retained.

The [range-index comparison](range-index-coverage.md) adds forty phases and
120,000 full-file checks against exact initialized prior and both Go references.
Fresh throughput/whole CPU/p99 means improve slightly, but warm throughput falls
3.72%, CPU rises 4.76% and p99 rises 14.32%; all dispersion and unfavorable pairs
remain published. Separate absolute profiles confirm reduced covering/Moka work,
not an overall win: Rust page faults and kernel copies remain, and profile timings
are not acceptance repetitions. See [absolute attribution](read-cost-attribution.md)
and both numeric receipts. Passthrough/disk caching are outside the first stage.

The [consumed receive-buffer fix](receive-buffer-reuse.md) uses maintained
futures/Tokio adapters after a failure-sensitive ownership regression. Forty
matched phases verify 120,000 files and improve throughput 4.96% fresh / 7.22%
warm, whole CPU 5.17% / 6.29% and p99 4.81% / 11.25% against exact index prior.
Final-fifth RSS rises 1.98% / 4.67%; warm p99 remains worse than both Go references.
Nine separate profiles confirm lower TCP/page faults and expose admitted-fragment
allocation as the next actor. This is measured progress, not overall acceptance.

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

The original Go 1.22.2 field identified the launcher, not the auto-selected build
compiler. Retained exact-hash binaries establish **go1.25.9**; see the
[hash-bound correction for all 12 retained Go executables](go-toolchain-correction-20261027.json).
Rust used rustc 1.99.1. Original per-run records,
memory/allocator traces and errors remain in the ignored
`benchmark-results/private/paired-full-20261026-v1/`. The dedicated unit stopped
and no owned FUSE mounts remained after the deadline.

`bench/summarize.py REPORT` includes every recorded run, explicit missing metrics,
counts, mean/min/max/dispersion and a whitelist of executable provenance.
`--table` gives a compact view. A targeted repeat can use
`--diagnostic-workload sequential` with the same full workload configuration and
unchanged limits: both baselines, states and all five repetitions still run for
that workload. Diagnostic selection never passes the full acceptance gate.

## After the adaptive-history fix only

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
mounted throughput regression remained at that source. Correcting the deterministic
adaptive history did not resolve it; its cause was not yet established at that
point. The subsequent actual FUSE diagnosis and measured repair follow below.
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

## After the overlapping-fill ownership repair

An actual mounted trace identified simultaneous neighboring FUSE reads repeatedly
evicting the active fill's cache policy and publishing into an orphaned clone.
[The repair and failure-sensitive regressions](read-fill-pressure.md) coalesce
overlapping, generation-scoped fills before large admission, retain independent
window concurrency and publish into the current cache. Budgets, workload data,
CPU quota, cache state definitions and integrity checks are unchanged.

At committed source `572195abfd37db14dc13b23b33708ddfdf4d662c`, the selected
sequential/eviction experiment completed **all 80 results**: two public baselines,
cold/warm states and five alternating repetitions per implementation/workload.
All 80 passed the full-byte checksum. It completed inside its 1,200-second
deadline (owned-unit lifetime about 537 seconds). All eight group mean throughputs
favor Rust, and all group mean peak/steady RSS and cgroup values are lower.
These are means, not a claim that every individual repetition wins.

Throughput is decimal MB/s; `±` is the population standard deviation of the five
run throughputs. Elapsed is the mean run time for the unchanged 256 MiB payload.

| Baseline | State | Workload | Go MB/s ± SD | Rust MB/s ± SD | Go seconds | Rust seconds |
| --- | --- | --- | ---: | ---: | ---: | ---: |
| Experimental | Cold | Sequential | 81.27 ± 12.14 | 86.99 ± 1.55 | 3.390 | 3.087 |
| Experimental | Cold | Eviction | 72.69 ± 6.00 | 80.96 ± 9.77 | 3.718 | 3.375 |
| Experimental | Warm | Sequential | 70.75 ± 11.05 | 85.63 ± 8.06 | 3.877 | 3.164 |
| Experimental | Warm | Eviction | 77.13 ± 11.32 | 80.73 ± 7.13 | 3.568 | 3.353 |
| Packaged | Cold | Sequential | 71.25 ± 8.24 | 83.39 ± 4.05 | 3.817 | 3.227 |
| Packaged | Cold | Eviction | 69.46 ± 12.84 | 85.82 ± 3.26 | 4.013 | 3.133 |
| Packaged | Warm | Sequential | 74.27 ± 12.00 | 81.44 ± 5.27 | 3.717 | 3.310 |
| Packaged | Warm | Eviction | 65.16 ± 8.67 | 87.27 ± 8.13 | 4.190 | 3.105 |

Memory values below are means of per-run combined daemon/client sampled RSS peak
and kernel cgroup lifetime peak, in bytes. Latency values are means of per-run
percentiles, not percentiles pooled across the whole experiment.

| Baseline/state/workload | Go peak RSS | Rust peak RSS | Go cgroup peak | Rust cgroup peak | Go p95 ms | Rust p95 ms | Go p99 ms | Rust p99 ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Experimental/cold/sequential | 222,817,485 | 125,027,942 | 433,440,358 | 355,783,475 | 26.258 | 110.147 | 239.757 | 244.180 |
| Experimental/cold/eviction | 229,411,226 | 129,785,856 | 421,291,622 | 354,363,802 | 20.480 | 111.671 | 235.741 | 244.513 |
| Experimental/warm/sequential | 233,300,787 | 126,402,560 | 422,105,907 | 356,714,906 | 26.787 | 69.265 | 263.235 | 244.781 |
| Experimental/warm/eviction | 241,272,422 | 125,749,658 | 427,796,890 | 353,671,578 | 24.452 | 111.277 | 227.398 | 244.967 |
| Packaged/cold/sequential | 233,218,867 | 133,813,043 | 433,697,587 | 352,287,949 | 26.364 | 111.658 | 255.056 | 245.136 |
| Packaged/cold/eviction | 233,999,565 | 127,257,805 | 432,548,250 | 353,558,528 | 23.393 | 110.321 | 421.805 | 244.378 |
| Packaged/warm/sequential | 227,966,976 | 128,143,360 | 426,196,992 | 351,041,946 | 27.978 | 113.082 | 229.040 | 244.357 |
| Packaged/warm/eviction | 223,955,354 | 131,012,198 | 435,766,067 | 356,604,314 | 25.769 | 47.070 | 311.806 | 244.879 |

**Tail non-regression does not pass.** Mean p50 improves (about 0.42 versus
4.3–4.8 ms), but Rust p95 is 47–113 ms versus Go 20–28 ms; p99 is mixed. The
current miss path still waits for a whole prefetched window. Do not describe
better total time as an across-the-board latency win. Rust steady file cache is
also higher in these groups (about 178–202 million bytes versus 165–173 million
for Go), despite lower cgroup totals. Kernel overhead and allocator counters are
reported separately, not excluded from memory accounting.

The reviewed [80-result summary](read-fill-results-20261027.json) includes every
group, counts, missing metrics, mean/min/max/standard deviation for RSS, cgroup,
file/kernel cache, allocator and latency, plus exact limits and executable
provenance. No mount arguments, credentials or private host/bucket identifiers
are included. Original per-run latencies, logs and traces remain in ignored
`benchmark-results/private/paired-read-fills-20261027-v1/`. The gate is correctly
**UNACCEPTED** with 261 errors: selection is not the full 280-result matrix, peak/
steady Go heap and compatibility receipts are absent, and measured tail/repeated
improvement requirements are not all met.

A separate unchanged-workload concurrent smoke pair also passed byte checks:
Go 211.299 seconds and 39,700 bytes/s versus Rust 196.377 seconds and 42,717
bytes/s, peak RSS 179,347,456 versus 127,680,512 bytes. This is one pair, not a
repeated concurrent acceptance result. Its [complete metric summary](read-fill-concurrent-20261027.json)
and original ignored `paired-read-concurrent-20261027-v1/` receipts are retained;
its gate remains false with 229 errors. Both owned units stopped and no owned
FUSE mounts remained.

Reproduction with the dedicated-unit wrapper above, using a fresh output path:

```bash
python3 bench/compare.py --go-reference "$GEESEFS_GO_REFERENCE" \
  --workloads bench/workloads.json --output benchmark-results/private/read-fills-new \
  --diagnostic-workload sequential --diagnostic-workload eviction --timeout 1200
python3 bench/summarize.py benchmark-results/private/read-fills-new/report.json --table
python3 bench/summarize.py benchmark-results/private/read-fills-new/report.json \
  --table --metrics rss_steady_bytes cgroup_peak_bytes cgroup_steady_bytes p95_ms p99_ms
```

The exporter accepts `--output NEW.json` for a new JSON summary; existing output
is rejected. Review exported data before publication: run error text is retained,
whereas limits and provenance use explicit field lists. The published successful
groups have empty error lists. Whitelist failure sensitivity and no-overwrite
behavior were verified; no raw private receipts were committed.

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
  budgets. Python evidence/CPU tests exercise accounting and rejection, not real performance.
- `--smoke`, diagnostic selection and `--allow-incomplete-accounting` cannot pass acceptance. Preserve
  unfavorable samples/errors; do not lower caches, change durability, omit metadata,
  force GC or invent a percentage threshold. Private receipts/credentials are not
  shared source artifacts.

## Clean-buffer follow-up: local selected matrix

Source `ba405d3485e66b00ad3bacc716177f0f9a145eb6` completed another 80 selected
results with full-byte correctness, the same 256 MiB data, 64 MiB user budget,
100,000 entries, one CPU quota, 1 GiB cgroup and five repetitions. The
[complete whitelisted summary](clean-read-results-20261027.json) retains mean,
dispersion, missing metrics, all memory/tail values and exact executable hashes.

| Baseline/state/workload | Go MB/s | Rust MB/s | Go p95 ms | Rust p95 ms | Go p99 ms | Rust p99 ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Experimental/cold/sequential | 75.82 | 111.66 | 31.286 | 66.759 | 230.279 | 244.827 |
| Experimental/cold/eviction | 66.98 | 110.78 | 28.926 | 25.195 | 232.504 | 254.265 |
| Experimental/warm/sequential | 64.69 | 135.48 | 22.709 | 2.601 | 312.806 | 251.920 |
| Experimental/warm/eviction | 59.28 | 135.92 | 33.342 | 2.693 | 411.770 | 254.471 |
| Packaged/cold/sequential | 68.28 | 110.72 | 24.638 | 46.915 | 253.867 | 248.282 |
| Packaged/cold/eviction | 58.56 | 120.44 | 34.050 | 2.938 | 428.457 | 246.459 |
| Packaged/warm/sequential | 66.28 | 114.59 | 28.724 | 3.197 | 400.545 | 290.191 |
| Packaged/warm/eviction | 62.61 | 117.30 | 37.956 | 25.235 | 283.100 | 268.706 |

Values are means of per-run metrics, not pooled percentiles. Mean Rust combined
daemon/client peak RSS is 129–137 MB versus Go 216–246 MB; cgroup peak is 359–367
MB versus 419–447 MB. Cold sequential p95 still loses against both references;
experimental cold p99 and individual file/kernel cache indicators also regress.
The full gate is false with 248 errors. CPU counters were not captured in this
historical run and must not be inferred from elapsed time.

## Real-object Kubernetes diagnostic

The same frozen Rust source and exact Go executables completed **18/18 phases**:
three order permutations, three implementations and fresh/warm-revisit states.
All phases verified **3,000/3,000 complete file hashes**, 8,298,727,528 bytes per
phase and 96 directory listings, without errors. The native oracle used If-Match
and SHA-256. The [full phase summary](k8s-readonly-results-20261027.json) preserves
the dispersion and every measured memory/latency field without production paths.

Common limits were two CPUs, 4 GiB container cgroup, 512 MiB user cache, 100,000
entries, four readers and 128 KiB reads. Every mount was isolated RO with multipart
expiration disabled. No served mount or data was changed. Fresh is a new mount,
not a controlled cold host/service cache. Warm revisits the complete working set.

| Implementation | State | MB/s, mean [min–max] | Mean read p95 / p99 ms | Mean listing seconds | Mean daemon peak RSS MB |
| --- | --- | ---: | ---: | ---: | ---: |
| Experimental Go | Fresh | 252.64 [239.99–261.87] | 6.955 / 24.048 | 9.859 | 1143.9 |
| Experimental Go | Warm revisit | 300.27 [293.78–304.45] | 6.938 / 22.080 | 0.042 | 1222.2 |
| Packaged Go | Fresh | 227.38 [225.13–229.10] | 7.197 / 25.575 | 10.357 | 1251.0 |
| Packaged Go | Warm revisit | 281.43 [270.56–295.17] | 7.220 / 23.573 | 0.526 | 1377.0 |
| Rust | Fresh | 200.84 [198.44–204.42] | 6.239 / 38.081 | 8.481 | 543.7 |
| Rust | Warm revisit | 265.42 [247.08–280.90] | 6.208 / 34.847 | 3.803 | 667.4 |

Rust fresh throughput and read p99 lose in all three repetitions. Summed per-file
stat time is about 55 worker-seconds versus 18–30 for Go; these sums overlap across
four workers. Repeated READDIR also incurs backend calls. The
[bounded metadata follow-up](metadata-page-reuse.md) tests these paths; its complete
speed benefit is not established. An interrupted next diagnostic retained ten
completed byte-correct phases, including an unfavorable new-Rust fresh result,
before the operator requested CPU accounting from the first phase. Those receipts
remain separate and are not substituted for a completed comparison.

All implementations fill roughly the 4 GiB cgroup limit with reclaimable file
cache. Rust holds more file cache and less anonymous/kernel memory. This diagnostic
cannot establish a strict lower total-memory peak merely from lower daemon RSS.
Unlike the earlier local table, Kubernetes RSS above covers the daemon only;
cgroup totals include client/controller. Samples are 50 ms and earlier mount
cache can remain. No forced GC or global cache drop was used. Go heap series,
writers and full compatibility remain incomplete. Historical CPU cost is missing.

## CPU cost and regression accounting

New windows use psutil 7.2.2 process user/system counters and owned cgroup v2
`cpu.stat` deltas. Reports retain daemon/client CPU seconds, complete cgroup CPU
seconds, CPU seconds per GiB or operation, average occupied cores, and quota
throttling/period counters. Throttled duration is not counted as useful CPU time.
Linux process counters have clock-tick precision; cgroup counters use microseconds.

The local runner measures the active workload after initialization/warmup and
before end pprof. Its owned cgroup includes daemon and client. Kubernetes also
retains whole-phase, listing-only and file-worker windows, plus whole-experiment
cgroup cost including the generation inventories. Its cgroup includes all owned
container work. These scopes are explicit and must not be mixed. Missing counters
or zero denominators stay missing, not zero. Original reports are not rewritten.

The fail-closed gate now requires daemon/client/cgroup CPU totals and checks
daemon/total CPU non-regression as well as wall throughput, memory and latency.
Faster elapsed time does not excuse higher CPU cost. Unit regressions first
rejected absent/greater CPU and missing summary fields on the old tooling;
system-time, microsecond-unit and process-identity mutations also failed. These
checks establish accounting behavior, not a measured CPU win.

## Completed CPU-accounted diagnostic: changed source, invalid comparison

The subsequent five-implementation, five-order fresh/warm matrix executed all
50 phases in 2,113 seconds with daemon/client/cgroup CPU counters from the first
phase. All 200 phase CPU windows were independently checked against retained raw
boundaries. This completes CPU collection, not acceptance: the initial native
inventory passed, but 214 original manifest objects returned native 404 after the
matrix. Only 144,478 of 150,000 attempted full-file checks passed. All 5,522 file
failures, including six separately unresolved EAGAIN failures, and 146 listing
mismatches remain in the evidence. No failed member or phase was filtered out.

Different phases consumed different bytes, so lower CPU totals cannot establish
CPU-efficiency improvement. Post-run native listing also confirmed source
mutation. The [CPU checkpoint](k8s-cpu-results-20261027.md) and
[whitelisted receipt](k8s-cpu-results-20261027.json) publish all five-run groups,
their error counts, CPU scopes, exact provenance and limitations. An immutable or
version-pinned dataset is required before claiming a repeated equivalent-workload
win. Earlier adverse results and the ten-phase CPU-less interruption remain intact.

## Metadata-coherence repair after the invalid comparison

Deterministic Kubernetes tests subsequently reproduced successful lookup/getattr
being rejected after an unrelated cached name disappeared remotely. The
[coherence repair](metadata-coherence.md) validates affected bindings/ownership,
retains necessary negative-directory snapshot checks, and prevents late responses
from discarding cancelled moves or dirty/newly durable descendants. Ten added
behavior tests, twelve independent guard faults and restored full checks provide
correctness evidence. This does not retroactively attribute all six historical
EAGAIN failures, repair changed-source equivalence, or establish a speed/CPU/RSS
win. A new immutable/version-pinned mounted comparison remains required.
