# Cross-reader refill eviction: measured result

## Scope and implementation

Candidate `d0d7b99abed63d38f5f32376b56186a8a4847735` fixes an independently
reproduced pressure defect in the [first refill](read-ahead-refill.md). Optional
refill could reclaim another reader's partially consumed cache backing and cause
an additional GET for that reader's next sequential request. The public regression
first fails with four GETs instead of three after verifying correct bytes.

Each existing bounded cache-index entry now tracks its contiguous consumed prefix.
Optional admission stops at a foreign unread LRU owner; a disjoint tail read does
not mark a gap consumed. Required admission keeps the existing LRU/FIFO policy,
so the hint cannot pin memory or prevent mandatory reads from progressing. Cache
replacement resets the hint; generation identity and full backing charges remain.

System-feature binary SHA256:
`0cbdca0da83cb233b824c5d0674c7f98f22e850ddb095abd8a1e4aea60a005ff`.
The [numeric report](read-ahead-eviction-results-20261029.json) binds every result
to its source and executable, including prior Rust `6e16525` and unchanged Go
`5c0884c` / `80c214e`. Only `src/cache.rs` and `src/filesystem.rs` compiled inputs
change relative to the first refill.

## Matched native measurements

96 unprofiled phases, four rotated mounts per implementation, each fresh plus five
warm revisits; 288,000 full-file checks. The sealed fixture remains 3,000 files /
8,298,727,528 bytes / 97 listings. All phases pass, followed by native hashes and
listing checks. Runtime workers stay at eight, backend at sixteen, application
budget 512 MiB and entry limit 100,000; four readers use 128 KiB requests.
Filesystem/client limits are 8 CPU / 16 GiB, backend 16 CPU / 12 GiB and parent
24 CPU / 28 GiB. No measured CPU throttling or memory-event increments occur.

Warm arithmetic means, twenty phases per implementation:

| Executable | MB/s | p50 ms | p95 ms | p99 ms | Whole FS/client cgroup CPU s | Daemon peak / final-fifth RSS MB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Rust protected refill `d0d7b99` | 1539.143 | 0.076950 | 0.843886 | 2.790580 | 26.333174 | 755.896 / 754.637 |
| First Rust refill `6e16525` | 1472.072 | 0.081720 | 0.918894 | 2.982718 | 27.791241 | 751.789 / 750.405 |
| Go experimental `5c0884c` | 1188.702 | 0.189599 | 0.872853 | 2.500064 | 27.167640 | 1326.374 / 1226.701 |
| Go packaged `80c214e` | 1093.106 | 0.209935 | 0.968654 | 2.855006 | 29.714737 | 1340.962 / 1216.852 |

Compared with the first refill, mean throughput increases 4.56%, p95 decreases
8.16%, p99 decreases 6.44%, and whole FS/client CPU decreases 5.25%. Both p95 and
p99 improve in all four mount pairs; throughput and CPU improve in three, with
the adverse pair retained. Mean daemon RSS rises about 0.55%; this is not a memory
improvement over prior Rust. All four experimental-Go pairs still have worse Rust
p99 (mean +11.62%), despite better p95, throughput, whole CPU and daemon RSS.

Mean warm excess response payload decreases from 83.830174 to 67.202226 MB, but
does not disappear. Range-response counts rise from 3634.75 to 3642.15. Both Go
references have zero excess payload and 3884 range responses in this comparison.
Smaller duplicate ranges can increase request count while decreasing bytes; do
not equate request count with traffic. Required-pressure eviction remains possible.

These are means of phase percentiles, not pooled percentiles; same-mount revisits
are not independent mounts. Final-fifth RSS is not a long-term memory plateau.
The numeric report preserves dispersion, per-mount deltas, client/backend CPU and
RSS, allocator samples and separate cgroup/file/kernel accounting. Changes between
different historical matrices are not attributed solely to this source fix.

## Direct transport evidence and remaining tails

The authoritative v3 socket/FUSE capture has four additional phases / 12,000 full
checks, 136,876 matched reads, no unmatched syscalls and complete HTTP bodies.
Repeated-range union accounting gives 71,266,928 prior bytes versus 59,789,310
candidate bytes. The original two affected files improve, but another file also
has repeats in the candidate. This supports a reduction, not elimination or an
attribution of every eviction caller.

The refill scheduling improvement remains: 633 of 634 unambiguous candidate
noninitial GETs start before consumer entry (632 already have required socket
bytes); prior refill has 617 of 618 before entry. Ambiguous requests are retained
separately. This diagnostic is not an acceptance repetition.

Of 683 candidate tail reads joined to network dependencies, 653 are initial
object reads. Summed service time is 3364.603364 ms: 381.143379 before request,
2646.531543 awaiting response headers, 173.330574 receiving required body bytes,
163.597868 after transport. These are observed stage boundaries, not causal
percentages of the global p99. Instrumented candidate service time is higher
than prior in this capture; it is not hidden by the unprofiled improvement.

Two earlier captures are retained and excluded:

- v1 fails the unchanged error guard on namespace PID helper `-22`, matching
  open upstream [bpftrace #4384](https://github.com/bpftrace/bpftrace/issues/4384).
- v2's group-leader filter compares psutil's expanded executable name against
  truncated kernel `comm`, so daemon events are absent and the FUSE join fails.
- v3 reads raw `/proc/PID/comm`, verifies a unique leader in the owned fixture,
  retains the exact cgroup and FUSE-device filters, and passes complete joins.
  Loss/error checks are not relaxed. Event IDs remain init-namespace PID/TID.

## Verification and next diagnosis

204 Rust tests, 37 Python tests, rustfmt, warning-denying all-target/all-feature
Clippy and the public-source audit pass in Kubernetes. Eleven inserted faults
and a selected Mutator boundary mutation fail their intended assertions. Coverage
and CRAP remain unavailable; the three earlier Dryer candidates remain open.
Actual mounted checks pass 17 ordinary cases and retain known hardlink EIO.
The export verifies 2444 artifact hashes, including explicitly excluded captures,
and 25 compiled inputs. Independent source-row/aggregate validation checks 9424
numeric cells and the resource intervals.

**UNACCEPTED.** The premise that advancing/refining refill alone would close the
remaining p99 gap is insufficient: initial-object dependencies dominate the
observed tail set. The pinned public Go `NewFileHandle` initializes access history
on every open; Rust stores it on the inode across opens. Captured large-file
reopens can therefore start a promoted 100 MiB Rust window while Go starts at
5 MiB. This policy difference is a separate failure-sensitive regression target;
its effect on tails must be measured, not inferred from these stage sums. Full
representative random/hot-set/writer/metadata and long-term compatibility gates
also remain outstanding.
