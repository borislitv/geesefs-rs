# GeeseFS Rust rewrite: authoritative project brief

## User request

Implement a Rust replacement for GeeseFS in the separate repository
`borislitv/geesefs-rs`. BMAD was initially required; the user subsequently made
its use optional. Keep concise history/reasons in source; retain detailed
verification evidence in ignored result directories. The primary
constraint is better memory behavior and faster operation than the Go reference,
with preserved required behavior. Always look for suitable ready-made Rust
crates before implementing a component; in particular, reuse an existing FUSE
implementation.

The prior Go optimization experiment is historical evidence, excluded from
acceptance comparisons by the operator on 2026-11-02. Its upstream PR239 was
closed without merge; native metadata/conditional-response checks also expose
unequal behavior. Preserve its raw measurements. This does not imply that Go
cannot be optimized or that Rust automatically improves memory or speed.

## Target and reference

- Initial target: Linux, S3-compatible storage, and the extended metadata used by
  Yandex S3; both filesystem readers and writers are required.
- Deployment clarification, 2026-11-01: the primary target is an LXC container
  serving nginx, rsync and golist. Kubernetes/containerd is not a prerequisite;
  actual container confinement and host-kernel support must be checked in LXC.
  Prefer official upstream libraries over forks, including at a modest speed cost.
- Priority clarification, 2026-11-02: read-only serving is the first performance
  target. Preserve writer correctness; prioritize read, metadata and cache work.
- Primary Go reference: `yandex-cloud/geesefs` release 0.43.9, source
  `80c214ef14f620d93407454fc76ca01dd71fa25f`. Preserve exact source/build hashes
  and distinguish rebuilt benchmark executables from installed readers. The
  experimental revision `5c0884c9e8799bbab93e2d4102fdc15e9c079e0d` remains
  archived evidence, not an acceptance candidate.
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
   and variance. Include daemon/client user/system CPU, total cgroup CPU,
   normalized CPU cost and throttling independently of elapsed time. Preserve
   failures and original measurements; changed object generations invalidate
   equivalent-workload acceptance.
5. The implementation demonstrates lower memory use and faster operation across
   the agreed representative workloads during normal, sustained operation of a
   long-lived mount, including throughput, p50/p95/p99 latency and CPU cost.
   Cold startup/listing cost remains reported separately; winning cold start is
   not required by the operator's 2026-10-29 clarification. Preserve adverse
   measurements without treating a cold-only regression as a steady-state gate.
   Do not weaken correctness/durability, invent acceptance percentages or mark
   the performance gate passed before representative measurements exist.
6. Reviewable source, dependency lockfiles, meaningful tests, benchmark tooling,
   reproduction commands and explicit remaining gaps are committed to a task
   branch and delivered through a draft PR. Production acceptance is separate.

Operator clarification, 2026-11-01: Go-relative superiority is required only for
correctness-equivalent behavior. Where a reference is demonstrably logically
weaker, such as missing a permission-only change at an unchanged content ETag,
Rust need not outrun that weaker operation. Preserve raw rates and resource
measurements, document the specific semantic difference and distinguish necessary
work from avoidable overhead. Do not exempt unrelated payload, writer or memory
losses without evidence, or claim a joint pass from this clarification alone.

Operator clarification, 2026-11-02: replace strict improvement in every metric of
every pair with [paired statistical screening](performance-methodology.md).
Require at least eight independently mounted pairs and an actual same-executable
A/A arm under the same workload/resources. Resample mounts, not individual calls.
Declare target superiority separately from A/A-noise-relative non-inferiority;
publish all effects, confidence intervals and inconclusive results. Historical
four-pair experiments cannot be promoted to acceptance by reanalysis. An A/A
noise allowance is not a practical production regression budget or a substitute
for sustained correctness-equivalent target-environment acceptance.

Operator cache clarification, 2026-11-02: parity with Go 0.43.9 is the primary
cache contract. Bounded negative caching under the same configured metadata TTL
is accepted; do not require immediate external-create visibility beyond Go's
behavior. Use the existing explicit Rust negative-cache configuration in read-only
comparisons. More granular cache TTL controls are future work, not a prerequisite.

## Working boundaries

Use dedicated disposable fixtures for writes and failure injection. Existing
readers, mounts, data, writer schedules and credentials are not part of this
general development workflow. On 2026-11-02 the operator explicitly authorized one
[read-only serving trial](pr-context.md#scoped-serving-trial--2026-11-02), retaining
traffic and existing cache limits. On 2026-11-03 a separate authorization extended
the trial to a second read-only reader with the same source-matching executable
and budgets. These exceptions do not authorize broader rollout or production
failure injection, and do not close acceptance. The user confirmed
the public repository: source and reproducible fixtures are public; credentials,
deployment identities and private operational evidence are not project artifacts.

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

## Current diagnostic priority

Attribute substantial CPU costs and read waits through profiles, FUSE/scheduler
events and controlled comparisons before another runtime fix. Keep instrumented
timings separate from acceptance. Higher CPU allocations are allowed when applied
equally to Rust and Go 0.43.9; report actual filesystem/client, backend and parent
limits, workers and pressure counters. Resource sensitivity is not a source fix.

The [listing diagnosis](performance-listing-diagnosis.md) attributes the default
loss to repeated absent-name probes. Eight controlled blocks verify the existing
explicit negative cache; use 64 misses per directory and one-minute stat TTL.
The default stays zero and five seconds was only a separate expiry probe.

The earlier frozen Rust executable lacked cross-UID wiring. Current explicit
`allow_other` uses the supported fuser access policy, with owner-only and file-mode
controls in a [public native fixture](testing.md#cross-uid-mount-access--2026-11-02).
Target service/profile compatibility and large-namespace memory remain open.
Observed reader memory is anonymous; the absent existing listener still blocks
live-heap-owner attribution. Small fixtures do not establish a whole-process bound.

[Architecture](architecture.md) describes retained corrections, and
[history](change-history.md) records their confirmed causes. Frozen
[measurement pages](performance.md) preserve exact-source tables and limitations;
[next work](next-work.md) lists actionable gaps. Short controls, missing/noisy
observations and a partial native-S3 run do not establish sustained acceptance.
FUSE io_uring remains an upstream watch; passthrough and disk-backed cache are
deferred. No single profile proves that all optimization opportunities are exhausted.
