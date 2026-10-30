# Change history and reasons

This is the concise engineering context for the experimental Linux/S3 rewrite.
Detailed results and operational logs stay outside tracked source files.
Overall compatibility and comparative performance remain **UNACCEPTED**.

## Initial implementation and correctness

- Reused fuser and AWS SDK rather than implementing FUSE/S3 protocols. Added
  bounded allocation ownership, generation-aware namespace state and sparse,
  truncate, multipart, rename and fsync paths. Optional backend adapters preserve
  reference metadata, authentication, signing and PATCH contracts through public
  dependency APIs; actual-service coverage remains incomplete.
- Reserved a flush lane and retained permits through pinned/queued buffers and
  cleanup. This fixes progress and ownership, not whole-process RSS.
- Bounded concurrent type resolution and directory-page/preload reuse avoid
  serial HEAD/repeated LIST work. Binding-aware metadata validation prevents an
  unrelated remote deletion from invalidating successful operations or late
  responses from removing dirty/newly durable descendants.

## Read-path investigation, 2026-10-27–28

- Adaptive history originally missed cache-hit progress. Corrected contiguous
  tracking, but deterministic request reduction alone did not fix mounted speed.
- Actual FUSE tracing then found neighboring reads evicting active-fill policy
  and publishing into an orphaned cache clone. `572195a` coalesces overlapping
  generation-scoped fills and publishes into current cache; disjoint work remains
  concurrent. This resolves the severe selected sequential throughput collapse.
- Charged immutable views remove redundant cache-hit and callback reply copies.
  Requested bytes can reach FUSE before unrelated read-ahead finishes, while the
  same caller-owned future retains cancellation, errors and admission.
- Incremental SDK body publication avoids waiting for an entire unchecksummed
  range. Whole-body checksums retain their validation barrier. Maintained Tokio
  copy APIs avoid pre-zeroing fully overwritten fragments. These changes had
  mixed CPU/RSS/tail tradeoffs, not an unconditional performance win.
- A consumed transport `Bytes` owner survived until the next body poll and
  impeded receive-buffer reuse. Maintained futures/Tokio adapters now drop it
  promptly. Permit splitting transfers charges to actual fragment backings.
- Required pressure eviction used whole-cache replacement because Moka lacks
  synchronous pop-LRU. Payload retention now uses maintained `lru`; metadata stays
  in Moka. Range-index coverage filtering avoids useless cache lookups.
- System allocator screening exposed a speed/RSS tradeoff. It remains an explicit
  comparison feature with scoped counters; jemalloc stays the default. Counter
  ablation did not identify telemetry as the remaining bottleneck.

## Read-ahead corrections, 2026-10-29

- Equal higher CPU/memory resources removed quota throttling and reclaim, but
  Rust tail latency still lost to Go. Resource effects were not a source fix.
- `6e16525`: cache hits skipped next-window planning, exposing the next GET on
  demand. Socket/FUSE joins and selective-delay injection confirmed this mechanism.
  Refill now runs after reply publication in the existing owned request lifetime,
  skips unavailable optional admission and preserves integrity/cancellation.
- `d0d7b99`: optional refill could evict another reader's unread backing. Track
  contiguous consumption and reclaim foreign backing speculatively only when
  fully consumed. Required LRU/FIFO admission stays unchanged. Traffic and tails
  improve, but the experimental Go p99 gap remains.
- `2bf8184`: inode history survived the final close, so reopening inherited a
  promoted initial window. Reset `Pattern` on zero-to-one open, preserving payload
  and active-reader history. Warm duplicate payload disappears in the selected
  comparison; p99 improves versus prior Rust but still loses to both Go references.
  P95/CPU regressions versus that prior remain part of the conclusion. Concurrent
  handles still share inode policy; full per-handle isolation is not implemented.

## Benchmark contract correction, 2026-10-30

The acceptance runner still required cold-start superiority after that requirement
was removed. Cold runs now retain all ratios and correctness, provenance and
accounting checks, while comparative performance gates apply to warm work.
The regression fails before the correction; adverse warm tails still fail.
This aligns the runner with the contract and establishes no runtime improvement.

Go allocator accounting no longer substitutes a final sampled pprof total for
missing temporal data. A separate bounded observer records live `HeapAlloc`
without requesting GC, retains failures and excludes boundary-crossing requests.
Loopback HTTP/timeout tests and four injected faults verify the adapter; actual
workload overhead and long-lived heap behavior still need measurement.
Reference inspection also finds baseline-owned `debug.FreeOSMemory` calls. Their
counter deltas are reported without rejecting the reference or changing its GC
policy; the observer itself never requests collection.

The worker also labelled metadata/random/writer runs warm after scanning only
payload. Warmup now executes the matching checked operation; the failure case
primes a healthy writer before injection. The client is closed even if warmup
fails, and warmup receives the bounded workload deadline rather than the short
cold-initialization timeout. Earlier screening evidence remains diagnostic.

Open-file fsync recovery failures in both pinned Go references are consistent
with their shared flush state machine: `SyncFile` clears `forceFlush` before
returning the retained error, and small-file background upload needs a closed
handle, `forceFlush` or memory pressure. Waiting past the retry interval does not
by itself restore progress. Preserve those failures and the original workload
rather than increasing retries until a different state happens to pass.

## Idle flush-capacity cache policy, 2026-10-30

