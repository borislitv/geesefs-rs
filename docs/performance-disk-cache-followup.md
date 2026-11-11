# Persistent L2 follow-up qualification — 2026-11-11

**Not qualified; not accepted for production.** The corrected native fixture and
fixed-work protocol completed all 456 planned mounts: 454 passed the complete
byte/state contract and two failed the required zero-source warm-L2 state.
Returned-byte corruption was not observed. Resource regressions remain alongside
improvements. This preserves, rather than replaces, the
[earlier failed qualification](performance-disk-cache.md).

## Frozen inputs and measurement scope

- Base revision `450d699c0d6b2082de314271160c5cac4b5e4fd0` plus the explicitly
  content-addressed runtime changes; runtime/Cargo manifest SHA256
  `a5815294550308e91d5fa85521bd335e1cbb2a652d2b8cfd82c287affc4294b5`.
- Default-jemalloc Rust 1.99.1 executable SHA256
  `7bd782e1c33c0540945ac10c635d505f4e63861ba4a4ac500310e8ad2a78d0f9`.
- Harness `bench/disk_cache.py` SHA256
  `3f8e63a3d8a45562ea452eba9d3e318b8cf5f2d8a495aba506daf612093b00db`;
  configuration SHA256
  `b4e3d287358089577448c1e9a22cbbb9ace30e47bb1eaae7760b1db994df8ad0`.
- Same executable RAM-only baseline/A-A versus L2 candidate. Payload 64 MiB,
  metadata 16 MiB/100,000 entries, disk 256 MiB, main dataset 128 MiB, pressure
  512 MiB, four readers. Effective L1 policy ceilings are 62,914,560 and
  57,241,600 bytes; the candidate funds 5,672,960 bytes. Actual residency and
  revocable-loan use are observed separately.
- Daemon plus fresh client share a two-CPU/2-GiB child cgroup; the owned ancestor
  has eight CPUs/6 GiB. Qualified SeaweedFS 4.51 is outside the child: sealed RO
  for reads and separate disposable RW for writer checks. Provider/ancestor
  CPU, memory and I/O are retained separately, not omitted whole-machine costs.
- Fixed work across arms: eight warm cycles, 64 passes for the 1-MiB sequential
  L1 case, one pass for cold-requested/eviction/writer. Hot concurrent performs
  exactly 1 GiB/8192 operations; cold-requested concurrent 128 MiB/1024 operations.
  Identical capacity preparation and the later witness probe are outside the
  timed interval; probe I/O is separate.
- Eight independently mounted pairs and eight same-executable controls per
  group, except concurrent/cold-requested with six complete pairs. Resampling
  uses mounts, not individual calls. DONTNEED is advisory, not verified cold media.

Both independent reviews passed before freezing: Rust ownership/race/deadline
tests rejected three reintroduced faults; harness tests rejected final-source
success, variable work and mixed I/O boundaries. All 71 frozen hashes remained
unchanged through the run. The CLI returned 1 for rejected qualification, not an
aggregate timeout or crash. All owned mounts, processes, workspaces and caches
were cleaned up.

## Remaining state failures

Concurrent/cold-requested candidate mounts 0 and 1 returned correct fixed-work
bytes but produced eight and nineteen timeout fallbacks, respectively. There
were no completed backend misses/errors; completed operation maxima were
754 ms and 1.350 s against the existing 250-ms optional read deadline. Source
GET attempts were six and fourteen. This does not prove missing inventory or
identify admission, scheduling and device time separately.

Four later traced reproductions passed with zero GET/timeout and longest actual
disk reads of 46–93 ms. Slow reads waited on kernel buffered pages. Passing
reproductions do not explain or erase the two failures; debug runs also show
instrumentation-induced throttling. No longer deadline or storage-delay exemption
is adopted from this evidence.

## Representative paired effects

Ratios below one favor the candidate; throughput is expressed as inverse cost.
Intervals are 95% paired bootstrap CIs over independent mounts. These rows expose
both improvements and regressions, not an aggregate pass. All 19 groups and A/A
effects remain in the raw tables; biased controls are inconclusive.

| Warm group | Throughput cost, CI | Daemon CPU cost, CI | Interpretation |
| --- | --- | --- | --- |
| Sequential L2 | 0.690, 0.645–0.733 | 1.147, 1.102–1.196 | Throughput improves; CPU non-inferiority inconclusive |
| Random L2 | 1.162, 1.113–1.204 | 1.402, 1.355–1.457 | Throughput and CPU regress |
| Concurrent L2 | 1.001, 0.977–1.018 | 0.952, 0.929–0.978 | Throughput within noise; CPU improves |
| Capacity eviction | 2.984, 2.736–3.333 | 2.460, 2.348–2.592 | Throughput and CPU regress |

Memory below is median baseline → candidate, in MiB. RSS is simultaneous
daemon+client scope, not provider-inclusive; cgroup includes its charged file
and kernel memory. Allocator and file observations are not interchangeable with
RSS. Peak and latter-half sampled RSS remain visible beside CPU effects.

| Warm group | RSS peak | RSS latter half | Allocator peak | Cgroup peak | File latter half |
| --- | --- | --- | --- | --- | --- |
| Sequential L1 | 82.469 → 83.059 | 82.446 → 83.039 | 20.227 → 20.088 | 54.693 → 55.287 | 0.016 → 0.016 |
| Sequential L2 | 160.592 → 130.174 | 156.718 → 126.124 | 79.279 → 57.631 | 142.492 → 249.592 | 1.617 → 138.002 |
| Random L2 | 149.215 → 137.508 | 147.579 → 136.242 | 79.559 → 64.232 | 128.920 → 262.777 | 1.634 → 143.980 |
| Concurrent L2 | 164.133 → 143.863 | 161.199 → 139.162 | 80.705 → 64.589 | 143.945 → 267.947 | 1.794 → 145.946 |
| Capacity eviction | 150.477 → 132.141 | 149.223 → 126.714 | 77.134 → 57.226 | 130.590 → 124.240 | 1.492 → 16.929 |

Lower process RSS is therefore not a whole-memory win. Sequential L1 has a small
but measured RSS regression. Other A/A-biased or incomplete effects are not
promoted to non-regression. The changed provider/preparation/fixed-work protocol
also prevents a numerical speedup claim against the earlier Moto matrix.

## Diagnosis and remaining work

Random-hot counters identify 212 additional fragment demotions, index growth
1024 → 1236 and 15,564,800 written bytes despite zero source GETs and region
reclaims: complete disk records are republished into L1 only as requested slices.
An independently preserved CPU/scheduling profile reproduced this work. It does
not establish lock contention as the dominant CPU cost or justify removing
checksums. A bounded covering-record retention correction needs its own tests
and paired verification.

Eviction lowers fetched source bytes but increases median GET attempts from
37 to 935.5. Alternating disk hits and missing ranges fragment source requests;
bounded read planning is a concrete investigation target. Do not attribute the
whole slowdown to storage bytes without profiling. Original timeout failures,
ongoing postallocation I/O errors, controlled media-cold behavior, Go 0.43.9 and
target-environment acceptance remain open.

Raw commands, all failures, complete CPU user/system/throttling and normalized
work, allocator/RSS/cgroup/file/kernel/provider tables and source receipts remain
in ignored `benchmark-results/private/harness-r123-20261111/`, with separate
`l2-timeout-attribution-20261111/` and `l2-cost-attribution-20261111/` evidence.
No deployment or serving configuration was changed.
