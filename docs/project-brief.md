# GeeseFS Rust rewrite: authoritative project brief

## User request

Implement a Rust replacement for GeeseFS in the separate repository
`borislitv/geesefs-rs`. BMAD was initially required; the user subsequently made
its use optional. Preserve the plan and verification evidence. The primary
constraint is better memory behavior and faster operation than the Go reference,
with preserved required behavior. Always look for suitable ready-made Rust
crates before implementing a component; in particular, reuse an existing FUSE
implementation.

The prior Go optimization experiment is retained as a reference, not accepted
as a sustained whole-process memory fix. Its draft upstream PR239 was closed
at the author's request, without merge or branch deletion. This does not imply
that Go cannot be optimized or that Rust automatically improves memory or speed.

## Target and reference

- Initial target: Linux, S3-compatible storage, and the extended metadata used by
  Yandex S3; both filesystem readers and writers are required.
- Go reference: `yandex-cloud/geesefs`, including the reproducible experimental
  revision `5c0884c9e8799bbab93e2d4102fdc15e9c079e0d` and an identified upstream
  packaged baseline. Preserve exact source/build hashes with benchmark results.
- Local reference checkout is supplied by the operator to the BMAD invocation;
  do not depend on a developer-specific absolute path in source code.
- Compatibility must be derived from source/tests, including both supported
  behavior and documented POSIX limitations. Explicitly inventory other OSes,
  Azure backends and optional features instead of claiming unsupported parity.
- Preserve license notices for any reused or translated upstream code.

## Acceptance gates

1. A documented compatibility matrix covers mount/CLI configuration, lookup and
   directory pagination, extended S3 metadata, permissions/xattrs, symlinks,
   mtime, Range/EOF, read-after-write, truncate, rename/delete, multipart writes,
   fsync durability/failure propagation and relevant client behavior.
2. A dependency decision record precedes custom infrastructure: evaluate existing
   crates against the real requirements, select supported implementations and
   justify any narrow missing adapter or custom component.
3. Independent full-byte and metadata checks pass for rsync full/delta and writer
   paths, nginx full/Range-style reads and error/cancellation/retry scenarios.
   Passing unit tests alone is not full FUSE/S3 interoperability acceptance.
4. Paired Go/Rust workloads hold data, budgets, hardware, concurrency and cache
   state constant. Report peak/steady RSS and cgroup/heap/kernel cache separately,
   payload and metadata throughput, and p50/p95/p99 latencies with repetition
   and variance. Preserve failures and original measurements.
5. The implementation demonstrates lower memory use and faster operation across
   the agreed representative workloads without hiding regressions or weakening
   correctness/durability. Do not invent acceptance percentages or mark the
   performance gate passed before measurements exist.
6. Reviewable source, dependency lockfiles, meaningful tests, benchmark tooling,
   reproduction commands and explicit remaining gaps are committed to a task
   branch and delivered through a draft PR. Production acceptance is separate.

## Working boundaries

Use dedicated disposable fixtures for writes and failure injection. Existing
readers, mounts, data, writer schedules and credentials are not part of this
development change. The user changed the repository to public and confirmed
that choice. Source, plans and reproducible test/benchmark artifacts are public;
credentials and private operational evidence are not project artifacts.

Use public upstream code and public crates only. Do not export internal Arcadia
libraries, translated internal code or internal-only dependencies without
explicit authorization. Preserve public source/license provenance. Keep the
implementation in the GeeseFS style: direct filesystem/S3 operations, explicit
buffer ownership and minimal abstraction, expressed idiomatically in Rust.
The user additionally requires no unsafe code in this package. Enforce the
restriction at compilation for every target and use safe dependency APIs.

All implementation work follows the repository's `AGENTS.md`, especially the
performance gate, public-source boundary and mandatory reuse-first rule. Use
finite command/external-operation timeouts and noninteractive SSH without
questions or authentication prompts.
