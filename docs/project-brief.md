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

## Current diagnostic priority

Attribute substantial CPU costs and read waits through CPU/off-CPU profiles,
FUSE/scheduler events and controlled comparisons before another runtime fix.
Keep instrumented timings separate from acceptance measurements. The operator
permits testing higher CPU allocations: apply each allocation equally to Rust and
Go 0.43.9, report filesystem/client, backend and parent-group limits,
and keep worker counts and memory budgets explicit. An improvement after adding
CPU is a resource-sensitivity result, not evidence of a source-code optimization.

The latest [listing and reader-memory diagnosis](performance-listing-diagnosis.md)
attributes the large golist loss to repeated absent-name S3 probes. The existing
explicit negative-cache option removes that cost in eight controlled blocks.
Its observed external-create delay is accepted Go-parity cache behavior. Both CLI
defaults and the main comparison use a one-minute metadata TTL; five seconds was
only a separate expiry probe. The cache-size default remains zero; comparisons
explicitly enable 64 misses per directory. The Go serving reader already uses
`allow_other`; the measured Rust executable still needs that cross-UID wiring and
a large-namespace memory envelope. Root-only fixtures
do not establish those contracts. Anonymous reader memory is identified, but its
live-heap owners remain unprofiled because the configured listener is absent.

The earlier verified code-diagnosis fixture uses 8 CPU / 16 GiB for filesystem plus
client and 16 CPU / 12 GiB for the synthetic S3 backend, with parent ceilings of
24 CPU / 28 GiB. An intermediate 8 GiB filesystem/client limit still caused
page reclaim despite low daemon RSS. Verify actual limits and pressure counters
for each new fixture instead of assuming these numbers guarantee headroom.
Keep the application payload budget and worker counts explicit and unchanged
when comparing fixed-binary resource effects.

The [change history](change-history.md) summarizes confirmed refill, cross-reader
eviction and read-history lifetime corrections. Warm duplicate payload disappears
in the selected comparison, but both Go mean p99 values remain lower. Initial-read
joins point to before-request and after-socket client work for further attribution.
These observations do not pass acceptance or explain every wait. The subsequent
64 MiB representative matrix and persistent mixed run expose substantial random
read and sustained-rate losses. Current checkpoint tables are in the
[README](../README.md); [architecture](architecture.md) describes the delivered
design and [next work](next-work.md) records confirmed causes and unresolved
attribution. Subsequent fixed-binary native GET/FUSE profiles confirm the repeated
warm cache misses without quota throttling or reclaim. Required admission can now
use revocable idle flush capacity without changing the total payload limit;
owned replies and active uploads retain their separate progress guarantees.
The subsequent 120-record healthy-warm comparison eliminates repeated random GETs
but retains tail, concurrent-read and memory losses. All four binaries complete
60 persistent mixed cycles; candidate Rust takes 709 seconds versus 600 for both
Go references. Rare successful approximately 30-second Rust reads are a new
attribution priority, not yet a proven SDK/admission defect. Full comparative and
compatibility acceptance remains open; current numbers and scopes are in the README.

The subsequent native VM investigation joins the 30-second read to admission
missing cache owners published after its initial reclamation sweep, and observes
a deterministic failing test before correction. Source `67f7aec` reclaims on
publication without losing FIFO position; directory-local epochs also preserve
fresh listings across unrelated writes. Its fixed-work control improves elapsed
and metadata work. The reverse-order equal-rate control meets its schedule, but
exposes a late-run metadata-tail regression and higher late cgroup memory than Go.
Tail/throughput losses and full compatibility remain open; neither control proves
a memory plateau. See the separate [2026-10-31 checkpoint](performance.md#native-vm-admissiondirectory-checkpoint--2026-10-31).

The first-stage scope still excludes FUSE passthrough and disk-backed caching.
No single profile proves that every possible optimization is exhausted; report
covered workloads, unresolved attribution and remaining compatibility gaps.

The [integrated 2026-11-01 checkpoint](performance-integrated.md) measures the
subsequent freshness/PLUS/LIST, per-open history and clean-fsync changes together.
Its 240 healthy records and 2,880 mixed phases are independently byte/generation
verified, while original isolated-controller errors and short-window accounting
gaps remain retained. Paced metadata tails improve; metadata/writer throughput
and concurrent/equal-rate cgroup memory still lose. Full acceptance remains open.
