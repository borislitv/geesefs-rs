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

## Current work and open gates

Initial-read joins find additional Rust client work before GET write and after
required socket bytes; similar header wait does not establish an SDK or backend
defect. Attribute scheduler/admission/SDK/body-to-reply stages, research exact
upstream versions/issues, reproduce a reachable problem and make a narrow fix.

Hardlink EIO, optional actual-service/CLI/imported-layout compatibility, the full
representative performance matrix, Go heap series and long-lived memory behavior
remain open. Lower daemon RSS and higher selected throughput do not close them.
FUSE passthrough/disk caching remain deferred. Dependency rationale and applicable
upstream links are in [dependency decisions](dependency-decisions.md).
