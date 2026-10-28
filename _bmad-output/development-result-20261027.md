# Development result: experimental implementation, acceptance incomplete

## Delivered scope

The task branch contains a Linux/S3 foreground read/write implementation using
public fuser, AWS SDK, Tokio, maintained transport/parser/cache/allocator crates
and bounded application-owned inode/dirty-generation state. It includes typed
supported configuration, paged metadata/namespace operations, conditional reads,
sparse/truncate/fsync writes, streamed multipart copy/upload and PATCH adapters.
Unsupported options reject explicitly. No internal source or dependencies are
included; every package target forbids unsafe code.

The principal implementation commit is
`830d7a41d4dc17d0f66ee50bf7ed0b827806512e`; the deterministic adaptive-read repair
and diagnostic tooling are in `980949072c02e786c213c227a4ff22122984cec7`. The
subsequent actual-FUSE overlapping-fill ownership repair is measured at
`572195abfd37db14dc13b23b33708ddfdf4d662c`.

## Verification and measured limits

- Format check, warning-denying all-target/all-feature Clippy and all 157 Rust
  tests passed in isolated Kubernetes, none ignored. The Rust evidence wrapper
  runs 23 Python benchmark
  cases; five additional Python provenance/oracle cases passed.
- Public-origin audit: 336 Cargo packages, no private/path sources or missing
  license metadata/file. Full binary-distribution notice aggregation remains open.
- Independent review covered behavior, edge cases, verification gaps and intent;
  deduplicated findings and failure-sensitive regressions are recorded in the
  review documents. A repaired-path static follow-up found no further confirmed
  consequential defect within that scope; this is not whole-project acceptance.
- Real disposable paired mounts exercised full/Range/mmap, rsync full/delta,
  metadata, sparse/truncate, rename and writer/fsync. Both hardlink-emulation
  probes failed; the Go reference also failed bounded fsync recovery. Actual-service
  optional modes, imported PATCH layouts and complete CLI compatibility remain open.
- The 5,400-second matrix attempt stopped at 42/280 results. Completed groups
  included substantial Rust metadata and sequential regressions. The subsequent
  1,200-second sequential diagnostic after the adaptive fix stopped at five results
  and still showed approximately 404 versus 3.5 seconds for 256 MiB. All original
  evidence and unfavorable results are retained.
- Actual mounted tracing then identified overlapping read fills evicting each
  other's cache and publishing into an orphaned policy. The ownership repair
  completed all 80 selected sequential/eviction measurements and one concurrent
  pair, all byte-correct. Mean sequential/eviction time is now 3.087–3.375 seconds
  for Rust; all group mean throughput and RSS/cgroup memory favor Rust. P95
  remains worse and p99 is mixed, so this is not overall acceptance. See
  `docs/performance.md` and its complete reviewed metric summaries.

## Later metadata, CPU and ub-mode checkpoints

- `ba405d3` removes the redundant clean-read materialization buffer. The selected
  eighty-result local matrix remains byte-correct but retains cold-tail failures.
  The subsequent eighteen-phase real-object diagnostic passed all file hashes,
  while Rust fresh throughput and p99 lost to Go. These failures remain visible.
- `ba9ba7d` adds bounded directory reuse/metadata preload with fourteen new public
  regressions and guard mutations. A whole-filesystem improvement is unproven.
- `daa164a` adds independently scoped process/cgroup CPU accounting and correct
  executable Go compiler provenance. CPU regressions are fail-closed; no missing
  historic CPU counters are inferred from wall time.
- `ea11229` applies the requested ub-mode checks and adds adjacent read-window
  coverage. Two surviving strict-boundary mutants are now killed; direct fault
  runs prove assertion-level failure sensitivity. Complexity/duplication findings
  remain open and coverage/CRAP are not claimed. See the ub-mode execution log.
- The full fifty-phase CPU diagnostic completed but lost source equivalence:
  214 objects returned final native 404, 5,522 file checks failed, and 146 listing
  mismatches were retained. All 200 CPU windows and 483 indexed evidence hashes
  were verified. The failed comparison cannot establish CPU/performance wins.
  Six EAGAIN failures remain distinct from remotely deleted data. See
  docs/k8s-cpu-results-20261027.md and the whitelisted JSON.
- The subsequent metadata-coherence follow-up reproduces unrelated-name EAGAIN
  through public lookup/getattr calls, fixes affected-binding publication and
  preserves move/delete and dirty-descendant ownership. Ten additional behavior
  tests, twelve assertion-level guard faults, two selected tool mutants, restored
  full checks and a native release build passed in isolated Kubernetes. The six
  historical mounted failures still require attribution and a valid paired rerun;
  no post-fix comparative performance win is claimed. See docs/metadata-coherence.md.

## Remaining work and decision

The sealed native S3 follow-up completes 40 additional equivalent read phases and
120,000 full-byte/metadata checks. Native generations and exact listings are stable;
280 CPU windows and every RSS/cgroup summary are independently checked. Plain
standard-listing Go metadata failures remain retained; the passing additional
workload explicitly reads xattrs and revalidates attributes for every implementation.
Rust still loses throughput, p99, cold listing and CPU while using lower daemon RSS.
Owned perf/syscall diagnostics and a three-repeat unchanged-binary allocator A/B
preserve a partially confirmed allocation cost, not a completed performance fix.
See docs/immutable-read-results-20261027.md and docs/immutable-read-dataset.md.

The charged-view, requested-reply and body-prefix checkpoints retain their
separate matched repeats. The subsequent cache-lookup repair at
`3b94a76abc422f6e944c9e596fa971cf19b03858` completes another forty phases with
120,000 full file checks, unchanged native data, 280 recomputed CPU windows and
460 verified artifacts. Mean throughput/CPU/p99 improve against exact body-prefix
source, but warm repetitions are mixed and RSS rises slightly; Go still wins
speed/CPU/tails. Three separate profiles pass 9,000 further file checks and native
verification while showing reduced selected BTree/cache leaf shares, not a whole
performance fix. See docs/cached-fragment-lookup.md and its numeric/quality/profile
receipts.

The synchronous borrowed-reply checkpoint at
`906eb45cef9a015c8725bda189b3431d7c6b7d7b` removes a second miss-output allocation
without letting escaping replies pin large GET owners. All 181 Rust cases,
37 Python cases, ten independent faults, five selected mutants, full checks and
the native release pass. Its forty-phase equivalent repeat passes all 120,000
file checks but shows no speed/RSS win against cache prior: throughput declines
0.86% fresh / 0.48% warm, fresh p99 worsens 5.48%, RSS rises slightly. All 460
artifacts, 280 CPU windows, public summaries and paired counts are checked. See
docs/borrowed-read-reply.md; no performance acceptance is inferred from the local
allocation reduction.

Compatibility and comparative performance are **UNACCEPTED**. Delivery is a draft
for review, not a complete port or permission to replace a served mount. Reduce
the remaining tail latency and metadata cost, resolve compatibility gaps and Go
heap time-series accounting, then obtain the complete equivalent repeated matrix.
Do not infer speed or memory acceptance from language choice, unit tests or a
small fixture's request count. Production was not modified.
