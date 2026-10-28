# Transfer admission with independent fragment backings

## Reproduced ownership cost

The [backend request inventory](backend-request-cost.md) exposes additional Rust
payload work. Following its admission path identifies a distinct conservative
retention: RangeSink shares one full GET reservation across separately allocated
stream fragments. Freeing a physical prefix does not release its global permits
while the producer or any other fragment survives. This can create pressure and
cache replacement without corresponding live payload bytes.

A gated real S3/SDK regression requests two 128 KiB fragments. A one-byte view of
the first fragment correctly retains its complete backing while the second half
is pending. After that view is dropped, the old source still reports **262144**
admitted bytes instead of the remaining **131072**. The HTTP exchange and full
payload checks finish before the assertion, so this is an ownership failure, not
a hanging server or compilation failure.

An earlier isolated TinyLFU test is not used as proof of this filesystem cause:
it configured cache capacity below payload admission, while the real limits are
equal and admission can replace the policy first. Moka policy and cache sizing
remain unchanged.

## Smallest ownership change

Reuse the existing Tokio `OwnedSemaphorePermit::split` path through Reservation:

- Full range/output admission still happens before allocation and network reads.
- RangeSink retains unfilled admission behind a short synchronous mutex.
- Each completed, exclusively owned physical backing receives its own portion
  before it enters the bounded channel. No lock is held across channel waiting.
- The reservation is stored inside the existing reference-counted Allocation;
  no extra shared GET owner or per-fragment owner allocation is required.
- Tiny, nested and empty views retain the entire backing allocation's charge.
- Cancelled/failed delivery drops unpublished backing and admission. Completed
  queued/cached blocks retain their own charge after producer completion.
- Delivery exceeding remaining admission fails before publishing any excess data.

This changes ownership precision, not configured limits, concurrency, FIFO
ordering, checksum/range/EOF validation, generations or durability. No new crate,
unsafe code, allocator, pool, reader, transport or cache implementation is added.
The two changed compiled inputs are `src/backend/mod.rs` and `src/budget.rs`.

Tests that formerly expected a full GET charge after tail failure/cancellation
now require the exact surviving complete prefix backing, plus a separate reply
allocation when one exists. The in-flight full-admission assertion is retained.
This is a contract correction for physically independent allocations; it does
not charge a small view by its visible length or replace exact checks with bounds.

## Verification

The [quality and mounted receipt](fragment-admission-transfer-quality-20261028.json)
binds the following Kubernetes checks to all 25 compiled inputs and the release:

- 188 Rust cases and 37 Python source/benchmark cases pass after package clean.
- rustfmt, all-target/all-feature Clippy with `-D warnings`, public-source audit
  of 338 packages and the native release pass. Dependency versions are unchanged.
- Three independent ownership/boundary faults fail four behavior assertions;
  sources are restored and package-cleaned. One selected tool boundary mutation
  is killed; one other scanned site remains unselected.
- Coverage/CRAP and three prior duplicate candidates remain open. Successful
  companion-tool exits are not a broad quality gate.

The release uses the explicit `system-allocator` feature to compare against the
frozen System candidate; default builds still use jemalloc. Binary SHA256:
`aeb8aed1fcc2f6a94bf3a993b45614baf5160754a2d7d583e75073448ea5609e`.
Compiled-input tree SHA256:
`06b02ab104179492284f9b57807bf5e00a5beec50fffaaa5f762e8a7317aa8d6`.

The actual Rust-only FUSE/Moto sweep completes checksum/Range/EOF/mmap, implicit
directories, HTTP full/Range, rsync full/delta, symlink, permissions/mtime/xattr,
FIFO/fsyncdir, sparse/truncate, fresh-reader durability, rename-before-close,
concurrent writer/fsync and injected PUT-failure recovery. No unexpected runner
error occurs. The existing hardlink-emulation probe still reports EIO; retain it,
the nonzero overall runner result and `ordinary_mounted_passed=false`. Optional
backend/CLI gaps remain. This is not paired performance or full compatibility
acceptance. All owned mounts, daemons and integration helpers are gone afterward.