Fixed-binary native controls confirm repeated warm random GETs in Rust while both
Go references issue none. These controls/profiles disable Go's allocation-triggered
GC threshold with `--gc-interval 0`, retaining pressure GC; their scope is configured
diagnosis rather than default-policy comparative acceptance. CPU profiles and
complete FUSE/transport joins locate the corresponding waits without quota
throttling or memory reclaim. Observers
add measurable CPU/time, and the profiles do not identify a single SDK defect.
Exact-version upstream discussions and supported Tokio APIs were checked first.

The new public-behavior regression fails with 192 rather than 96 reads for a
12 MiB clean working set at an unchanged 16 MiB payload limit. Required admission
now transfers exclusive cache backings into idle flush permits instead of losing
those bytes. Upload activity revokes loans and follows cloned budgets/bodies;
owned replies copy loaned bytes into data admission. No total/dirty limit or
durability barrier changes. Optional speculation keeps its previous policy.

Kubernetes verification passes 211 Rust cases, 57 Python cases, fmt, all-target/
all-feature Clippy and the public-source audit. Five deliberate faults fail the
intended assertions: disabled lending, exported loan owner, omitted revocation,
dropped upload activity and omitted reopen reset. Sources are restored and tests
rerun. The history test now forces cache eviction independently of its old cap.
Default and System-allocator ThinLTO releases build successfully. The default
release also passes 30 backend, cache-loan and read-view contract cases.
Quality tools report complexity without Rust coverage; CRAP remains unavailable,
and a mutation-site scan is not a whole-repository mutation score. Mounted
comparative performance, sustained memory and compatibility remain separate gates.

## Cache-policy measurement and remaining waits, 2026-10-30

The versioned default-GC comparison completes 120 healthy-warm records on four
binaries, with all byte checks and 30 frozen generations verified. Candidate warm
random GETs are zero versus 3,380–3,389 for its predecessor. Random throughput
improves, but tails, concurrency and memory still lose agreed gates. Mean writer
p99 also hides an unfavorable independently pooled p99. The latest README uses
total checked work / summed measured time for throughput, keeping mean rates,
variance and every outlier in retained evidence.

The first default-GC attempt had stopped after 47 records on memory pressure;
backend working files outlived the paired workload. Scoped per-group bucket
lifetime removes that imbalance in the new configuration without raising payload
or weakening the resource gate. It is not a source optimization. The earlier
GC-threshold-disabled diagnostic and incomplete/rejected attempts are retained.

All four binaries also complete 60 mixed cycles / 360 passes each. Candidate Rust
takes 709.11 seconds versus predecessor 873.78 and both Go at 600.00; its late RSS
still exceeds experimental Go. Raw serial latencies expose 11/16 approximately
30-second successful reads in candidate/predecessor and none in either Go. Quota,
memory-pressure and reclaim counters are clean, but I/O waits are substantial.
Their causal stage remains unproven. Independent whole-series accounting replaces
bounded-tail means for this checkpoint; no plateau or overall acceptance follows.

## Earlier measurements and open gates

The completed 2026-10-30 native operation matrix retains all 210 records: 190
correct and 20 Go open-file fsync-recovery failures. It exposes a severe warm
random loss despite selected sequential/writer gains. Deterministic traces
confirm repeated backend reads under the static data-lane working-set pressure;
they do not isolate all mounted latency. The idle-flush policy above now passes
that diagnostic and the ownership regressions; these older operation/mixed
measurements describe its unchanged predecessor, not the new release.

A persistent mixed mount completes 60 cycles / 360 checked passes on each binary.
Rust takes 885 seconds versus about 600 for both Go references, despite lower
process RSS and CPU. Whole-series cgroup/file/kernel accounting prevents calling
that an all-metrics memory win or plateau. Parent/backend quota interference was
measured and versioned separately; increased headroom is not a code improvement.
The observer-on/off experiment stops at its bounded pre-mount deadline after 133
correct records, rather than completing 180; it remains partial attribution.
The [README](../README.md) contains the requested aggregate checkpoint; detailed
evidence stays ignored. [Architecture](architecture.md) describes delivered
ownership and [next work](next-work.md) separates confirmed causes from hypotheses.

Release builds now use ThinLTO and one codegen unit to optimize across the SDK
and application crate boundaries. With unchanged runtime sources, four rotated
mount pairs improve throughput, p50/p95/p99 and CPU versus the former Rust build
in the selected System-allocator fixture. Small RSS/cgroup increases are retained;
both Go p99 values still win. Default jemalloc builds and release-mode backend
contracts also pass, but this is not representative-workload acceptance.

Initial-read joins find additional Rust client work before GET write and after
required socket bytes. Internal SDK hooks retain the setup/signing/response work
with external observers removed and with client-wide rather than per-request
instrumentation. Default endpoint resolution is only part of that work; its two
calls correspond to auth selection and request endpoint orchestration. These
boundaries do not establish an SDK defect or a causal contribution to global p99.
Keep instrumentation outside the runtime and validate a targeted change before
claiming an improvement.

Hardlink EIO, optional actual-service/CLI/imported-layout compatibility,
representative performance acceptance and long-lived memory behavior remain open.
Go live heap series now exist for the persistent run; short phases can still
lack two valid samples, and observer overhead is not eliminated. Lower daemon
RSS and higher selected throughput do not close these gates.
FUSE passthrough/disk caching remain deferred. Dependency rationale and applicable
upstream links are in [dependency decisions](dependency-decisions.md).
