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
       active read ownership + Foyer / sparse write generations
  -> Store trait (src/backend/mod.rs)
  -> AWS S3 SDK + narrow public-API adapters (src/backend/)
  -> S3-compatible endpoint
```

`src/main.rs` validates foreground configuration, constructs the SDK/runtime,
mounts fuser, runs periodic dirty-file flushing and optionally writes allocator
statistics. Startup multipart expiry is prefix-confined and must be explicitly
disabled for a read-only diagnostic requiring no backend mutation.

`Fuse::spawn` admits at most `max_requests` active jobs. Closing admission returns
EINTR. Saturation waits on the existing semaphore in the fixed fuser receiver
threads, instead of returning EAGAIN that Linux buffered reads can expose as EIO.
There is at most one pending callback per receiver in addition to admitted jobs;
no detached waiter queue is created. Pending WRITE input remains ingress-funded.
No admission, file-registry or namespace mutex spans this wait.

One absolute deadline starts at request admission and covers capacity wait,
executor queueing and execution. Already-expired jobs cannot begin a mutation.
A job owns its reply and permit until actual completion or cancellation, including
read-ahead after an early reply. Timeout returns ETIMEDOUT for an unsent reply;
an already-published reply stays terminal and cannot be sent again.

Waiting on a receiver deliberately delays later callbacks, including metadata
and FORGET on a single receiver. Admitted async jobs progress independently of
that receiver; kernel notification work has a separate slot and blocking worker.
This is bounded backpressure, not priority scheduling or a bound on time still
queued in the kernel. Synchronous non-yielding work remains non-preemptible.
The executable uses a multithreaded Tokio runtime; tests using a current-thread
runtime must continue driving it while fuser waits, as for other async callbacks.

File and directory handles share an owned semaphore bounded by `entry_limit`,
independently of active-request concurrency. Reserve a slot before create can
mutate the namespace or opendir can retain an inode. File registry entries own
their slots and `Arc<OpenFile>`; directory cursors retain their slots through any
admitted readdir owner. Admitted reads retain that open inode until
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

An active directory remainder pins its unsent inodes through frame preparation;
the TTL page cache still holds numeric identities, not the whole namespace alive.
Kernel lookup ownership is separate: TTL expiration does not send `FORGET`.
A zero/nonzero identity counter requests bounded, detached `inval_entry` batches
near count pressure. The finite scan excludes active, open, dirty and moving
owners; names survive notification without retaining inode pins. Application locks
are released before the potentially blocking notification. Real `FORGET` alone
releases kernel lookup ownership. This improves the verified nested walk, but
does not guarantee progress through every over-capacity flat directory.

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
remaining contention risks. Active-fill publication and subscriber coverage use
short internal transactions. Response composition is synchronous and retains its
byte charges; a subscriber releases published owners before awaiting pressured
output admission. There is no retained clean-cache or loan-revocation registry.

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

The three disk-cache flags enable a private, locked local container through the
maintained Foyer Store adapter. Proven-new media initially stays Prepared: funding
and directory ownership are reserved, but no engine or payload file is constructed
until an eligible validated source fill. Existing media starts bounded recovery
eagerly; lazy creation must not discard its inventory. Population funds an exact
visible-range copy without retaining its full source backing. Qualified L2 hits
serve active reads; no decoded RAM copy survives independent requests. Missing
coverage and optional errors use ordinary conditional S3 reads. Partial records
remain partial; unknown bytes are never rounded into a cache hit.

Persistent keys include validated backend/access/encryption scope, exact object
key, opaque ETag, size and coverage. Dirty/sparse/pending namespace states bypass
the tier. Reads revalidate generation after waits, including after funding a
window whose file may have changed size. Opaque/rotating credentials explicitly
bypass persistent reuse rather than guessing a stable principal.

Funding covers bounded index/order state, pools, copies, read/decode work and
submission credits, reducing active data capacity without taking writer or
ingress reserves. Recovery is asynchronous with source fallback and no inventory
reset. Owned jobs retain funding/directory ownership past caller cancellation;
shutdown starts cache teardown even when writer flushing later fails.

Required read windows use a key-free FIFO queue before allocating persistent
identities. Four funded read-work owners are independent of four funded spill
owners; eight aggregate jobs are possible without enlarging the fixed pools.
Each potential disk block also reserves 4096 bytes of bookkeeping within the same
data-window admission, reducing its effective ceiling rather than adding a pool.
Eligibility borrows that credit and a read slot for bounded blocking work. It
releases namespace ownership before waiting for inode state, then releases inode
ownership before revalidating the namespace. No guard crosses disk/source I/O.
Qualification and each block lookup have their own bounded operation deadline;
a source gap does not age later lookups against one shared window deadline.
Timed-out or cancelled tasks retain their actual slot, credit and owners until
completion. Source-fill population uses existing guarded admission with one
cumulative bounded wait allowance per window. It is independent of RAM pressure
and never borrows the writer flush reserve.

Metrics distinguish admission, actual worker completion and caller delivery.
Region-reclaim callbacks are not per-record eviction counts; funded metadata is
not measured index heap, and opening-time filesystem allocation is not a live
physical-storage gauge. See [the design](disk-cache-design.md) and
[qualification plan](disk-cache-plan-track.md) for remaining acceptance gates.

## Payload ownership and progress

`src/budget.rs` uses Tokio owned semaphore permits. Every `Block` owns its full
physical allocation through an `Arc`; slices and clones retain that backing's
charge until the last active reader, reply, upload frame or dirty snapshot drops.
There is no retained application's clean-payload cache.

All three lanes are carved from the requested payload limit:

| Lane | Capacity | Purpose |
| --- | --- | --- |
| ingress | `max(total / 16, 1 MiB)` | Bounded callback input and read replies |
| flush | `2 * 5 MiB` | Reserved materialization/upload progress under dirty pressure |
| data | `total - ingress - flush` | Persistent-tier funding, dirty extents and active/pinned payload |

A 64 MiB configuration therefore provides 50 MiB data, 4 MiB ingress and 10 MiB
flush. `effective_budgets()` reports these owner capacities and usage. No loan,
revocation, cache-publication notification or upload/lending activity guard remains.
Under data pressure, flush dirty data through the independent reserve and wait on
the existing FIFO semaphore with a finite deadline. Cancellation drops the waiter
with its caller-owned future. These are payload-owner bounds, **not RSS, allocator
or cgroup memory bounds**. Metadata, SDK/runtime, allocator retention, clients and
kernel caches require separate measurement.

## Reads, generations and cache

`src/read.rs` derives a generation from local version and remote ETag. Overlapping
fills of that generation share a bounded interval/watch notification; disjoint
fills remain concurrent. The fill guard removes the interval on success, error
or cancellation. Followers recheck the current generation and reuse validated
active owners. Before retaining those owners across a wait, obtain reply admission;
otherwise release the subscription before queueing so it cannot self-pin an older
FIFO reader. A covering immutable owner supplies a view; fragmented results use
their admitted composition output. Independent reads consult Foyer/source again.

Each FUSE `OpenFile` owns its adaptive `Pattern`, matching the reference handle
lifetime. Concurrent and reopened handles share persistent payload/active fills
but not stream history. Legacy inode-only APIs retain their separate inode
pattern. Every caller read advances history. A scalar planned-end hint prevents
sliding redundant speculative windows, without claiming persistent coverage.
Without an eligible funded L2, fetch required bytes only. Read-ahead remains in
the same caller-owned future, request slot and deadline; there is no hit-triggered
retained-RAM refill.

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

A clean fsync returns before upload admission; dirty
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
recorded in [next work](next-work.md); removing retained clean payload is an
ownership change, not proof of whole-filesystem speed or memory acceptance.
