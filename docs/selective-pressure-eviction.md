# Reclaim only the cache ownership needed by admission

## Real filesystem reproduction

The fragment-admission correction leaves measurable excess payload and the
filesystem's whole-cache pressure path. Reproduce that path through Filesystem,
using its actual equal cache/data limits rather than a smaller standalone cache:

1. Use the supported 16 MiB total budget, giving a 5 MiB shared data lane.
2. Read three distinct 2 MiB remote objects, retaining no caller views.
3. Revisit the second, more recent object.

The old source performs **four GETs instead of three**: admitting the third object
drops both previous cache owners although dropping the oldest was sufficient.
Payload checks succeed before the additional-request assertion fails. This is
a reachable pressure regression, separate from the rejected isolated TinyLFU
example in the preceding investigation.

## Supported cache operations

Reuse maintained MIT-licensed lru 0.19.0 for payload retention. Its sparse
constructor avoids allocating storage for the configured entry limit, push
returns entry-limit victims, and pop_lru synchronously transfers the oldest
cache owner for dropping. Moka remains in the metadata caches. The SDK's existing
lru 0.18.5 and all other dependency versions stay pinned.

The concrete missing Moka operation is synchronous LRU victim removal before
payload allocation/admission; remove returns a clone and changing its policy
alone does not provide that operation. No replacement algorithm, linked list,
allocator or pool is implemented here. Inspecting the upstream 0.19.0 release
also confirms corrected Send/Sync bounds on shared iterators; the application
does not expose iterators across threads.

Keep the existing ordered coverage index with the LRU under one mutex. Eviction
and replacement update index membership and weighted size together, so deferred
removal callbacks and their stamps are no longer required. Actual byte admission
remains the authority: a pinned caller can retain an evicted backing, and eviction
is not assumed to have freed its permits.

On pressure, remove old owners one at a time and retry the existing FIFO-aware
try_reserve. Stop when admission succeeds. If no cache ownership can satisfy it,
retain the existing ENOMEM/flush/wait path. No mutex is held across that await.
Full backing ownership, configured byte/entry limits, read-ahead/concurrency,
generation checks, checksum/EOF and durability are preserved.

## Verification boundary

The filesystem regression passes after the change, alongside existing overlap,
generation, pinned-view, read-window and FIFO cases. Additional public cache cases
cover weighted recency, pinned evicted backings, entry-limit churn, larger and
oversized replacements, exact-capacity insertion and coverage removal.

The [verified receipt](selective-pressure-eviction-quality-20261028.json) records
192 Rust cases, 37 Python cases, formatting, warning-denying all-feature Clippy,
339 public packages and the native release. Independent faults replace selective eviction with whole-cache clear,
evict newest instead of oldest, and omit byte-weight release; all fail behavior
assertions before restored package-clean verification. Both selected byte-capacity
boundary mutations are killed; 29 other scanned sites, coverage/CRAP and three
prior duplicate candidates remain open. All 25 compiled inputs match; changes are
limited to Cargo.toml/Cargo.lock, cache.rs and filesystem.rs.

The System-feature release SHA256 is
`46350f9c4c4f6db26a1fdbdea35a5ebb532e7fe7667380f88d77fe1e6a1d2cfd`, with input-tree
SHA256 `5da8fa1337f10dc3a2935e981bcec159d5052faa4993ce8193ebda38467520db`.
No allocator default change is included.

The real Rust-only FUSE/Moto sweep completes seventeen ordinary reader/writer/
fsync/error-recovery checks with no unexpected runner error. The known hardlink
emulation EIO remains; retain the nonzero overall runner result and
ordinary_mounted_passed=false. Owned mounts, daemons and integration helpers are
gone afterward. One local launch-directory setup error occurred before the
workload started and is not a filesystem failure.

A repeated same-budget comparison is next. Do not infer lower RSS, faster
operation or full compatibility from the local pressure case. Preserve random/
hot-set tradeoffs, counter/CPU window scope and every adverse result. Overall
acceptance remains open.
