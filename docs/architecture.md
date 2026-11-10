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
or the request deadline. Timeout drops the future and returns ETIMEDOUT for an
unsent reply. A reply published before read-ahead is already terminal and cannot
be sent again during cancellation. There is no unbounded application request queue.

File-handle slots are reserved before create can mutate the namespace. A registry
entry owns its slot and `Arc<OpenFile>`; admitted reads retain that open inode until
their last owner finishes. Closing admission drains requests before clearing the
registry. READDIRPLUS holds accepted inode owners until successful buffer delivery;
error/cancellation transfers no lookup references, and dot entries acquire none.

## Namespace and metadata

An optional `--metadata-limit` adds independent lifetime byte admission for
application metadata. It includes canonical inode/listing/result/writer owners
and temporary operation plans; payload `--memory-limit` retains its meaning.
The count guard is additional. Reclamation examines bounded batches and yields
outside the namespace lock; lookup/open/dirty and in-flight pins remain intact.
These are admitted logical allocations, not allocator size classes or an RSS cap.

Clean current attributes and their committed baseline share `CachedMetadata`, an
immutable sorted slice. Writers prepare a separately admitted copy-on-write
version; old snapshots/baselines remain unchanged. `CachedRemote` retains only
size, ETag and the canonical baseline. Full `ObjectHead`/HTTP properties belong to
transient backend HEAD/COPY/REPLACE handling, where private upload receipts are
checked before projection. Operation credit for committed-generation records is
reserved before PUT/completion/PATCH and transferred into the new owners.

`SharedKey` keeps one immutable key backing and its charge across namespace name
bindings, inode state, snapshots and cached listing records. Both key and metadata
wrappers expose borrowed data, preventing a caller from detaching storage from its
permit. Shared backings are charged once until their final actual owner; index
and operation-wrapper costs remain separate. Extended listings pass canonical
metadata directly to inodes instead of encoding then decoding it again.

`src/namespace.rs` binds names to stable inodes; `src/inode.rs` owns generation,
remote object identity, dirty extents and lifetime state. Namespace epochs,
tombstones and move/delete leases prevent stale responses from resurrecting
removed or renamed bindings. Pending operations remain retryable rather than
pretending S3 copy/delete is an atomic POSIX rename.

Inode-number resolution uses a private Papaya weak index. A lookup upgrades its
protected weak reference to an owned `Arc` before leaving the index; no pin spans
an inode lock or await. Name bindings and canonical strong owners remain under
the namespace transaction mutex. Eviction uses `Arc::try_unwrap` to arbitrate
against concurrent resolution, never reuses an inode number and cannot detach a
retained owner. This is lock-free inode lookup, not a lock-free filesystem.

An ordered dirty-generation index is maintained under namespace-then-inode lock
order for every writer mutation. Flush snapshots visit dirty owners only;
ancestor retention uses an ordered prefix existence query. Commit completion
reconciles the current generation, preserving newer writes, failed/cancelled
uploads and renamed descendants. Retiring a deleted identity cannot erase a
recreated name's dirty owner. Rename pins/rekeys only bound source entries and
their relevant subtree, rather than locking unrelated cached inodes.

Local directory overlays seek directly between immediate children, skipping
entire cached subtrees. Expired wide directories yield after at most 128 ordered
scan steps per namespace critical section. Existing epoch/binding validation
still rejects publication across mutations. Rename/delete transactions and
per-inode state/flush locks remain; dirty snapshots and subtree mutations cost
proportionally to their own pending work.

The [locking audit](locking.md) records every application mutex/RwLock domain and
remaining contention risks. Payload cache reads use only short internal metadata
transactions, with no outer mutex spanning response allocation/copying. Loan-key
tracking makes revocation proportional to actual loans; synchronous in-progress
copies retain their byte charges until they finish, never across an await.

Moka bounds directory-page/preload retention; listing and type resolution are
paged and admission-limited. Remote metadata is validated against the observed
binding/generation before installation. Dirty or newly durable local state is not
replaced merely because an older response completes later.

Directory pages use inode-local listing epochs. A mutation invalidates its parent
and any moved/deleted cached subtree, preserving fresh pages elsewhere. Epochs
are read and changed under the namespace lock, independently of refreshed inode
metadata. In-flight listings validate both epoch and name-to-inode binding before
publication; move/delete leases invalidate on entry and on every completion path.

Cached pages validate every unconsumed child's binding, type and freshness. A
FUSE frame resumes the same backend page but binary-searches past names already
delivered, avoiding their repeated validation and copies. Consumed inode eviction
does not force a new LIST. The backend-page continuation remains distinct from
true EOF, including insertions before the cursor; no inode or lookup owner is
retained merely to keep a consumed prefix alive. Authoritative
extended LIST metadata can refresh an existing clean inode, preserving owners and
local counters; a later HEAD or verified commit wins over the older request.
Kernel replies carry the remaining metadata TTL. Common prefixes establish names,
not authoritative directory-marker attributes. Basic READDIR remains available;
kernel continuation resumes the same bounded backend page until it is consumed.

