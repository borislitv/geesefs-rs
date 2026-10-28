# GeeseFS Rust implementation

## Required development workflow

- BMAD is optional by the user's updated instruction. Use its installed workflow
  where useful, retaining the existing plan, verification evidence and real task
  state; do not let orchestration replace implementation or verification.
- A delegated coding subagent executes its supplied plan and must not start a
  recursive build-auto run. The parent owns final review and VCS delivery.
- Use explicit finite timeouts for commands, subprocesses and external operations.
  SSH must be noninteractive (BatchMode, connection/operation deadlines); do not
  prompt the user about SSH or wait for interactive authentication.
- Read `docs/project-brief.md` before planning, implementation or review.
- Follow the requested [ub-mode](https://github.com/unclebob/ub-mode) workflow:
  understand the contract, choose the smallest cohesive change, observe a
  meaningful failing test, implement, verify and only then refactor. Keep an
  action/reason log and report quality-tool findings with their actual scope;
  tool success does not replace compatibility or performance acceptance.
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
  Include daemon/client user/system CPU, whole-cgroup CPU, normalized work cost
  and throttling; elapsed wall time is not CPU consumed.
  Keep peak/steady RSS visible in performance checkpoint tables alongside CPU;
  publish cgroup/file/kernel memory and label process scope and missing metrics.
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
- Maximize use of the selected libraries' mature buffer, streaming, cache and
  ownership APIs. Inspect those APIs before adding a wrapper or custom mechanism;
  remove redundant application allocations/copies instead of replacing libraries.

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

## Public-source provenance and implementation style

- Forbid unsafe code in this package, including binaries and tests, through
  Cargo's `unsafe_code = "forbid"` lint. Use safe maintained crate APIs for OS
  and allocator operations; do not introduce local unsafe blocks or FFI shims.
- Only use independently public upstream sources and public Rust crates with
  compatible licenses. Do not copy, translate, vendor or link internal Arcadia
  libraries, generated code or internal-only dependencies into this repository.
  Any proposed exception requires explicit authorization before inclusion.
- Record the public origin and license for reused or translated source. Audit
  Cargo manifests, lockfiles, vendored files, generated artifacts and CI for
  internal registries, paths, hosts or unpublished source before pushing.
- Follow the GeeseFS implementation style: direct filesystem/S3 operations,
  explicit buffers and ownership, and small cohesive modules. Use idiomatic Rust;
  avoid unnecessary frameworks, abstraction layers and speculative generality.
