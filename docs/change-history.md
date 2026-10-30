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

## Current work and open gates

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

Hardlink EIO, optional actual-service/CLI/imported-layout compatibility, the full
representative performance matrix, Go heap series and long-lived memory behavior
remain open. Lower daemon RSS and higher selected throughput do not close them.
FUSE passthrough/disk caching remain deferred. Dependency rationale and applicable
upstream links are in [dependency decisions](dependency-decisions.md).