Each selected entry carries the observation time of its metadata source: cached
inode, authoritative LIST or resolved HEAD. Publishing a new names page does not
renew cached permission/xattr freshness. Standard LIST may supply size/ETag but
cannot revalidate absent user metadata; kernel attribute TTL remains bounded by
that older observation. Later HEAD/commit generations still override older data.

## Optional persistent payload tier

The three disk-cache flags enable a private, locked, preallocated local container
through the maintained Foyer Store adapter. Only eligible capacity evictions
populate it, after releasing the L1 lock; replacements, invalidations and loan
revocation do not. Demotion funds an exact visible-range copy, promptly releasing
the original backing. Qualified L2 hits promote into L1 while retaining the disk
copy. Missing coverage and optional errors use ordinary conditional S3 reads.

Persistent keys include validated backend/access/encryption scope, exact object
key, opaque ETag, size and coverage. Dirty/sparse/pending namespace states bypass
the tier. Reads revalidate generation after waits, including after funding a
window whose file may have changed size. Opaque/rotating credentials explicitly
bypass persistent reuse rather than guessing a stable principal.

Funding covers bounded index/order state, pools, copies, read/decode work and
submission credits, reducing effective L1 capacity without taking writer or
ingress reserves. Recovery is asynchronous with source fallback and no inventory
reset. Owned jobs retain funding/directory ownership past caller cancellation;
shutdown starts cache teardown even when writer flushing later fails.

Metrics distinguish admission, actual worker completion and caller delivery.
Region-reclaim callbacks are not per-record eviction counts; funded metadata is
not measured index heap, and opening-time filesystem allocation is not a live
physical-storage gauge. See [the design](disk-cache-design.md) and
[qualification plan](disk-cache-plan-track.md) for remaining acceptance gates.

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
a caller-owned `Block` can leave the cache, so callers cannot pin a revocable reserve.

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

Each FUSE `OpenFile` owns its adaptive `Pattern`, matching the reference handle
lifetime. Concurrent and reopened handles share payload/fills but not stream
history. Legacy inode-only APIs retain their separate inode pattern. Cache hits
advance policy and can start next-window refill after publishing the reply, in
the same owned future, request slot and deadline.

`FOPEN_KEEP_CACHE` is unset. After releasing a clean regular file with no remaining
open owners, the adapter asks fuser to invalidate its kernel pages rather than
retaining pages that the next open will discard. One independent blocking slot
bounds these potentially writeback-waiting notifications without taking request
admission or holding userspace inode locks. Shutdown joins that slot. Eligibility
is rechecked in the worker; concurrent reopen, retained readers or an occupied
slot can defer or skip this best-effort reclamation. Kernel page locking and
writeback govern a racing notification; dirty userspace extents are not discarded.

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

PUT and multipart completion carry a private 128-bit `geesefs-upload-receipt`
metadata value. After a lost response or cancellation, a size-matching HEAD with
the owned receipt permits baseline recovery, never durability acknowledgement.
Dirty extents and newer truncate cuts remain until another verified commit.
Unrelated remote generations fail closed. The receipt is hidden from filesystem
xattrs and reserved against user/generated-attribute collisions; it consumes
54 bytes of the object metadata budget. Recovery is scoped to a live mount,
not persistence of RAM dirty data across restart. This provenance is not an
authentication boundary against an authorized external writer copying metadata,
nor server-side fencing of a request that completes after a later operation.

Generated permissions and mtime are captured with the payload generation under
the same state lock, after upload admission. Namespace refreshes compare their
observation time as well as content generation, because a metadata-only update
can retain its ETag. Pending renames preserve both parents until reconciliation;
expired implicit directories recheck marker metadata unless directory objects
are disabled.

A clean fsync returns before upload admission and cache-loan revocation; dirty
generations and deleted inodes retain their durability/error checks.

`src/parts.rs` supplies configured multipart boundaries. Upload/copy/queue
semaphores bound work. `src/upload_body.rs` builds retryable SDK bodies from
1 MiB materialized frames rather than allocating an entire object/large part.
Output plus conditional-GET scratch is admitted atomically in the flush lane;
emitted frames retain their charge through transport ownership.

When one dirty extent already covers an upload range of at most 1 MiB, expose its
immutable charged view through `Bytes::from_owner` instead of materializing a
second frame. The byte owner also retains upload activity through retries,
cancellation and retained transport bytes. The SDK's known-body path signs a
checksum header without aws-chunked encoding. Sparse, composed and remote ranges
still use bounded streaming; checksum policy and post-commit verification remain
unchanged. A short final multipart range respects the extent's backing offset.

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
