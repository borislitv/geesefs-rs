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

A repeated matched native comparison is required before claiming throughput,
CPU, RSS or tail improvement. Keep the previous measured losses and the complete
new results, including adverse repetitions. Overall compatibility/performance
acceptance remains **false**.
