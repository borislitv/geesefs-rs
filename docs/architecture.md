# Linux/S3 architecture and ownership

This describes the delivered implementation, not a complete POSIX port or a
production acceptance claim. See [compatibility](compatibility.md),
[dependency decisions](dependency-decisions.md) and [next work](next-work.md).

## Components and request lifetime

```text
Linux VFS / FUSE
  -> fuser 0.18 callbacks (src/fuse.rs)
  -> bounded Tokio request + owned reply + request deadline
  -> Filesystem (src/filesystem.rs)
       namespace / inode state / paged metadata
       read policy + charged cache / sparse write generations
  -> Store trait (src/backend/mod.rs)
  -> AWS S3 SDK + narrow public-API adapters (src/backend/)
  -> S3-compatible endpoint
```

`src/main.rs` validates foreground configuration, constructs the SDK/runtime,
mounts fuser, runs periodic dirty-file flushing and optionally writes allocator
statistics. Startup multipart expiry is prefix-confined and must be explicitly
disabled for a read-only diagnostic requiring no backend mutation.

`Fuse::spawn` admits at most `max_requests` active jobs. Closing admission returns
EINTR; saturation returns EAGAIN. A job owns its reply and permit until completion
or the request deadline. Timeout drops the future and its reply; fuser supplies
the failure reply. There is no unbounded application request queue.

## Namespace and metadata

`src/namespace.rs` binds names to stable inodes; `src/inode.rs` owns generation,
remote object identity, dirty extents and lifetime state. Namespace epochs,
tombstones and move/delete leases prevent stale responses from resurrecting
removed or renamed bindings. Pending operations remain retryable rather than
pretending S3 copy/delete is an atomic POSIX rename.

Moka bounds directory-page/preload retention; listing and type resolution are
paged and admission-limited. Remote metadata is validated against the observed
binding/generation before installation. Dirty or newly durable local state is not
replaced merely because an older response completes later.

Directory pages use inode-local listing epochs. A mutation invalidates its parent
and any moved/deleted cached subtree, preserving fresh pages elsewhere. Epochs
are read and changed under the namespace lock, independently of refreshed inode
metadata. In-flight listings validate both epoch and name-to-inode binding before
publication; move/delete leases invalidate on entry and on every completion path.

## Payload ownership and progress

`src/budget.rs` uses Tokio owned semaphore permits. Every `Block` owns its full
physical allocation through an `Arc`; slices and clones retain that backing's
charge. Cache eviction releases only cache ownership, not permits held by an
active reader, reply, upload frame or dirty snapshot.

All three lanes are carved from the requested payload limit:

| Lane | Capacity | Purpose |
| --- | --- | --- |
| ingress | `max(total / 16, 1 MiB)` | Bounded callback input and read replies |
| flush | `2 * 5 MiB` | Reserved materialization/upload progress under dirty pressure |
| data | `total - ingress - flush` | Shared clean cache, dirty extents and pinned/temporary payload |

A 64 MiB configuration therefore provides 50 MiB data, 4 MiB ingress and 10 MiB
flush. Required admission may move an exclusively cache-owned clean backing's
charge from data into idle flush capacity. The retained policy ceiling is then
60 MiB; no lane or total limit increases. Shared/pinned allocations cannot move.
`effective_budgets()` reports capacities/use and current clean cache loans;
`flush_used_bytes` includes these loans as well as upload allocations. These are
payload-owner bounds, **not RSS, allocator or cgroup memory bounds**. Metadata, SDK/runtime,
allocator retention, clients and kernel caches require separate measurement.

Required data admission reclaims only enough LRU ownership to satisfy the request;
queued semaphore waiters retain FIFO priority. Dirty-pressure progress can flush
through the independent reserved lane. Optional read-ahead uses nonblocking
data admission and skips pressure instead of initiating loans or blocking required
work. A required miss may reclaim any cache owner; existing FIFO admission stays
ahead of later hits or loans.

A fill can publish new cache ownership after admission's first reclamation sweep.
Waiting admission therefore listens for cache publication through Tokio `Notify`
and retries reclamation while retaining the same FIFO semaphore reservation.
Registering before inspecting the cache closes the check/wait race; cancellation
drops the reservation and notification with the caller-owned future.

Before uploading, a flusher holds shared Tokio `RwLock` activity and synchronously
revokes cache loans. Independent uploads remain concurrent. The guard travels
with cloned upload budgets into SDK bodies and jobs, preventing new loans until
the last allocation-capable upload owner finishes, including after caller
cancellation. Cache lending only tries exclusive activity synchronously and
never waits. Loaned bytes are copied into an already admitted data reply before
an owned `Block` can leave the cache, so callers cannot pin a revocable reserve.

## Reads, generations and cache

`src/read.rs` derives a generation from local version and remote ETag. Overlapping
fills of that generation share a bounded interval/watch notification; disjoint
fills remain concurrent. The fill guard removes the interval on success, error
or cancellation. Followers recheck current state/cache rather than inheriting an
unvalidated result from a different generation.

`src/cache.rs` retains charged immutable blocks using `lru` plus a range index.
A normal cached backing can supply a view; joining fragments or reading a loaned
backing uses admitted output. Required pressure may reclaim any LRU owner.
Optional refill protects a foreign unread prefix using contiguous-consumption
hints, not permanent pins.

Adaptive `Pattern` currently belongs to the inode, not each open handle. The
zero-to-one open transition resets history while preserving cached payload;
concurrent handles still share history. Cache hits advance policy and can start
next-window refill after publishing the reply, in the same owned future,
request slot and deadline.

The SDK range path preserves If-Match, exact range/EOF and body integrity.
Unchecksummed responses can publish bounded fragments incrementally; a whole-body
checksum retains its validation barrier before any bytes become reusable.
Consumed transport owners are dropped promptly through maintained stream APIs.

## Writes, durability and cancellation

`src/write.rs` stores sparse, generation-tagged extents; truncation preserves
zero-hole/visible-remote boundaries. A flush snapshots one version under the
inode's flush lock. Later writes retain their newer dirty state. The durable
version advances only after successful backend completion and reconciliation.
Errors retain dirty data for retry; fsync is a completion barrier, not enqueue
acknowledgement.

`src/parts.rs` supplies configured multipart boundaries. Upload/copy/queue
semaphores bound work. `src/upload_body.rs` builds retryable SDK bodies from
1 MiB materialized frames rather than allocating an entire object/large part.
Output plus conditional-GET scratch is admitted atomically in the flush lane;
emitted frames retain their charge through transport ownership.

Cancellation aborts/drains multipart tasks and retains the admission token until
owned cleanup completes. Backend cleanup is drained before runtime teardown.
FUSE destroy closes request admission, waits for active jobs under a shutdown
deadline and flushes remaining dirty state; failures reach the process exit path.

## Verification boundary

Public behavior tests cover ownership, FIFO/pressure, generation races, checksum
barriers, cancellation and writer recovery. Real HTTP and mounted fixtures cover
the wiring that memory-store tests cannot prove. CI library checks, representative
benchmarks and actual-service compatibility are distinct gates.

Default release builds use jemalloc and ThinLTO with one codegen unit. The
`system-allocator` feature is an explicit alternative with differently scoped
counters. RSS, live allocator use and cgroup file/kernel memory are not
interchangeable. Current measured losses and remaining compatibility work are
recorded in [next work](next-work.md); the idle-loan policy is a bounded ownership
change, not proof of whole-filesystem speed or memory acceptance.
