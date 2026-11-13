# Persistent L2: frozen local qualification result

**Qualification failed; the feature remains unaccepted for production.** This
records negative and partial evidence, not a successful performance checkpoint.

## Source, workload and resources

Runtime: `fac68b40d8a31c15dcd03d87f0537068137c22ff`, Rust 1.99.1,
release/default jemalloc. Executable SHA256:
`cb9deac6b527ab0de2ab48416bea1a76805a891ee208650d17ce93ed6e160f8f`.
The same executable runs RAM-only baseline/control and L2 candidate. This
compares the feature plus residency policy, **not** metadata compaction or Go.

Every measured arm uses 64 MiB payload, 16 MiB metadata, two CPUs and a 2-GiB
child cgroup limit. Dataset is 128 MiB, disk capacity 256 MiB, eviction object
512 MiB, excluded pressure 128 MiB. Planned L2 funding is 5672960 bytes; the L1
policy ceiling falls from 62914560 to 57241600 bytes. These ceilings include
revocable loans, not guaranteed selected-data residency. Actual resident/loan
samples accompany the raw measurements.

Population has eight fixed fresh mounts with 128-KiB warmup-only read-ahead,
matched across arms; measurement mounts keep defaults including the initial
5-MiB window. Warmup mounts do not count as independent measurement pairs.
Scratch is disk-backed; per-case cache cleanup follows unmount and lies outside
the timed interval. Original fixture-OOM attempts remain retained separately.

## Complete execution, incomplete qualification

All 456 planned cases were attempted: **239 correct/measured; 217 failed**.
Nine groups reach eight matched pairs and eight same-executable A/A controls.
Sequential cold has only seven pairs; nine large-hot/eviction groups have none.

- 192 failures: initial population payload read exceeds its 30-second deadline.
- 24 failures: excluded-pressure population read exceeds that deadline.
- One failure: a partially written allocator JSON frame was read live. The later
  harness correction was not applied retroactively; that row remains failed.

Among the 135 declared metric screens in nine fully paired groups: 43 regress,
10 improve, 52 are within noise, 14 have biased controls and 16 are inconclusive.
These correlated metrics are not 135 independent experiments. No group passes
all its screens; declared large L2-hot superiority targets were not measured.
Requested page-cache cold is advisory, not verified media-cold.

## Selected paired results

All 95% intervals below are **cost ratios, lower is better**: baseline/candidate
for throughput, candidate/baseline for memory and CPU. They use the existing
paired framework and A/A screening, not post-hoc practical margins.

| Group, eight pairs and A/A | Throughput cost CI | Peak RSS cost CI | Steady RSS cost CI | Daemon CPU cost CI | Whole-case CPU cost CI |
| --- | --- | --- | --- | --- | --- |
| Metadata | 0.9891–1.0165, noise | 0.9980–1.0166, noise | 0.9942–1.0178, noise | 0.9715–1.0024, noise | 0.9708–1.0016, noise |
| Sequential L1-hot | 0.9226–1.0214, noise | 1.0128–1.0160, regressed | 1.0128–1.0159, regressed | 0.9823–1.0796, noise | 1.0001–1.0068, noise |
| Concurrent cold | 0.9156–0.9275, improved | 0.9101–0.9151, improved | 0.9074–0.9120, improved | 1.1182–1.1500, regressed | 1.0764–1.0996, regressed |
| Writer/fsync | 0.9897–1.0439, noise | 0.9999–1.0099, noise | 1.0014–1.0095, biased | 0.9919–1.0609, biased | 0.9855–1.0449, biased |

Concurrent cold's lower process RSS is not a whole-memory win: cgroup peak cost
CI is **1.6940–1.7036**, cgroup steady **1.4661–1.4780**, and kernel steady
**2.5961–2.6503**, all regressed. Changed selected residency is also a confounder
for interpreting its lower process RSS as an independent implementation benefit.

## Memory and CPU scope

Memory medians below are MiB. RSS sums daemon and fresh client; allocator is
daemon jemalloc only. Cgroup peak covers mount lifetime including startup/warmup;
steady/file/kernel are workload samples, not proven long-term plateaus. Shared
RSS pages may be counted twice while cgroup pages are charged once.

| Group / arm | RSS peak / steady | Allocator peak / steady | Cgroup peak / steady | File / kernel steady | Daemon CPU user / sys s | Client CPU user / sys s | Cgroup CPU user / sys s |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Sequential L1 baseline | 81.613 / 81.563 | 19.783 / 18.860 | 54.199 / 54.011 | 0.018 / 0.713 | 0.130 / 0.425 | 0.380 / 0.070 | 0.518 / 0.487 |
| Sequential L1 candidate | 82.775 / 82.683 | 21.412 / 20.348 | 57.611 / 57.333 | 3.078 / 0.735 | 0.140 / 0.450 | 0.360 / 0.075 | 0.505 / 0.510 |
| Concurrent cold baseline | 148.363 / 146.664 | 78.161 / 77.334 | 127.783 / 125.166 | 1.685 / 1.251 | 1.550 / 1.340 | 1.375 / 0.070 | 2.936 / 1.392 |
| Concurrent cold candidate | 135.346 / 133.463 | 64.071 / 63.281 | 216.768 / 184.137 | 70.658 / 3.280 | 1.780 / 1.465 | 1.365 / 0.070 | 3.174 / 1.520 |

For sequential L1, both arms retain 1 MiB and zero loans. Concurrent cold retains
62677529 versus 40961548 visible bytes and 10433331 versus zero loan bytes.
Whole-case normalized CPU is 34.421 versus 37.336 s/GiB for concurrent cold.
CPU is consumed time, not elapsed time; the Moto provider is outside the child
cgroup. Concurrent L1 shows actual throttling and biased throughput controls.
Full per-case CPU, tails, RSS/allocator/cgroup samples, normalized cost, control
intervals and failed rows remain in ignored raw evidence.

## Native correctness and remaining diagnosis

A separate small-object native protocol passes on the same runtime: after
restart, all 1048576 package bytes come from L2 with zero source bytes/GETs.
nginx full/Range/EOF, rsync read/write, metadata, fsync error/recovery and genuine
startup-preallocation ENOSPC fallback also pass. This does not prove a large warm
working set, native region eviction or ongoing postallocation I/O-error recovery.

Read-only inspection of Moto 5.1.17 identifies a fixture bottleneck: its GET path
reads the complete object buffer under a key lock before applying Range slicing.
Thus logical 128-KiB priming can repeatedly read a 128-MiB provider buffer. This
is a code-path diagnosis, not measured wire amplification or a profile proving
all observed cost belongs to Moto. It does not cancel the measured runtime
resource regressions. Qualify a supported bounded/streaming fixture and isolate
those side costs before another large matrix; do not increase budgets or call a
longer timeout a runtime improvement.

Executed matrix overlay SHA256:
`04d3478fb6734b5ae60e183523b358cb1fc44b6f832c4361771eb7da91b67ea5`.
Later harness fixes are separately hashed and do not rewrite this result.
All owned mounts, workers and cgroups were cleaned up. Go-relative and target
environment acceptance remain open; no served reader was changed.
