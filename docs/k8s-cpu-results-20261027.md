# CPU-accounted Kubernetes diagnostic: completed, comparison invalid

**All 50 phases executed; compatibility/performance acceptance remains false.**
The original manifest passed its initial native generation inventory. During
the 2,113-second matrix, 214 objects disappeared: the final independent native
HEAD inventory returned 404 for them. Different phases therefore read different
byte totals. Neither lower CPU totals nor higher observed throughput in this
run establish a win on equivalent work.

## What was measured

Five balanced orders, five executables, fresh/warm-revisit phases; the same
3,000-file, 8,298,727,528-byte manifest and 96 listing oracles as the earlier
diagnostic. Limits remained 2 CPU, 4 GiB cgroup, 512 MiB user cache, 100,000
entries and four readers with 128 KiB reads. Both Go baselines, the retained
original Rust and matched-native prior/new Rust builds have exact source and
executable hashes in [the whitelisted receipt](k8s-cpu-results-20261027.json).

CPU was captured from phase one: daemon/client user and system counters, whole
cgroup usage, average occupied cores, CPU/GiB or operation and throttling. Raw
boundaries distinguish listing, read workers, workload body and whole client
startup/exit windows. Two hundred phase windows were independently recomputed
from the raw counters. All 483 indexed evidence files passed hash verification.
The experiment runner is separately hash-bound from the public accounting helper.

Fresh means a new process/mount, not controlled cold host/remote caches. Cgroup
usage includes controller, client, daemon, sampler and occasional monitoring.
Process counters have clock-tick precision. Throttling is reported separately
from useful work; cgroup CPU must not be added to its constituent process totals.

## Failed-workload observations, not paired acceptance

Each row includes all five phases, including failures. MB is decimal; CPU values
are seconds averaged over the read/stat/open/hash window, excluding the preceding
listing and client startup. RSS is the filesystem daemon's sampled memory over
the whole client phase, including listing; it is not an all-process RSS total.
The JSON additionally retains user/system splits,
cgroup dispersion, whole-phase CPU and the complete failure counts. These means
do not compensate for differing bytes or errors.

| Implementation | State | Verified files / 15000 | Observed MB/s | Read p99 ms | RSS peak MB | RSS steady MB | Daemon CPU s | Client CPU s | Cgroup CPU s | Throttled s |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| go-experimental | fresh | 14358 | 219.215 | 27.715 | 1138.208 | 1092.696 | 40.470 | 14.346 | 55.364 | 2.457 |
| go-experimental | warm-revisit | 14358 | 268.281 | 23.813 | 1217.713 | 1109.181 | 32.552 | 14.980 | 47.970 | 5.516 |
| go-packaged | fresh | 14358 | 206.098 | 25.030 | 1241.598 | 1203.881 | 46.170 | 13.870 | 60.555 | 3.236 |
| go-packaged | warm-revisit | 14358 | 243.822 | 24.123 | 1331.284 | 1222.838 | 39.196 | 14.980 | 54.622 | 5.799 |
| rust-original | fresh | 14400 | 184.440 | 40.645 | 547.730 | 397.017 | 30.134 | 13.706 | 44.434 | 0.749 |
| rust-original | warm-revisit | 14358 | 260.402 | 33.785 | 757.681 | 339.713 | 25.980 | 14.048 | 40.446 | 2.083 |
| rust-prior | fresh | 14572 | 196.463 | 36.821 | 548.506 | 363.125 | 29.814 | 13.222 | 43.574 | 0.801 |
| rust-prior | warm-revisit | 14572 | 275.760 | 33.894 | 689.815 | 363.449 | 26.668 | 14.032 | 41.071 | 2.159 |
| rust | fresh | 14572 | 184.242 | 37.757 | 637.986 | 471.993 | 30.980 | 13.546 | 45.099 | 0.642 |
| rust | warm-revisit | 14572 | 269.400 | 33.715 | 818.549 | 473.964 | 27.026 | 14.810 | 42.270 | 2.389 |

### Total memory, file cache and kernel memory

Mean of all five phases, including failures, in decimal MB. Peak is the sampled
per-phase maximum at 50 ms intervals; steady is the mean of the final 20% of
samples, not proof of a stationary plateau. Cgroup totals include the client,
controller and cache; standalone client/controller RSS was not collected and
must not be inferred as zero. Cgroup memory is near its 4 GiB limit even when
daemon RSS is lower. Source mutation still invalidates comparative acceptance.

| Implementation | State | Cgroup peak MB | Cgroup steady MB | File cache steady MB | Kernel steady MB |
| --- | --- | ---: | ---: | ---: | ---: |
| go-experimental | fresh | 4298.206 | 4293.619 | 3100.860 | 42.625 |
| go-experimental | warm-revisit | 4306.436 | 4293.582 | 3080.845 | 45.894 |
| go-packaged | fresh | 4299.583 | 4293.535 | 2988.769 | 43.381 |
| go-packaged | warm-revisit | 4306.630 | 4292.266 | 2965.596 | 46.135 |
| rust-original | fresh | 4294.942 | 4249.809 | 3738.894 | 39.566 |
| rust-original | warm-revisit | 4295.760 | 4239.740 | 3784.097 | 41.525 |
| rust-prior | fresh | 4296.790 | 4227.619 | 3750.718 | 39.677 |
| rust-prior | warm-revisit | 4297.282 | 4232.622 | 3753.246 | 41.972 |
| rust | fresh | 4295.868 | 4249.823 | 3664.614 | 39.303 |
| rust | warm-revisit | 4299.947 | 4243.129 | 3653.122 | 42.191 |

## Correctness and source changes

- 144,478 of 150,000 attempted file checks passed full independent SHA-256 checks.
- 5,516 ENOENT failures match the 214 paths with final native 404 responses.
  They remain failures in the report rather than being removed or replaced.
- Six further failures were EAGAIN on the retained original Rust executable,
  on paths without a final native generation error. Their cause remains open;
  remote deletion is not an established explanation for these six failures.
- 146 listing mismatches occurred. Post-run native listing independently found
  four changed directories, one added name and 375 removed names. This establishes
  source mutation, not the exact reason for every earlier listing mismatch.
- No object was written by the benchmark. The mounts used RO/no-expire/no-detect;
  no global cache drop, forced GC, served mount or writer configuration change.
  All owned mounts were normally unmounted and evidence retrieved.

## Metadata investigation and next reproducible unit

The new Rust run does not establish a preload throughput or RSS improvement.
Its observed fresh listing time averaged 11.482 seconds versus matched prior
8.581, while warm listing averaged 0.177 versus 4.400. Observed daemon peak RSS
was higher for the new binary. Because namespace contents changed during the
matrix, these are investigation leads, not a controlled causal comparison.

Post-run native first-page inspection covered 1,261 manifest parent directories;
2,314 of the original 3,000 names appeared on their first page. This is only an
upper bound on potentially useful metadata preloads, not an actual request trace;
the 214 deleted objects are still in that denominator. Current preload only
consumes immediate authoritative objects. Go's public lookup path uses targeted
recursive slurp around the key; the algorithms are not equivalent.

Before another acceptance comparison, obtain an immutable fixture generation or
version-pinned source with complete payload/listing oracles. Keep this failed
matrix intact. Reproduce the six EAGAIN failures independently and capture
HEAD/LIST counts and read-tail traces; only then change the responsible Rust
operation and repeat the bounded, failure-sensitive tests and matched matrix.
No SDK/language/network performance floor has been established. Go heap series,
full writer compatibility and the complete representative matrix remain open.
