# Selective disk payload cache: design proposal

Status: adapter and runtime integration implemented and locally regression-tested.
The maintained Foyer fork supplies guarded Store APIs; funding is deducted from
effective L1 capacity. Native filesystem failure checks pass. Mounted workload and
performance acceptance remain open; no production acceptance is implied.
See the
[decision and reproduction](dependency-decisions.md#selective-disk-payload-cache-qualification--2026-11-09).
Inspected source: `a5aa9c003d219d7e683cc2053f8e32c653173742`.
The new request supersedes the earlier disk-cache deferral. FUSE passthrough is
outside this proposal. [Implementation plan](disk-cache-plan.md).

## Goal

Retain reusable package payload beyond RAM capacity: keep the current RAM cache
as L1 and add a bounded local disk L2. Configurable regular expressions select
files, initially `.deb` and `.rpm`; three CLI flags select the directory,
capacity and regex. There is no payload TTL or inactivity expiry.
S3 remains authoritative and writer durability remains independent of caching.
Unselected files retain the existing read path.

The operator selected on-eviction population, approximate LRU, blocks aligned with
the current L1/S3 read path, persistent reuse at terabyte scale, normal metadata
validation and S3 fallback under disk overload. The subsequent clarification
removes the configuration file entirely; queue limits are internal bounded
settings. Whole-file caching, early writes on fill and payload expiry are out.

## Findings in the current source

- `src/cache.rs::ReadCache` owns immutable, variable-sized spans indexed by
  `(inode, generation, offset)`, using `lru` and a range index. It caches bytes,
  not complete files. Eviction currently drops ownership synchronously.
- `src/budget.rs::Block` retains the entire backing allocation's permits across
  clones and slices. Enqueuing an evicted block does not free its memory.
  `Filesystem::admit_pressured` relies on reclaiming these permits to progress.
- Revocable cache loans use the flush reserve. `reclaim_loans` must release them
  promptly; a disk writer must never retain a loaned allocation across I/O.
- `read_inner`, `load_window` and `refill_after_hit` implement admission,
  generation checks and overlapping-fill coalescing. A new L2 lookup belongs in
  this read pipeline, including optional refill, rather than in a second FUSE
  implementation or a blanket backend wrapper affecting writer materialization.
- `read::generation` hashes a process-local version and ETag. It is not a
  persistent identity. `ObjectHead` currently exposes size and ETag, not S3
  VersionId. Metadata-only changes can preserve ETag.
- The SDK delivers ordinary reads in explicit 128-KiB application fragments;
  checksummed responses can deliver a larger backing only after whole-body
  validation. Reuse these read-path units without treating arbitrary HTTP/TLS
  chunks as persistent blocks or bypassing the checksum validation barrier.
- `src/config.rs` is currently clap-based; `src/main.rs::configuration` does not
  load a general configuration file. `--shared-config` concerns AWS configuration.

## Public precedents and dependencies

Sources were inspected on 2026-11-09. Documentation and source inspection are not
an integration or performance qualification.

| Solution | Applicable mechanism | Limitation for this project |
| --- | --- | --- |
| [nginx proxy cache](https://nginx.org/en/docs/http/ngx_http_proxy_module.html#proxy_cache_path) | Persistent files, temporary-file publication, `max_size`, `min_free`, LRU manager, cache loader and fill locking | Disk response cache, not application RAM-victim demotion. `max_size` is enforced by background reclamation and can be temporarily exceeded. |
| [nginx expiry/revalidation](https://nginx.org/en/docs/http/ngx_http_proxy_module.html#proxy_cache_revalidate) | Freshness TTL, conditional revalidation and explicitly enabled stale serving | `proxy_cache_valid` and `inactive` mean different things. HTTP response caching does not define filesystem metadata consistency. |
| [nginx slice](https://nginx.org/en/docs/http/ngx_http_slice_module.html) | Cache byte ranges independently; useful for large files | Very small slices cost memory/file descriptors; documented incompatibility with background cache update. |
| [Mountpoint S3](https://github.com/awslabs/mountpoint-s3/blob/main/doc/CONFIGURATION.md#caching-configuration) | Separate data retention and metadata validation; local capacity/free-space eviction | Its documented local cache is cleaned at mount and exit, so it is not the persistence contract proposed here. |
| [rclone VFS](https://rclone.org/commands/rclone_mount/#vfs-file-caching) | Sparse, downloaded-range cache, max age since access, fingerprints, space limits | Limits can be exceeded between sweeps and by open files. This is a complete Go VFS, not an embeddable Rust L2. |
| [JuiceFS](https://juicefs.com/docs/community/guide/cache/#data-cache) | RAM/disk/object-store hierarchy, bounded blocks and disk-failure fallback | Its immutable block IDs come from its own metadata/storage design; ordinary mutable S3 keys do not provide that guarantee. |
| [Foyer 0.22.6](https://docs.rs/foyer/0.22.6/foyer/) (Apache-2.0) | Maintained Rust hybrid cache; write-on-eviction/insertion, recovery, checksums, capacity and I/O controls | Not a drop-in replacement for charged `ReadCache`; default disk eviction is not LRU, and recovery builds an in-memory index for all records. |
| [cacache 13.1.0](https://docs.rs/cacache/13.1.0/cacache/) (Apache-2.0) | Maintained content-addressed disk storage, streaming and integrity checking | No complete bounded LRU manager. Removing an index key leaves its content; physical reclamation needs explicit ownership/GC. |
| [sqlite-cache](https://github.com/losfair/sqlite-cache) | TTL key/value storage and per-key update locking | The reviewed interface stores byte values in SQLite; it does not establish a bounded streaming/Range payload tier. Not selected from its microbenchmarks. |

Released Foyer 0.22.6 loses completed reinserted records after reopen. The operator's
maintained fork is selected for adapter development after guarded admission,
recovery and lifecycle qualification. This is not acceptance of the full feature.
Development remains in the operator's repositories; returning to upstream requires
equivalent verified fixes and a separate dependency decision.
Inspect
the released [policy API](https://docs.rs/foyer/0.22.6/foyer/enum.HybridCachePolicy.html)
and [storage source](https://github.com/foyer-rs/foyer/blob/v0.22.6/foyer-storage/src/store.rs).
Important concrete gaps:

- Missing TTL is not a selection blocker: the operator removed payload expiry.
- The released [block engine](https://github.com/foyer-rs/foyer/blob/v0.22.6/foyer-storage/src/engine/block/engine.rs)
  defaults to invalid-ratio/FIFO eviction, not per-record disk LRU. Memory LRU
  configuration does not change that. The
  [object engine proposal](https://github.com/foyer-rs/foyer/issues/1082) is open.
- The supported [EvictionPicker](https://github.com/foyer-rs/foyer/blob/v0.22.6/foyer-storage/src/engine/block/eviction.rs)
  interface selects storage regions, not individual payload records. A second-chance
  policy requires a verified access signal and bounded hot-record handling; the
  existence of an access-counter field alone does not establish that reads update it.
  The manager falls back to random eviction when every picker returns `None`.
  Do not use that return value as an assumed instruction to pause reclamation.
- `Store::enqueue` takes a Foyer memory `Piece`; `StoreBuilder` takes a Foyer
  memory cache. Using its disk store still requires a bounded ownership adapter.
  `Code::decode` has no caller-provided byte-budget parameter; serialized buffers,
  decoded values, queued owners and the engine's pool need separate accounting.
- The queue threshold is a size check rather than atomic admission. Configure
  it, but do not treat it as proof of the application's hard memory bound.
- The released source explicitly tests that `delete` can leave an in-flight
  keeper value readable until flushing finishes. Deletion alone is not the
  filesystem's invalidation barrier. Persistent tombstones also need explicit
  configuration for mutable cache keys.
- [Recovery](https://github.com/foyer-rs/foyer/blob/v0.22.6/foyer-storage/src/engine/block/recover.rs)
  scans [blob indexes](https://github.com/foyer-rs/foyer/blob/v0.22.6/foyer-storage/src/engine/block/scanner.rs),
  not whole payloads, but accumulates recovered entries and deduplication/index
  structures before returning. Measure startup peak memory and duration at the
  target record count; do not claim a lazy nginx-like loader from this API.

Use the fork's fallible guarded Store APIs without replacing L1; its automatic
HybridCache write/eviction API is incompatible with guarded admission. Record
measured memory, recovery and hit-retention results in `docs/dependency-decisions.md`.
Continue correcting reproduced dependency defects in the maintained fork or
qualify another maintained engine without weakening the accepted contract.
Do not start a custom storage engine merely because its API is unfamiliar.

Use maintained [regex](https://github.com/rust-lang/regex) (MIT OR Apache-2.0)
and existing clap/concurrency primitives. No configuration-file parser is needed.
Select and lock released versions during implementation. Rust regex is not PCRE:
look-around and backreferences are unsupported; matching is bounded by pattern
and input size. Compile bounded rule sets once, not on each read.

## Selected contract and proposed mechanics

### Read and demotion

```text
filesystem read -> existing RAM L1 -> eligible disk L2 -> conditional S3 range
                         |                   |
                  capacity eviction     verified hit
                         |                   |
                    bounded spill       admitted L1 promotion
                         |
                      disk L2
```

Use **best-effort write-on-eviction**, matching the selected victim-cache model.
A healthy disk preserves eligible victims; a full queue or failing disk
can discard a cache copy. It must not hold up required reads or writer fsync.
Guaranteed preservation, bounded RAM and nonblocking progress cannot all be
promised when disk writes stop.

Reserve spill capacity within the existing total payload budget, leaving ingress
and writer progress reserves intact. Transfer an exclusive complete owner when
supported; otherwise use an already-admitted bounded copy. Account for complete
backings and all engine serialization/I/O buffers. Never hold the cache mutex
across disk I/O. Queue admission is nonblocking and byte/job bounded; skip
demotion when it cannot be funded. Loan revocation and invalidation are immediate
discard paths, not asynchronous spill opportunities.

Report any reduction of effective L1 capacity needed to fund spill/engine buffers;
do not hide it behind the unchanged total or add an unaccounted second memory pool.
Disabled mode retains its existing partition and cache capacity.

An independent opt-in `--metadata-limit` controls application metadata ownership.
It does not redefine `--memory-limit` or fund a second uncharged payload cache.
Metadata charges follow retained inode/listing/returned/writer owners, including
copies and operation plans; canonical lookup/open/dirty pins remain authoritative.
Neither limit alone promises a process RSS or cgroup bound.

On L2 hit, retain the disk copy and promote only a bounded requested/read-ahead
range to L1. Do not repeatedly rewrite the same generation on promotion/demotion.
This is a victim-populated, non-exclusive hierarchy, not two disjoint copies of
one global LRU list. Reuse fill coalescing and recheck generation after awaiting
disk work. Optional refill must consult L2 rather than refetching disk-resident
ranges from S3.

### File selection and record shape

One configured regular expression selects regular files by the UTF-8 path relative
to the mount root, with a leading `/`. No URL decoding or regex substitutions;
no match bypasses L2. Symlink targets are selected by their resolved regular-file
paths. Additional policy actions or per-pattern TTLs are unnecessary.

Use the current 128-KiB clean-read alignment/delivery unit as the initial logical
L2 block size, shared in one definition with the read path. Keep shorter EOF
tails and explicit covered ranges; split larger verified L1 backings without
retaining uncharged whole backings. Do not force partial reads to download more
payload solely to complete a cache record. Missing coverage is a miss, never a
zero-filled cache hit. Account for any partial-record range index during recovery.
S3 Range GET has no fixed cache-block size; multipart-upload sizes are not the
read-cache unit. Whole-file completion is not a prerequisite for using a block.

Logical read blocks and physical storage regions are different sizes. Pack many
small payload records into larger engine-managed files/regions to avoid one local
file and inode per 128 KiB. A 1-TiB cache contains up to 8,388,608 full 128-KiB
records; even 32 bytes of index state per record costs 256 MiB before allocator,
hash-table and recovery overhead. Bound and measure the index separately.

Persistent identity includes a format version, backend namespace, exact object
key, opaque ETag, object size and block offset/length. Backend namespace separates
endpoint, bucket, prefix and incompatible access/encryption contexts without
storing credentials. Do not persist inode numbers or `DefaultHasher` generations.
Validate length, full identity and payload integrity before returning bytes.
ETag is a remote validator, not necessarily a content checksum.

### Freshness without payload expiry

There is no L2 TTL, `inactive` policy or age-based payload purge. A valid, cold
record may remain indefinitely while capacity is available. Existing metadata
cache TTL and local mutation semantics remain in force; removing payload TTL
does not mean never checking S3 again.

A newly observed ETag/size makes old payload ineligible. Permission/metadata-only
changes retain their normal validation even if content ETag stays unchanged.
Validate a restored object's current identity through the normal namespace path
before using its payload. Do not perform HEAD or GET for every cached block at
startup. Recovery restores cache inventory, not authoritative filesystem attrs.
Source failures do not authorize stale data beyond the existing contract.

Operator decision, 2026-11-11: the scoped deployment is single-user and accepts
ordinary downloaded-copy semantics. A cache hit does not perform an additional
source GET solely to reauthorize KMS decryption. Revoking remote decrypt access
does not retroactively remove an already downloaded local copy. This resolves
that deployment-contract question without relaxing normal namespace/permission
or ETag/size validation, changing credential-scope separation, or claiming
multi-tenant isolation. Payload-cache speed need not exceed RAM on every access;
the separately authorized HDD trial is a functional/resource check, not a waiver
of returned-byte correctness or bounded ownership.

### Choosing what to evict at capacity

Use approximate recency, with **CLOCK/second chance** as the proposed policy:

1. Mark a reused record as recently used with a cheap in-memory reference bit.
   A read served by L1 can touch an existing corresponding L2 record without disk
   I/O or creating a new L2 copy. A newly demoted record starts unreferenced;
   maintenance/recovery scans do not count as use.
2. Reclaim already known invalid generations, corrupt records and abandoned
   temporary work first. Do not scan the remote namespace to discover garbage.
3. Walk a bounded rotating set of evictable records. A referenced record gets its
   bit cleared and a second chance; an unreferenced record is a cold candidate.
   Skip active writers and storage ranges still required by in-flight disk I/O.
4. Remove cold records until the configured free-slot reserve is restored. Batch
   maintenance instead of reordering a global LRU or persisting a timestamp on
   every read. Record creation age is not the eviction decision.
5. Bound each sweep and any hot-record copying during region reclamation. If
   space cannot be made available promptly, reject the new cache insertion;
   required reads continue through L1/S3.

If the engine only reclaims physical regions, use its supported region selection
and bounded hot-record reinsertion facilities. Qualify the resulting approximate
policy on mixed hot/cold data; do not silently substitute default FIFO. Storage
regions remain library-owned, including safe publication and reuse during reads.
Deleting a region can remove several logical blocks, not necessarily a whole
package. A later read fetches only its missing coverage from S3.

Reserve space before starting writes: accounted allocated storage plus unallocated
reservations plus the new allocation must fit `max_size`. Do not count the same
preallocated engine space twice: manage free slots inside that reserved pool.
Unlinked-but-pinned/retired bytes still count until actually reclaimable.

Use high/low watermarks to reclaim batches ahead of the hard limit. For example,
start at 95% occupied usable capacity and restore 90%; these are proposed tuning
values, not selected defaults. An engine using preallocated containers translates
this to a free-region reserve: recycling space need not shrink file sizes or `du`.
A filesystem-free-space guard is independent of the three CLI options; a fixed
preallocated pool must not pretend that recycling records frees bytes for unrelated
processes.

### Terabyte-scale restart

Reopen completed records without clearing the directory or downloading their
contents again. Restore the inventory from persistent indexes/journals or bounded
record-header scanning; verify payload checksums when actually serving records.
Persist approximate recency in batches/checkpoints, not on each hit. Losing some
recency after a crash may cause suboptimal eviction, but must not invalidate all
payload or authorize a mismatched generation.

Recovery must have explicit peak-memory/I/O/time accounting. Prefer incremental
availability where supported, falling back to S3 for not-yet-indexed ranges.
While inventory is incomplete, do not admit writes or reclaim unaccounted old
space. Foyer's current build-time full-index recovery must be measured against
this requirement; a fork or a new custom persistent index is not implicit.

### Capacity, lifecycle and errors

- One owned cache directory per mount instance, protected against concurrent
  process use. Keep it on a local filesystem outside the served FUSE mount.
  Reject overlaps, unsafe directory ownership and configuration mistakes before
  backend initialization. Cache files are private to the daemon identity.
- Include records, metadata, temporary/in-flight reservations, engine space and
  retired-but-open storage in managed capacity accounting. Bound index memory,
  file descriptors and recovery concurrency as well as payload bytes.
- The configured maximum size bounds managed cache allocation. Any internal
  filesystem-free-space guard is not a reservation against other disk users.
  Filesystem metadata/journal overhead must be stated separately; use a filesystem
  quota or dedicated volume when an exact physical allocation boundary is needed.
- Evict known invalid or cold reusable data under pressure using the capacity
  policy above. No time-based payload eviction or expiry clock is required.
- Keep verified completed records across graceful restart. After a crash, losing
  recent cache insertions is acceptable; serving partial/corrupt data is not.
  Storage-format upgrades rebuild incompatible cache rather than trusting it.
- Disk ENOSPC, corruption, read/write errors and queue saturation cause a visible
  cache miss/bypass with counters and bounded diagnostics. Successful S3 reads
  remain successful. Source errors still propagate; caching is not durability.
- A caller timeout cannot cancel a kernel-blocked file syscall. Retain worker
  ownership and charges until completion, bound workers, and bypass an unhealthy
  disk rather than spawning unbounded replacement jobs. Shutdown joins owned work
  within its declared limits; do not claim impossible hard syscall cancellation.
- Dirty/sparse/local generations and pending rename/delete states bypass L2.
  Keep write/truncate/rename/unlink publication guards. Verified subsequent clean
  reads may repopulate under a new identity; disk cache completion never completes
  fsync or substitutes for an S3 commit.

## Proposed CLI, not implemented

Add only `--disk-cache-dir`, `--disk-cache-max-size` and `--disk-cache-regex`.
The directory enables L2 and requires both remaining options. Reject partial
combinations, malformed regex/sizes and invalid directory relationships before
backend initialization. Retain existing CLI/fstab parsing.

```bash
geesefs-rs -f example-bucket /mnt/packages \
  --disk-cache-dir /var/cache/geesefs-rs/packages \
  --disk-cache-max-size '1 TiB' \
  --disk-cache-regex '\.(deb|rpm)$'
```

Values and paths above are synthetic examples, not selected operating budgets.
Queue bytes cover complete retained owners; job limits alone are insufficient.
Derive bounded queue/worker limits from the existing payload budget and request
admission; publish the effective bounds. On-eviction population and persistence
are fixed behavior, not mode switches. Without the flags, retain RAM-only behavior.

## Decisions remaining

The operator's choices above are fixed inputs, not questions to repeat. The
proposed CLOCK-style eviction mechanics and resource tuning require engineering
qualification. Select directory, device and maximum size for the intended
environment. Qualify the engine, read-block/range index,
physical region layout and restart costs as engineering work. No implementation
or measured improvement follows merely from this design update.
