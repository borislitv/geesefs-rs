# Persistent L2 follow-up qualification — 2026-11-11

**Not qualified; not accepted for production.** The first corrected native fixture and
fixed-work protocol completed all 456 planned mounts: 454 passed the complete
byte/state contract and two failed the required zero-source warm-L2 state.
Returned-byte corruption was not observed. Resource regressions remain alongside
improvements. This preserves, rather than replaces, the
[earlier failed qualification](performance-disk-cache.md).
The subsequent [combined checkpoint](#combined-pipeline-checkpoint) also fails
absolute qualification; its measurements predate the later WOOF repairs.

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

## Diagnosis at the first corrected checkpoint

Random-hot counters identify 212 additional fragment demotions, index growth
1024 → 1236 and 15,564,800 written bytes despite zero source GETs and region
reclaims: complete disk records are republished into L1 only as requested slices.
An independently preserved CPU/scheduling profile reproduced this work. It does
not establish lock contention as the dominant CPU cost or justify removing
checksums. The later bounded covering-record retention correction and its paired
verification are preserved below.

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

## Validated backing correction

The subsequent minimal correction retains the actual validated disk backing and
offset in L1 instead of creating a new fragment identity on eviction. Reply
coverage and entire-backing admission are unchanged. Default/system-allocator
suites pass 502 tests each, with 24 opt-in entries. Independent review kills
four relevant isolated faults; refill proves removal of the canonical live-index
entry followed by persisted readmission, not physical target-region recycling.

Candidate SHA256 `125c2c6b52c8261e2bce8a77df4747a7d8ef1c5fb144b18997a183321f2925e8`
is compared with the earlier `7bd782…` executable, **both L2 enabled**, with the
same fixed work, budgets and eight pairs plus eight baseline A/A controls across
six predeclared groups. All 144 attempts completed: 142 byte/state passes, with
two baseline concurrent timeouts and all 48 candidate mounts passing. Neither
failed baseline was retried. This isolates the correction; it cannot establish
absolute L2-versus-RAM or Go-relative superiority.

All eight warm-random baseline/control mounts reproduce 212 admissions, added
index entries and spill operations; every candidate has zero. Adapter writes
fall from about 15.56 MB to zero. Legitimate eviction admissions remain active.

| Group | Throughput cost, 95% CI | Daemon CPU cost, 95% CI | Scope |
| --- | --- | --- | --- |
| Random L2-hot | 0.926, 0.882–0.976 | 0.884, 0.845–0.932 | Both improve, 8 pairs/8 A-A |
| Random cold-requested | 0.982, 0.931–1.044 | 0.912, 0.868–0.961 | Throughput within noise; CPU controls biased |
| Concurrent cold-requested | 0.973, 0.889–1.075 | 0.784, 0.730–0.847 | Insufficient: 6 valid pairs/6 A-A |
| Eviction | 0.976, 0.951–1.001 | 0.994, 0.968–1.018 | Within observed noise |
| Sequential L2-hot | 1.038, 0.944–1.125 | 1.010, 0.961–1.061 | Within observed noise |
| Sequential L1-hot | 1.047, 0.876–1.235 | 1.049, 0.931–1.200 | Throughput within noise; CPU controls biased |

Warm-random throughput improves about 8.05%, daemon CPU falls 11.60% and target
cgroup CPU falls 9.00%. Its median RSS peak/latter half is
135.402/134.430 → 135.758/134.768 MiB; biased RSS controls preclude a memory
improvement claim. Heap peak is 64.282 → 64.065 MiB, target cgroup peak
260.379 → 259.365 MiB and file latter half 145.608 → 146.009 MiB. Complete
CPU user/system, normalized work, allocator/RSS/cgroup/file/kernel/provider
observations and every adverse/insufficient effect remain in ignored
`benchmark-results/private/backing-paired-20261111/`.

Independent contract review confirms that S3 single-Range requests cannot merge
disjoint missing runs without also fetching cached bytes. It does not prove every
observed attempt minimal or excuse avoidable serialized waits. Likewise bounded
optional waiting cannot guarantee zero source reads under arbitrary external
latency; this does not establish the cause of the historical timeouts or waive
their failed state checks. Kernel file-cache costs are real but are not a proven
universal lower bound for every supported storage strategy. Acceptance remains open.

## Combined pipeline checkpoint

Commit `8d91571dffb22ff2560b7cba21817ce2caa050d9` adds bounded source-gap
overlap and decoded-Box ownership transfer to the backing correction. Its default
jemalloc executable SHA256 is
`e5f68b49d441c5dfa5688aa31b4d4de662c2c0909a09d89305187c242ee1f443`.
All 145 verification inputs and 156 experiment inputs match before/after both
predeclared runs. Harness hash, datasets, fixed work, 64-MiB payload/16-MiB
metadata/256-MiB disk budgets, SeaweedFS 4.51 and child/ancestor limits remain as
above. The added pipeline ownership reduces the L2 maximum effective window
22,282,240 → 22,151,168 bytes; this contractual deduction is not normalized away.

Run A compares the **same e5 executable RAM-only versus L2**. All 456 attempts
complete; 449 pass the full byte/state contract, including 145/152 candidate rows.
Seven candidate advisory-cold zero-source failures retain correct bytes but use
timeout fallback: three sequential, three random and one concurrent mount.
Completed backend misses/errors are zero; recorded maxima are 0.273–1.969 s.
Admission, scheduling and actual I/O shares are not separated. The native
preflight passes, but the matrix correctly exits 1 for rejected qualification.
There are no retries, changed budgets/deadlines or removed failed rows.

| A warm group | Throughput cost, 95% CI | Daemon CPU cost, 95% CI | Scope |
| --- | --- | --- | --- |
| Sequential L2 | 0.747, 0.705–0.782 | 1.189, 1.132–1.241 | Throughput improves; CPU inconclusive |
| Random L2 | 1.087, 1.034–1.130 | 1.281, 1.245–1.316 | Throughput inconclusive; CPU regresses |
| Concurrent L2 | 1.005, 0.989–1.024 | 0.872, 0.849–0.903 | Throughput within noise; CPU improves |
| Capacity eviction | 2.633, 2.483–2.792 | 2.357, 2.223–2.506 | Both regress |

Median baseline → candidate memory, MiB; simultaneous daemon+client RSS excludes
the separately accounted provider. Cgroup includes charged file/kernel memory.

| A warm group | RSS peak | RSS latter half | Allocator peak | Cgroup peak | File latter half |
| --- | --- | --- | --- | --- | --- |
| Sequential L2 | 161.008 → 131.473 | 157.619 → 128.298 | 79.312 → 58.179 | 143.902 → 255.840 | 1.632 → 145.530 |
| Random L2 | 150.316 → 139.340 | 148.804 → 138.364 | 79.693 → 64.069 | 128.504 → 263.262 | 1.630 → 145.655 |
| Concurrent L2 | 166.152 → 144.975 | 161.475 → 139.104 | 80.925 → 64.480 | 145.258 → 272.225 | 1.792 → 146.658 |
| Capacity eviction | 150.275 → 132.207 | 148.078 → 126.631 | 77.855 → 57.980 | 130.438 → 124.170 | 1.487 → 17.122 |

Run B compares old `125c2…` against e5, **both L2 enabled**: 96/96 byte/state
passes, all 32 new-candidate mounts, eight independent pairs plus eight actual
old-baseline A/A controls per group. It isolates the combined correction, not
Box transfer from overlap or its window deduction, and not L2/RAM superiority.

| B group | Throughput cost, 95% CI | Daemon CPU cost, 95% CI | Scope |
| --- | --- | --- | --- |
| Capacity eviction | 0.889, 0.872–0.906 | 1.030, 1.016–1.043 | Throughput improves; CPU within A/A noise |
| Random L2-hot | 0.966, 0.934–1.003 | 0.992, 0.955–1.031 | Both within noise |
| Sequential L2-hot | 0.948, 0.852–1.045 | 0.990, 0.921–1.052 | Both within noise |
| Concurrent cold-requested | 0.969, 0.938–1.004 | 0.994, 0.965–1.028 | Both within noise |

Eviction throughput improves about 12.47%, but consumed CPU does not improve.
Five inconclusive and three biased-control metric results remain. Memory is not
an accepted win: eviction RSS peak/latter half 132.070/125.103 → 131.980/126.415
MiB, heap peak 57.537 → 58.587 MiB, cgroup peak 123.094 → 125.123 MiB and file
latter half 16.179 → 17.428 MiB. All CPU user/system/client/cgroup/provider,
normalized work, latency tails, heap/RSS/cgroup/file/kernel values, adverse
effects and A/A intervals remain in the ignored raw tables, without filtering.

Evidence: `benchmark-results/private/combined-qualification-20261111/{A,B}/`
contains `FINALREPORT.md`, `METRICS.csv`, `PAIRED-CIS.csv`, `ARM-MEDIANS.csv`,
manifests, exact binary hashes and every failed row. All 552 owned caches and
process/mount/cgroup workspaces were cleaned up. DONTNEED remains advisory; no
Go 0.43.9, target LXC or large populated-restart acceptance follows.

### Later WOOF corrections are not measured by these tables

Full-PR WOOF at e5's source commit retains two P2 defects after falsification:
false count-admission ENOMEM after a previously pinned prefix becomes available,
and renewed optional disk waits before the first source GET of a qualified
producer range. Admission-local bounded scans and range-local fallback correct
them; 518 default/all-features tests pass, with 24 opt-in entries. Ten executed
fault controls reject relevant failures. These checks neither identify those
defects as the cause of the seven A failures nor reclassify any failed row.
There is no post-fix paired performance claim or production acceptance.
