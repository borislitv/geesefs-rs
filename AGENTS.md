# GeeseFS Rust implementation

## Required development workflow

- Use the installed BMAD workflow, with `bmad-build-auto` owning the unattended
  build loop. Follow its rendered instructions and record real task state.
- Read `docs/project-brief.md` before planning, implementation or review.
- The current scope is a Linux/S3 replacement for both GeeseFS readers and
  writers, not an unlabelled read-only prototype or a Go wrapper.
- Keep Go behavior, source revisions, compatibility gaps and performance
  acceptance independently verifiable. Do not describe incomplete work as a
  complete port or measured improvement.

## Primary acceptance constraint

The Rust implementation must use less memory and run faster than the Go
reference on equivalent, reproducible workloads while preserving the required
filesystem and S3 behavior. A language change alone does not satisfy this goal.

- Fix datasets, object generations, machine/kernel, CPU quota, concurrency,
  cache budgets, working-set size and cold/warm state in paired comparisons.
- Measure steady-state and peak RSS, allocator/heap use where available, total
  cgroup memory, kernel cache, throughput and p50/p95/p99 latency separately.
- Include metadata-heavy traversal, sequential/random payload reads, concurrent
  readers, cache eviction and writer/fsync workloads, including dependency errors.
- Require reproducible improvement rather than one favorable sample; retain raw
  measurements, failures, variance and baseline executable provenance.
- Do not lower caches, omit metadata, remove compatibility, change durability,
  force garbage collection or omit clients/cache memory to manufacture a win.
- Correctness is a prerequisite. Until compatibility and performance gates pass,
  keep the implementation and PR explicitly unaccepted for production.

## Mandatory reuse-first rule

Before implementing any infrastructure component, search for maintained Rust
crates and existing implementations. Reuse a suitable crate first. Write a
component only after documenting a concrete unmet requirement in the available
solutions; convenience or unfamiliarity is not sufficient justification.

- FUSE already has implementations: select and integrate an existing FUSE crate;
  do not write a FUSE protocol stack or syscall adapter from scratch.
- Apply the same rule to S3, HTTP/TLS, credentials/signing, async/concurrency,
  XML, caching, CLI, observability and test infrastructure.
- Record evaluated crates, versions, primary-source links, maintenance, license,
  required API/protocol coverage and the selection or rejection reason in
  `docs/dependency-decisions.md` before writing a custom replacement.
- Prefer small adapters around supported APIs. Preserve bounded ownership,
  streaming, cancellation, retries and integrity checks when using dependencies.

## Compatibility and verification

- Establish contracts from the Go source and callers: listing pagination and
  extended metadata, symlinks, mtime, Range/EOF, cache coherence, rename/delete,
  multipart writes, fsync failure propagation and recovery.
- Test public behavior through real filesystem/S3 boundaries where wiring or
  protocol semantics matter. Keep expected bytes and metadata independent of
  the implementation; prove regression-test failure sensitivity.
- Keep data buffers bounded and ownership explicit. Account for queued and
  in-flight work as well as retained cache; do not buffer an unbounded listing
  or whole large object simply to simplify implementation.
- Benchmark correctness-checked workloads and include unchanged-budget Go
  references. Unit tests or synthetic allocator counts alone do not prove a
  whole-filesystem memory/performance improvement.
- Do not deploy, replace served mounts, modify production data or reuse private
  credentials in this development workflow. Integration writes use dedicated
  disposable test data only.
- Keep credentials, private receipts and production identifiers out of source
  and shared artifacts. Preserve upstream licenses and attribution.
