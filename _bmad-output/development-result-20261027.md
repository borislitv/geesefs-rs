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

- Format check, warning-denying all-target/all-feature Clippy and all 130 Rust
  tests passed, none ignored. The Rust evidence wrapper runs 12 Python benchmark
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

## Remaining work and decision

Compatibility and comparative performance are **UNACCEPTED**. Delivery is a draft
for review, not a complete port or permission to replace a served mount. Reduce
the remaining tail latency and metadata cost, resolve compatibility gaps and Go
heap time-series accounting, then obtain the complete equivalent repeated matrix.
Do not infer speed or memory acceptance from language choice, unit tests or a
small fixture's request count. Production was not modified.
