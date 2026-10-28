# Range-index coverage filtering

## Mechanism

[Absolute CPU profiles](read-cost-attribution.md) identify repeated cache
predecessor lookups as a concrete application cost. Previously the index stored
only replacement stamps: every predecessor required another BTree root search
and a Moka `get` to discover that its bytes did not cover a request. Successful
but unusable gets cloned payload owners and recorded cache popularity.

Store each immutable block's length beside its stamp in the existing bounded
index. Use one reverse BTree range iterator to skip spans that cannot contain
the requested bytes, then consult Moka only for a covering candidate. Continue
to check the returned block's real bounds; the index and Moka are not a single
transaction under concurrent clones/deferred eviction. An older overlapping
block may cover a range that the nearest short block cannot.

The index retains no payload owner. Stamp-protected replacement, entry limits,
generation isolation, gaps, full backing charges, FIFO admission before borrow,
conditional GET validation, final EOF and writer/fsync paths remain required.
No cache/allocator tuning, protocol, dependency, unsafe code or extra GET is added.

## Verification boundary

Three public `ReadCache` tests cover overlapping short spans, replacement growth
and shrinkage with an old pinned view, gap/entry eviction and the last
representable offset. They pass on the unchanged source first: this is a
behavior-preserving optimization, so no artificial functional red is claimed.
The retained matched performance deficit is the performance red. Independent
index-length, exact-end and stale-eviction-stamp faults must fail the observable
byte/coverage/ownership assertions before restoring and checking the candidate.

Full Kubernetes checks and an uninstrumented, exact-source comparison against
initialized `e000ecf` and both frozen Go binaries are complete. They establish
correctness for this workload, not an overall speed, tail-latency or memory win.

The [quality receipt](range-index-coverage-quality-20261028.json) verifies 184 Rust
and 37 Python cases, formatting, all-target/all-feature warning-denying Clippy,
the 336-package public-source audit, three assertion faults and one selected tool
mutant. Twenty-eight cache sites remain unselected; coverage/CRAP and three prior
duplication candidates remain open. It binds all 25 compiled inputs to release
SHA256 `e39a70ba3e8c492398005a13a95aebd7242c27c6badfc02b3ed0df7a1c70e047`.

## Matched native result

Source `a5862ed9b3e3da884ba4a6a75ddb77df1202c1cd` differs from frozen initialized
`e000ecfd0d05aac39fc5809bfadc784ad1a75946` only in `src/cache.rs` among the
25 compiled inputs. Five rotated/reversed orders, repetitions 70–74, compare both
Rust executables and both frozen Go references on fresh and warm-revisit mounts.
The forty phases complete in 711.564 seconds: 120,000 full-file/metadata checks,
unchanged native hashes before/after, 97 exact listings, 280 recomputed CPU windows
and 460 indexed artifact hashes pass. Both Rust input sets match their commits.

The same sealed 3,000 files / 8,298,727,528 bytes, filesystem/client 2 CPU / 4 GiB,
separate backend 4 CPU / 6 GiB, 512 MiB payload budget, 100,000 entries and
4 × 128 KiB readers are used. Default allocator and concurrency are unchanged.
Complete xattr/forced-stat work is included for every implementation; original
plain-list/stat Go failures remain separate. Fresh means a new mount/process,
not controlled cold host/backend caches. No global cache drop or forced GC.

Five-phase means, decimal MB. RSS is process peak / mean final fifth, not a
stationary plateau. The [numeric receipt](range-index-coverage-results-20261028.json)
also retains p50/p95, daemon/client user/system CPU, allocator, cgroup/file/kernel
and separate-backend memory, dispersion and paired directions.

