# Charged immutable cache-hit replies

The sealed read diagnostic's owned-daemon profile includes
`Reservation::copy` on the cached FUSE reply path. A public filesystem regression
independently observed the corresponding new payload charge: a cached 4 KiB
read raised data ownership from 2,097,152 to 2,101,248 bytes. This is a concrete
copy/allocation cost, not proof that copies explain every CPU or tail deficit.

## Ownership change

`Block` retains its existing `Arc<Allocation>`, complete allocation reservation
and optional outer-owner reservation, plus a visible standard-library byte
range. `Block::slice` validates against the current visible range, including
nested views. Clones, subranges and empty views retain the entire backing charge
until the last owner is dropped. No small view claims to own only its visible
bytes. Mutable fill and owner attachment remain exclusive to a new allocation.

Both cache-hit paths return such an immutable view; a miss still allocates its
admitted output and bounded backend windows. The owned fuser reply borrows the
view synchronously, then drops it. No allocator/cache/FUSE replacement or unsafe
code is introduced. The [dependency decision](dependency-decisions.md) records
the evaluated Bytes adapter and the existing mutable/charged-owner requirements.

The small admission check remains before borrowing a cache block. An attempted
removal failed the existing FIFO regression: a large waiter could need bytes
published into the cache after that waiter's eviction. Returning a view alone
did not reclaim the cache's ownership and stranded that waiter. The final change
preserves pressure reclamation and fair admission, while avoiding allocation and
copy into the unused reply reservation. The failed variant is retained as a
rejected intermediate implementation; its timeout is not a passing check.

## Observable regressions and failure sensitivity

- `tests/read_views.rs`: a cache-hit reply adds no new payload allocation;
  after cache pressure its small view still holds the complete 2 MiB backing
  charge; last-owner drop releases that charge. A held reply also retains its old
  generation across a subsequent write and successful fsync, while a new read
  returns the independently expected new bytes.
- `tests/budget_limits.rs`: nested ranges and an empty view retain full ownership;
  invalid relative ranges cannot expose bytes outside their parent even when the
  backing allocation is larger. Existing large-budget bounds remain checked.
- Existing `tests/read_windows.rs` tests continue covering FIFO pressure,
  cancellation, overlapping/adjacent windows and publication into the current
  cache policy. Their assertions were not weakened to accept the new layout.

The original-copy source failed the new cache-hit assertion with the expected
extra 4,096 charged bytes; the immutable-generation case already passed. The
first view variant then failed the pre-existing FIFO progress assertion. After
retaining admission, all twenty focused cases pass. Separate parent-offset and
parent-bounds faults each failed their intended public behavior assertion with
exit 101; restored source passes. Two selected arithmetic tool mutants are also
killed. This deliberately limited mutation scope is not whole-program coverage.

## Verification and measurement boundary

The secret-free Kubernetes build environment passed:

- all **161 Rust tests**, failed/ignored zero;
- 23 Python evidence cases through the Rust wrapper and fourteen source/oracle
  Python cases, formatting, all-target/all-feature warning-denying Clippy and the
  336-public-package source audit;
- restored focused tests after both direct faults and the selected tool run;
- native prior and candidate release builds with all compiled-input hashes.

The prior source rebuild reproduces the exact executable SHA-256
`876795011201d5098b4f60f25e3f98b6191c0d008fdef215894ab9a9bf77a2a7`.
The view candidate executable is
`0f4e8dd2b03ca5b681bce474a448c54d5c68e45c1b6692be8fecafc1183b1f39`;
only `src/budget.rs` and `src/filesystem.rs` differ among compiled inputs.

Crapper reports Block view helpers at CC 1 and the existing filesystem read
function at CC 50. Coverage/CRAP remain unmeasured; the three earlier duplicate
candidates remain. These are not completed broad quality gates.

## Matched native comparison

The forty-phase comparison at `df7e755` completed with 120,000 full file checks,
unchanged native conditional hashes/listings before and after, and 280 independently
recomputed CPU windows. All 460 indexed artifacts were checked again after
collection. Both Rust compiled-input sets match their recorded Git commits.
The [numeric summary](cached-read-views-results-20261027.json) includes source and
executable hashes, all group means, primary dispersion, client/allocator/cgroup/
backend memory and CPU. The original full numeric receipt and per-phase raw
files remain retained separately; this does not overwrite the earlier comparison.

The [sealed methodology](immutable-read-dataset.md) is unchanged: 3,000 files,
8,298,727,528 bytes, 97 exact listings, 2 CPU/4 GiB filesystem cgroup, separate
4 CPU/6 GiB backend cgroup, 512 MiB payload budget, 100,000 entries and four
128 KiB readers. Five interleaved rotations/reversal cover both states for both
Go references, the exact prior copy binary and charged views. The default allocator
is unchanged. Explicit xattr/forced-stat work is included for every implementation;
the earlier failing plain-list/stat Go workload remains separate.

Five-phase means, decimal MB; RSS is sampled peak / mean final fifth, not a
stationary plateau. CPU is whole filesystem-cgroup usage, not wall time.

| Implementation/state | Read MB/s | Daemon RSS peak / steady MB | Cgroup CPU s | Read p99 ms | Listing s |
| --- | ---: | ---: | ---: | ---: | ---: |
| Go experimental fresh | 605.337 | 1292.725 / 1206.547 | 29.723 | 9.858 | 0.453 |
| Go experimental warm revisit | 595.021 | 1273.804 / 1213.711 | 29.215 | 9.986 | 0.032 |
| Go packaged fresh | 601.326 | 1313.608 / 1222.234 | 29.911 | 9.976 | 0.471 |
| Go packaged warm revisit | 573.798 | 1279.944 / 1215.664 | 30.278 | 10.509 | 0.025 |
| Rust charged views fresh | 574.811 | 537.125 / 361.390 | 31.107 | 16.932 | 1.481 |
| Rust charged views warm revisit | 535.804 | 641.930 / 359.425 | 31.998 | 17.315 | 0.020 |
| Rust prior copy fresh | 562.545 | 538.467 / 354.158 | 31.783 | 17.647 | 1.418 |
| Rust prior copy warm revisit | 524.622 | 659.778 / 356.828 | 32.652 | 18.165 | 0.020 |

The modest mean improvement against the matched copy binary does not establish
an overall win: Go remains faster with lower CPU and p99, Rust fresh listing is
slower, and view steady RSS/allocator use and system CPU are not uniformly lower
than the prior Rust binary. Dispersion overlaps; no significance claim is made.
All filesystem cgroups remain near their memory cap under file-cache pressure,
so lower daemon RSS is not a whole-memory acceptance result. Native backend
request metrics and Go heap time series remain missing. Overall compatibility/
performance acceptance remains **false**.
