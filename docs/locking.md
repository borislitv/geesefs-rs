# Locking and progress audit

Audit scope: all application-owned `Mutex`/`RwLock` fields in `src/`, their
acquisition paths, nested ownership and work performed before release. This is
not a proof that no workload can contend. SDK, fuser, Tokio, allocator, Moka and
Papaya internals remain versioned dependencies, not lock-free application claims.
The package continues to forbid unsafe code and uses safe dependency APIs only.

## Shared hot paths

| Domain | Ownership and retained synchronization | Correction / remaining cost |
| --- | --- | --- |
| `Namespace` | One name/transaction mutex; inode owners in a private Papaya weak index | Inode resolution does not acquire it. Flush/ancestor checks use dirty indexes. Directory scans seek children and yield every 128 steps. Rename scans only its source bindings. Eviction does not wait for an inode state acquired by a concurrent resolver. |
| `ReadCache::state` | One mutex for LRU, range index, weights and loan ownership | Remove the redundant outer filesystem mutex. Resolve/touch/mark each cached read span in one metadata transaction; allocate and copy payload after releasing it. Loan revocation visits an explicit loan-key set, not the whole clean cache. |
| `flush_activity` | Async shared guards owned by uploads/bodies; lending only tries an exclusive guard | Required to prevent new loans during uploads, including cancellation and retained retry bodies. Independent uploads share the guard; speculative lending never waits for it. |
| FUSE file/directory registries | Separate short map transactions; clone owners before using them | File release drops its owner outside the registry. Directory open no longer waits for inode state or writes a reply while holding its registry. A shared file-handle lookup still takes a short registry lock. |
| FUSE admission | Closing flag plus nonblocking request-permit acquisition | Scheduling and error replies occur after unlocking. A retained permit still makes shutdown wait for an admitted job even if it has not yet been scheduled. |

The read-cache outer mutex was not needed for payload correctness: a borrowed
loaned backing is copied into admitted output synchronously and never exported
or retained across an await. After revoking cache ownership, a flusher can briefly
wait on byte admission for such an in-progress copy, not on a global copying
mutex. All loans and in-flight allocations remain charged to the same limits.
Writer input is also copied into its admitted block before acquiring namespace
and inode-state locks; validation and extent/generation publication stay atomic.

## Object-local and lifecycle locks

| Field(s) | Critical section / await boundary | Assessment |
| --- | --- | --- |
| `Inode::state` | Metadata, generation, counters, sparse extents and upload reconciliation; no network await | Necessary atomic object state. Snapshots/commit may clone or filter up to the configured extent/cut bounds; a heavily mutated single inode can contend. Namespace-to-state transactions can propagate this delay to name operations. |
| `Inode::flush` | Async per-object commit/rename/delete gate, deliberately spans backend I/O | Serializes conflicting durability operations, not independent files. Multi-inode rename acquires gates in inode order. Removing it would weaken commit/delete ordering. |
| `Inode::preload` | Async per-directory single-flight, spans one metadata request | Concurrent cold lookups of the same directory can wait up to the request timeout. Other directories remain independent. |
| `Inode::pattern`, `OpenFile::pattern` | Small adaptive-history updates, at most 64 samples | Per-handle policy in normal FUSE reads; legacy inode-only API has a per-inode pattern. No backend work or payload copy under the lock. |
| `Inode::read_fills` | Register/query/retire bounded in-flight intervals; notify followers | Per-inode metadata only. Overlapping same-generation reads deliberately wait on a watch channel after unlocking; disjoint fills proceed independently. |
| FUSE `Directory` cursor | Async per-open-handle pagination/cookie gate, spans LIST | Same-handle readdir is serialized to preserve cursor semantics; separate handles are independent. |
| FUSE `notifier` | Clone configured sender before kernel notification | Actual potentially blocking notification runs on a separately admitted blocking worker without the mutex. |
| FUSE `shutdown_error` | Terminal diagnostic publication/take | Not a request hot path. |
| `RangeSink::owner` | Split an already admitted reservation | Per-producer, no allocation or channel send while locked. Channel backpressure happens after release. |
| `IamProvider::cache` | Async single-flight token refresh, spans timeout-bounded HTTP | Cold/expired IAM identity requests can share a network wait. Cached credentials return promptly; this does not block executor threads or apply to other authentication modes. |
| `UploadBody::pending` | `Mutex::get_mut` through exclusive body polling | No runtime lock acquisition. It supplies the SDK's `Sync` bound for a `Send` future; network polling does not hold a mutex guard. |

READLINK clones its bounded target before sending a kernel reply, so a blocked
reply cannot retain inode state and indirectly block namespace transactions.
Metrics use try-lock snapshots and return unavailable instead of joining a queue.

## Residual scaling risks

- Full-capacity namespace eviction still searches canonical owners; all-pinned
  pressure can cost O(cached inodes) under the namespace mutex. The weak upgrade
  race is safe, but this is not a constant-time eviction algorithm.
- Global tombstone expiry cleanup and some pending-prefix conflict checks still
  scale with their own registries. Local directory scan chunks no longer trigger
  whole-tombstone cleanup. Large move/delete transactions still cost their subtree.
- The cache metadata mutex remains shared. Overlap searches, evicting many small
  spans, lending past already loaned owners and releasing loan keys can extend
  its hold time. Payload copies are outside it; sharding or lock-free replacement
  would require preserving global byte admission, eviction and revocable loans.
- Namespace publication/preload/page validation can process a bounded backend
  page and acquire object-local state. Sustained writer/metadata pressure and
  same-inode fragmentation need target-environment hold/wait profiles.

The [diagnostics](testing.md#namespace-progress--2026-11-06) test removed global
serialization and retained writer/loan contracts. They do not prove zero tail
latency, mounted performance acceptance or the absence of locks inside dependencies.
