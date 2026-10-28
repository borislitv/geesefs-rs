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

## Next measured check

Compare exact source/binary inputs with frozen System and both Go references on
the same sealed fixture and unchanged budgets. Include backend payload bytes and
request counters now that the fixture listener works, alongside CPU, peak/final-
fifth RSS, cgroup/file/kernel memory, latency and all adverse repetitions. Native
functional checks do not establish a speed or RSS improvement. Overall acceptance
remains open.
