# Initialize streamed fragments through maintained buffer APIs

Status: **unaccepted for production**. The preceding
[borrowed-reply repeat](borrowed-read-reply.md) removed an allocation but did not
improve whole-filesystem speed/RSS. Its adverse results remain retained.

## Narrow change

The streaming adapter previously allocated a zero-filled fragment and immediately
overwrote every byte through `read_exact`. Tokio 1.54.1 already supplies the
needed mechanism: `copy_buf` consumes the SDK reader's AsyncBufRead buffer with
no extra intermediate allocation, `take` bounds consumption to the admitted
fragment length, and `AsyncWrite for Vec<u8>` extends initialized bytes without
pre-zeroing the unused capacity. This is supported library behavior, not a local
unsafe/uninitialized-buffer implementation.

`Reservation::read_exact` preallocates only the reserved capacity, invokes those
APIs and checks the copied count before moving the Vec into the existing charged
Block. Short reads/errors drop the private partial allocation and reservation;
cancellation drops both through the same future owner. The fragment limit keeps
the vector within the admitted size and does not consume the following fragment.
No global pool, detached producer, new dependency or budget/concurrency tuning is
introduced. Only `src/budget.rs` and `src/backend/s3.rs` change among compiled
inputs relative to the exact borrowed-reply release.

The existing final EOF check still runs before the last fragment is delivered.
Advertised whole-body checksums still take the complete-validation path and do
not publish early. Their existing materialization path remains unchanged.

## Verification before measurement

- All 181 Rust cases, 14 source/oracle Python cases and 23 benchmark Python cases
  pass in isolated secret-free Kubernetes. Formatting, all-target/all-feature
  warning-denying Clippy, the 336-package public-source audit and native release
  pass after explicit package cleanup.
- Existing real SDK/HTTP tests cover prefix timing, unaligned/EOF bytes, short/
  excess bodies, checksum barriers, exact charges, cancellation and late errors.
  Original writer/generation/FIFO ownership tests remain unchanged. The real
  matched performance failure is the optimization's red; no artificial private
  initialization counter or flaky timing assertion is added.
- Eleven independent faults fail the relevant public behavioral checks, including
  suppressed IO errors and consuming past the admitted fragment. Restored suites
  pass. The initial copied-count guard mutant survives because the real SDK
  rejects truncated HTTP first; that observation is retained. The revised fault
  suppresses the reachable SDK IO error instead of inventing an impossible mock.
- Crapper reports the new admitted fill at CC 3 and read-inner at CC 53. Dryer
  retains three previous candidates. Coverage/CRAP remain unavailable. Mutator
  scans 18 budget sites and kills the one selected count-comparison mutation;
  17 sites are unselected, not covered by that score.
- Independent receipt reconciliation verifies test counts, fault/tool logs,
  11 restored source hashes, 25 compiled inputs and exact executable bytes. An
  initial proof expected CC 2 and failed; the measured CC 3 was retained and the
  corrected receipt verification passes without changing production source.

See [the quality receipt](initialized-read-fragment-quality-20261028.json).
Reproduction uses the commands in
[the previous checkpoint](borrowed-read-reply.md#verification), on the exact
source revision recorded with the forthcoming matched results.

Baseline source: `906eb45cef9a015c8725bda189b3431d7c6b7d7b`, executable SHA256
`b96c9427be58f5c6ad3f22920417aea697ab01cdc83b0a64d0e60cedfbe8a55f`.
Candidate executable SHA256:
`20d450570792eb21a6090e1399d429472fac8d1648ae1fcc52bb95dda27c40e7`.
Candidate 25-input tree SHA256:
`42fd17aa65700cde56239094b8df7c4275a44a2d26263e6b5daaffaedf142472`.

## Matched native comparison

Source `e000ecfd0d05aac39fc5809bfadc784ad1a75946` completes repetitions 60..64,
five rotated/reversed orders with fresh and complete-dataset revisit for the new
release, exact borrowed prior and both unchanged Go references. All 40 phases
finish in 675.245 seconds: 120,000 full-byte/metadata checks, unchanged native
conditional hashes before/after and 97 exact listings. All 460 indexed artifacts,
280 CPU windows and both Rust compiled-input commits are independently checked.
The [numeric receipt](initialized-read-fragment-results-20261028.json) includes
all CPU splits, p50/p95/p99, dispersion and allocator/client/cgroup/file/kernel/
backend memory, including unavailable metrics.

The sealed SeaweedFS 4.51 manifest, 3,000 files / 8,298,727,528 bytes, 2 CPU / 4 GiB
filesystem quota, separate 4 CPU / 6 GiB backend, 512 MiB payload budget,
100,000 entries and four 128 KiB readers are unchanged. Default allocator settings
and complete xattr/forced-stat workload are identical. Host/backend cache is not
controlled cold or globally dropped; the full revisit exceeds daemon cache.

Five-phase means, decimal MB. RSS is peak / final-fifth mean, not a stationary
plateau. Whole CPU includes daemon, clients, controller and metadata work.

| Implementation/state | Read MB/s | Daemon RSS peak / steady MB | Whole cgroup CPU s | Read p99 ms | Listing s |
| --- | ---: | ---: | ---: | ---: | ---: |
| Go experimental fresh | 652.207 | 1238.274 / 1208.453 | 27.589 | 9.714 | 0.420 |
| Go experimental warm | 629.293 | 1286.134 / 1203.692 | 27.746 | 10.274 | 0.034 |
| Go packaged fresh | 622.615 | 1276.957 / 1211.058 | 29.036 | 10.416 | 0.448 |
| Go packaged warm | 600.116 | 1280.059 / 1208.621 | 29.188 | 10.990 | 0.030 |
| Rust initialized fresh | 560.922 | 559.474 / 438.540 | 32.035 | 12.034 | 1.512 |
| Rust initialized warm | 537.871 | 561.948 / 452.708 | 32.053 | 12.931 | 0.020 |
| Rust borrowed prior fresh | 556.240 | 564.261 / 429.579 | 32.145 | 12.192 | 1.499 |
| Rust borrowed prior warm | 538.578 | 563.561 / 456.058 | 32.028 | 12.488 | 0.021 |

Against exact prior, throughput changes +0.84% fresh / -0.13% warm and whole CPU
-0.34% / +0.08%. Each favors only two of five pairs in both states. Fresh p99
improves 1.30% (four favorable pairs), but warm p99 worsens 3.55% (two favorable
pairs). Peak RSS falls 0.85% / 0.29% (three favorable pairs each); final-fifth RSS
rises 2.09% fresh and falls 0.73% warm (one/two favorable pairs). The repeat does
not demonstrate a reproducible overall gain or statistical significance.

Both Go references still win throughput, whole CPU, p95/p99 and cold listing.
Rust retains lower process RSS, but file-cache pressure prevents a whole-cgroup
memory win. Go peak/steady heap series and backend request metrics remain missing,
not zero; the separate plain-list/stat Go failure remains retained.

Two targeted removals of application copy/initialization have not closed the
sustained gap. Reject the premise that those redundant operations alone dominate
this workload. Remaining actors include allocation/retention and kernel page
work, per-fragment SDK/delivery/cache publication, and separate cold metadata
resolution. Their absolute costs need a new controlled attribution before another
mechanism change; selected sample percentages are not proof of dominance or a
Rust/SDK/network ceiling. The existing writer/random/HTTP and broader compatibility
gaps remain, and overall acceptance stays **UNACCEPTED**.
