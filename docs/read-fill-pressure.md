# Mounted sequential-read pressure regression

## Confirmed mechanism

The earlier adaptive-history repair corrected a real defect, but its single-reader
fixture did not reproduce neighboring kernel requests arriving before read-ahead
completed. A bounded diagnostic of an actual disposable FUSE mount exposed that
overlap at the unchanged 64 MiB configured memory limit:

- The shared data lane is 50 MiB. At large-window promotion, one fill reserves
  51,511,296 bytes for a 25,690,112-byte prefetch plus a 128 KiB reply, accounting
  for materialization and GET scratch buffers.
- A neighboring 128 KiB request misses while that fill is still in flight. It
  attempts another whole-window admission and replaces the retained cache policy
  to reclaim memory before waiting for permits.
- The first fill publishes its blocks into a previously cloned, now detached
  cache. The waiting reader cannot observe them and fetches another overlapping
  window. More neighboring requests repeat this cycle.
- The trace consequently shows successive offsets 21,364,736, 21,495,808,
  21,626,880 and onward, each fetching a 25,690,112-byte window. This explains the
  large amplification rather than attributing it to language/runtime overhead.

The trace and scratch diagnostics stay in ignored private output. Temporary
instrumentation was removed. These diagnostic runs are not equivalent-quota
Go/Rust performance evidence; the authoritative paired results are recorded
separately in [performance](performance.md).

## Repair and invariants

Register generation-scoped in-flight intervals before whole-window admission.
Overlapping readers wait for the existing owner and then retry normal cache
lookup; independent intervals still fetch concurrently. Reuse Tokio watch channel
closure for completion/error/cancellation notification. The RAII interval owner
removes its registration on every exit; follower waits have the existing request
deadline. No lock is held across backend I/O.

Completed blocks publish into the current cache policy, including after another
inode replaces that policy under pressure. Preserve output admission before
borrowing charged cache blocks, the existing atomic scratch reservation, dirty
generation checks, conditional GETs, bounded parallel chunks and reserved flush
progress. Payload, cache, entry and concurrency budgets are unchanged. No unsafe
code, new dependency or production operation is introduced.

## Public regression evidence

Four new `Filesystem::read` tests reproduce the actual ownership/admission paths:

1. After 160 contiguous 128 KiB reads, overlap the promoted fill with two neighboring
   reads under a blocked backend. Cancel one follower, then release the backend.
   Exact remaining bytes and unchanged capacities must hold; the four initial
   ranges plus two promoted chunks are sufficient. Original code failed with
   eight backend reads instead of six.
2. Replace the cache policy through pressure from a different inode while the
   promoted fill is blocked. Its next neighboring read must still hit the completed
   window. Restoring publication into a detached clone failed with nine backend
   reads instead of seven.
3. Block two disjoint windows of one inode. Both must reach the backend before
   either completes, then return exact bytes. Changing interval overlap from AND
   to OR serialized them and failed the controlled three-second deadline.
4. Cancel the owner while its overlapping follower waits. The follower must wake,
   retry correctly and retain exactly the cache-plus-reply budget. Removing the
   owner's unregister operation failed the follower deadline.

All faults were restored. Formatting, warning-denying all-target/all-feature
Clippy, all 130 Rust tests and the 336-package public-origin audit passed.
Fresh committed-source paired mounted verification subsequently completed all 80
selected sequential/eviction results against both Go baselines, cold/warm and
five repetitions. Mean Rust time is now 3.087–3.375 seconds for 256 MiB rather
than the previous roughly 404 seconds; all group mean throughputs and RSS/cgroup
totals favor Rust. An additional concurrent smoke pair also passed. The complete
reviewed metric summaries are linked from [performance](performance.md), including
the unfavorable p95 and mixed p99 results. Whole-project compatibility, full-matrix
and tail-latency acceptance remain **UNACCEPTED**.