One initial focused attempt failed to compile because an optional futures macro
was unavailable; use stable `std::future::poll_fn` without another feature/dependency.
The subsequent two legacy accounting failures exposed the old whole-GET
expectations above; they are retained and corrected to physical backing sizes,
not retried until green or weakened to ignore leakage.

## Matched result and remaining work

Exact source `666bb2e7e03e797650c6fd0cd4119a3a9fd7b1c5` completes all forty rotated
phases, repetitions 130–134, in 829.812 s. All 120,000 file checks, native hashes
before/after, 97 listings, 280 CPU windows, forty backend-counter windows and 460
artifact hashes pass. Both Rust executables use System/stats_alloc; only the two
ownership inputs differ. The same sealed fixture, budgets, metadata work and
CPU/memory quotas apply.

The [numeric receipt](fragment-admission-transfer-results-20261028.json) includes
all selected CPU/memory scopes, counter metrics and dispersion. Five-phase means,
decimal MB; RSS is peak / final-fifth mean, not a stationary plateau.

| Implementation/state | Read MB/s | Daemon RSS peak / final-fifth MB | Whole cgroup CPU s | Read p99 ms | Listing s |
| --- | ---: | ---: | ---: | ---: | ---: |
| Go experimental fresh | 459.133 | 1284.634 / 1204.717 | 38.758 | 16.735 | 0.515 |
| Go experimental warm-revisit | 446.397 | 1283.653 / 1220.570 | 38.591 | 17.500 | 0.037 |
| Go packaged fresh | 470.645 | 1271.024 / 1202.470 | 38.147 | 16.319 | 0.590 |
| Go packaged warm-revisit | 453.173 | 1274.926 / 1221.329 | 37.910 | 16.599 | 0.041 |
| Rust fragment-owner fresh | 509.493 | 574.050 / 566.073 | 34.890 | 15.107 | 1.825 |
| Rust fragment-owner warm-revisit | 469.036 | 590.071 / 580.326 | 36.016 | 16.865 | 0.025 |
| Rust frozen System fresh | 520.080 | 569.839 / 557.035 | 34.252 | 14.669 | 1.864 |
| Rust frozen System warm-revisit | 467.826 | 590.605 / 578.900 | 36.237 | 17.068 | 0.019 |

Against prior System, fresh mean throughput regresses 2.04%, CPU 1.86%, p99 2.98%,
peak RSS 0.74% and final-fifth RSS 1.62%. Warm mean throughput improves 0.26%, CPU
0.61%, p99 1.19% and peak RSS 0.09%, but final-fifth RSS rises 0.25%. Favorable
throughput pairs are only 2/5 in each state; CPU 3/5 fresh and 2/5 warm. Candidate
throughput population stdev is 28.922/35.750 MB/s and p99 stdev 1.476/2.167 ms.
Retain all adverse pairs rather than calling the accounting correction a general
speed win.

Payload writes above the logical 8298.728 MB dataset decrease, but remain:

| Source/state | Extra object payload MB | Extra payload % | Range responses |
| --- | ---: | ---: | ---: |
| Candidate fresh | 237.420 | 2.861 | 4003.2 |
| Prior fresh | 342.179 | 4.123 | 4000.6 |
| Candidate warm-revisit | 473.409 | 5.705 | 3887.0 |
| Prior warm-revisit | 520.419 | 6.271 | 3880.6 |

Experimental Go has zero excess in these windows; packaged Go has 0.025 MB mean
fresh excess and zero warm excess. Both Rust sources still have 213 fresh
LIST-labelled requests versus seven. GET-labelled HTTP 200 includes HEAD; do not
misclassify metadata or directory probes as full-body downloads or filesystem
failures. Counter windows follow the phase boundaries, not a forced quiescent
state; background read-ahead can cross those boundaries.

Candidate fresh throughput/CPU/p99 and process RSS beat both Go means in this
repeat, while warm p99 still loses to packaged Go and cold listing remains much
slower. Whole-cgroup memory stays near its cap with different file-cache
composition. The correction establishes backing ownership, not all-metric
performance acceptance. Remaining payload work and whole-cache pressure eviction
need further attribution; do not stop at the favorable fresh comparison. Default
allocator, budgets and production acceptance are unchanged.