| Implementation/state | Read MB/s | Daemon RSS peak / final-fifth MB | Whole cgroup CPU s | Read p99 ms | Listing s |
| --- | ---: | ---: | ---: | ---: | ---: |
| Go experimental fresh | 611.120 | 1285.700 / 1207.845 | 29.561 | 10.379 | 0.452 |
| Go experimental warm-revisit | 614.903 | 1301.544 / 1223.898 | 28.355 | 10.188 | 0.027 |
| Go packaged fresh | 556.448 | 1285.680 / 1191.434 | 32.351 | 11.427 | 0.482 |
| Go packaged warm-revisit | 542.404 | 1271.868 / 1202.970 | 32.083 | 12.148 | 0.033 |
| Rust candidate fresh | 550.252 | 565.741 / 438.515 | 32.732 | 12.573 | 1.575 |
| Rust candidate warm-revisit | 508.725 | 566.253 / 455.383 | 34.292 | 14.684 | 0.021 |
| Rust initialized prior fresh | 537.500 | 567.533 / 454.507 | 33.240 | 12.640 | 1.426 |
| Rust initialized prior warm-revisit | 528.391 | 566.238 / 462.940 | 32.732 | 12.844 | 0.020 |

Against exact initialized prior, mean throughput changes +2.37% fresh / **−3.72%
warm**, whole CPU −1.53% / **+4.76%**, p99 −0.53% / **+14.32%**. Throughput,
CPU and p99 favor only two of five fresh pairs and three of five warm pairs;
the adverse warm means are retained even though most individual pairs favor the
candidate. Peak RSS changes −0.32% / +0.003% (two favorable pairs each);
final-fifth RSS −3.52% / −1.63% (four/three favorable pairs).

Candidate throughput population standard deviations are 48.931 MB/s fresh and
64.136 MB/s warm; p99 standard deviations are 2.300 and 3.203 ms. This is a mixed
repeat, not a uniform gain or statistical-significance claim. Both Go references
still have higher mean throughput and lower CPU/p95/p99/cold-listing cost in
this repeat. Lower daemon RSS is not a whole-cgroup memory win under file pressure.

Mean daemon user/system CPU is candidate 4.280/11.346 s fresh and 3.612/11.598 s
warm, versus prior 5.342/10.968 and 4.712/10.118 s. Lower application CPU does not
compensate for higher observed warm kernel cost or tails. Separate attribution
below tests the mechanism without reclassifying instrumented timings as repeats.

## Separate absolute-cost follow-up

Nine more owned-PID perf profiles rotate candidate, initialized prior and
experimental Go, three fresh mounts each. All 27,000 complete file checks,
native hashes/listings, zero lost samples, 27 raw profile/report/stack hashes and
CPU period/counter reconciliation pass. The
[profile receipt](range-index-coverage-profiles-20261028.json) binds exact source
and executable hashes. These are **instrumented diagnostics, not acceptance
timings**, and cannot replace the adverse matched warm result.

Within this same diagnostic, prior → candidate selected Moka/BTree leaf cost is
1.577 → 0.365 sampled CPU seconds; inclusive covering cost is 1.541 → 0.145 s.
These overlapping categories are not additive. Daemon user/system means are
6.400/13.870 → 4.707/13.410 s, with process RSS peak/final-fifth means
558.488/425.700 → 562.924/440.879 MB. Experimental Go is 6.257/12.453 s and
1278.759/1206.140 MB. Instrumented RSS is not a memory improvement either.

Rust minor faults barely change: 1,199,178 → 1,195,905 versus Go 305,973.
Selected fault-leaf cost is 1.620 → 1.459 s, Go 0.430 s. TCP receive/page work,
kernel copies and separate cold metadata resolution remain actors; this does not
prove allocator causality. The warm system-CPU increase is not reproduced by
these fresh instrumented profiles, so it is not attributed to the index change.
Capture coverage differs: candidate 96.7–97.5%, prior 97.2–97.7%, Go 92.3–93.3%.
Unknown/truncated frames and unrecorded CPU remain unassigned.

The index mechanism demonstrably removes lookup work, but overall compatibility
and performance stay **UNACCEPTED**. Go heap series, backend request metrics,
representative writer/random/HTTP workloads, broader compatibility, historical
EAGAIN attribution and binary notices remain open. FUSE passthrough and disk cache
are deferred beyond the first stage by the operator.

Final collected receipts were rechecked: 272 public metric means, 72 dispersion
values, eight Markdown rows, paired changes/directions, 12 source/test hashes,
25 unchanged compiled inputs and the release executable hash. The owned mount
and daemon inventory is empty. Both completed diagnostic/build Pods were deleted
with UID/owner guards after final collection; deletion receipts are retained.
