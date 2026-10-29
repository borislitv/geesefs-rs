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

## Matched comparison

Source `3016433370e9d2ea7daee495106572012a70228f` completes all forty phases,
repetitions 150–154, in 832.687 s. All 120,000 file checks, native before/after
hashes, 97 listings, 280 CPU windows, forty backend-counter windows and 460
artifact hashes pass. Rust variants use the same System instrumentation; the
four compiled-input changes and immutable dataset are independently verified.

The [numeric receipt](selective-pressure-eviction-results-20261028.json) retains
all selected memory/CPU/counter scopes and dispersion. Five-phase means, decimal
MB; RSS is peak / final-fifth mean, not proof of a stationary plateau.

| Implementation/state | Read MB/s | Daemon RSS peak / final-fifth MB | Whole cgroup CPU s | Read p99 ms | Listing s |
| --- | ---: | ---: | ---: | ---: | ---: |
| Go experimental fresh | 464.164 | 1290.811 / 1200.484 | 37.911 | 16.244 | 0.545 |
| Go experimental warm-revisit | 479.125 | 1278.407 / 1203.145 | 36.118 | 15.599 | 0.040 |
| Go packaged fresh | 455.624 | 1301.918 / 1200.441 | 38.871 | 16.254 | 0.581 |
| Go packaged warm-revisit | 452.854 | 1316.224 / 1211.306 | 37.580 | 16.365 | 0.036 |
| Rust selective eviction fresh | 495.758 | 603.774 / 603.751 | 35.550 | 15.689 | 1.910 |
| Rust selective eviction warm-revisit | 474.407 | 626.632 / 626.632 | 35.664 | 16.714 | 0.025 |
| Rust fragment-owner prior fresh | 506.083 | 571.915 / 556.837 | 35.026 | 15.491 | 1.784 |
| Rust fragment-owner prior warm-revisit | 469.114 | 583.950 / 579.541 | 35.772 | 17.058 | 0.026 |

Payload amplification is substantially reduced, but not erased:

| Source/state | Extra object payload MB | Extra payload % | Range responses |
| --- | ---: | ---: | ---: |
| Candidate fresh | 35.967 | 0.433 | 3997.8 |
| Prior fresh | 221.105 | 2.664 | 4008.2 |
| Candidate warm-revisit | 27.368 | 0.330 | 3833.6 |
| Prior warm-revisit | 491.179 | 5.919 | 3896.8 |

Fresh/warm excess is about 84%/94% lower than prior. However, fresh mean throughput
falls 2.04%, CPU rises 1.50% and p99 rises 1.28%; warm throughput improves 1.13%,
CPU 0.30% and p99 2.02%. Retaining useful cache ownership increases peak RSS
5.57%/7.31% and final-fifth RSS **8.43%/8.13%**, with no favorable memory pair.
Throughput/CPU/p99 favorable pairs are 2/5 fresh and 3/5 warm. Candidate throughput
population stdev is 42.131/16.011 MB/s, and p99 stdev is 1.683/0.552 ms. These
tradeoffs remain visible; lower backend bytes do not establish a general win.

Process RSS remains approximately half of Go's and fresh means improve on both
Go references, but warm p99 loses to both and warm throughput loses to experimental
Go. Cold listing still takes 1.910 s versus 0.545/0.581 s; LIST-labelled requests
remain 213 versus seven. Whole-cgroup memory remains near its cap. Request/cost
windows follow client phases, not forced quiescence; background read-ahead can
cross the boundary. Keep the small 0.010 MB mean warm experimental-Go excess too.

The pressure regression is fixed and amplification is reduced; all-metric and
representative-workload acceptance remains open. In particular, fuser 0.18.0's
public readdir API requires a concrete FileType and cannot express DT_UNKNOWN.
Standard LIST lacks authoritative special-file metadata, so omitting those HEADs
would require a supported unknown-type reply path rather than false regular-file
hints. A dependency/API decision is needed for that cold-listing path; no unsafe
encoding or source-audit exception has been introduced.
