# Paired performance screening — 2026-11-02

**UNACCEPTED.** The operator approved replacing strict all-metric/all-pair wins
with paired confidence intervals and actual same-executable controls. This
corrects inference, not measured values or filesystem contracts.

## Why the earlier decision rule is withdrawn

The classic trial's one-loop binary and observation-age baseline are a useful
near-A/A check, not a true control: their binaries and source inputs differ.
The old strict joint predicate also rejects every such pair. Metadata throughput
ratios range from 0.714 to 1.208 and writer p99 reaches 1.932 times. Inline trial
writer/tail variations therefore cannot be called caused regressions solely from
the four-pair screening result. Four signs in one direction have two-sided sign
test p=0.125. Thousands of calls do not create thousands of independent mounts.

Extra loops remain disfavored by separately demonstrated per-loop ownership
(16 MiB+4 KiB) and consistently lower four-loop random rates. Inline first-poll,
lookup-probe overlap and cached-observation reuse are **suspended/inconclusive**,
not demonstrated joint regressions. Their implementations remain absent; raw
data and source archives are retained. This does not authorize reinstatement.
First-poll is subsequently rejected on a separate liveness contract (below).

## Reproduction and estimands

- Use at least eight mount blocks, with baseline A, the **same executable** A'
  and candidate B. Rotate all three orders; record executable/source hashes,
  generations, effective budgets, CPU placement and normal cleanup. Controls
  from another workload, compiler, kernel, allocator or observer are not margins.
- Preserve raw per-call latencies, but bootstrap whole paired mounts. Report
  per-mount geometric-mean cost ratios and percentile bootstrap 95% intervals
  (4,096 deterministic resamples). Lower cost is better; invert throughput.
  Separately report sign-test p and pooled raw tails; do not confuse their
  estimands. Small-sample bootstrap intervals are screening evidence, not proof
  of population coverage or a memory plateau.
- Estimate a metric's symmetric noise factor from the 95th percentile of
  absolute log(A'/A) ratios. A biased A/A mean interval that excludes unity blocks
  inference. The control arm must be chosen before examining B; do not enlarge
  allowances after observing a candidate loss. Record the calibration with the
  full block dataset. A future confirmatory run must lock calibration in advance.
- The comparison harness declares throughput and combined steady RSS as primary
  Go-relative targets: their upper cost-ratio CI must be below one. Other required
  metrics need an upper CI within both their calibrated noise factor and their
  **predeclared practical non-inferiority factor**. Supply those factors in the
  workload JSON's `non_inferiority_margins` metric map before running the candidate;
  no production percentages are invented in the default workload. Missing margins
  yield `within_noise`, not `non_inferior`, and do not pass a secondary metric.
  Pooled p50/p95/
  p99 above that factor also block screening, even if mean mount percentiles look
  favorable. Missing controls/metrics and unresolved intervals fail closed.
- Noise-relative non-inferiority only means the experiment cannot distinguish a
  cost within the measured control envelope. It is **not** equivalence, proof of
  no regression, or a production loss budget. Excessive noise requires longer,
  better-isolated controls and an explicit practical margin before deployment.
- Keep sparse tail samples visible. A p99 of one/five writer operations is a
  maximum diagnostic, not a stable tail estimate. Use many independent files and
  report the number of write+fsync calls. Warm stat loops dominated by kernel
  dcache do not measure userspace metadata CPU; count actual FUSE operations in a
  separate instrumented control.

`bench/paired.py` uses standard-library sampling/math APIs; `bench/compare.py`
retains provenance, compatibility, raw-byte and complete-accounting requirements,
but now schedules a real `go-control` arm and requires eight repetitions. A
successful statistical screen alone still cannot close actual-service, Linux/S3,
sustained-memory or LXC acceptance. Historical reports are never overwritten.

## Review measurements, 2026-11-02

All rows below are short screening evidence on the same VM fixture (Linux 7.3.1,
affinity 0-63, no CPU quota, SeaweedFS loopback, unchanged 64 MiB/100,000-entry
budgets). They are not LXC, real-S3, sustained or acceptance results.

**Identical-executable writer control.** Eight rotated blocks of the archived
pre-inline binary (A, A'), the archived first-poll binary (B) and both Go
references: 40 mounts, 150 independently remote-verified 1 MiB write+fsync files
each (6,000 files), normal cleanup, zero reclaim/OOM/throttle deltas. A'/A noise
factors are writer throughput 1.34, p95 2.09 and p99 5.15; CPU 1.11-1.13; RSS and
cgroup memory 1.04-1.08. B/A is `within_noise` for every metric (throughput cost
0.977, CI 0.934-1.023; p99 0.836, CI 0.487-1.471). The earlier "writer tail/RSS
regression" of first-poll is therefore not reproduced; it is also not proven
absent, because no practical margin was predeclared.

Against Go (no Go A'/A arm in this run, so these are descriptive cost ratios with
mount-bootstrap CIs, not gate results), retained-pre-inline Rust uses 0.53x daemon
late RSS, 0.40x/0.29x daemon CPU and 0.90x/0.88x p50 write+fsync latency of the
experimental/packaged Go builds, while total writer throughput costs 1.20x versus
experimental Go (CI 1.14-1.26) and 1.07x versus packaged Go (CI 0.99-1.15).

**Writer RPC actors.** Calibrated request-header traces (three files each, zero
dropped packets) show per file: Rust 5 HEAD + 2 LIST + PUT, packaged Go 2 HEAD +
2 LIST + PUT, experimental Go at most one prefix LIST + PUT. The harness adds the
independent GET; unlink adds DELETE. Rust's extra requests are kernel LOOKUP
(object, marker, implicit-directory LIST) repeated by create validation, plus
post-commit HEAD. Experimental Go relies on its cached listing and can acknowledge
unlink before sending DELETE. These are unequal create/delete contracts, so its
writer rate is not a correctness-equivalent target without that analysis.

**Per-packet delay sensitivity.** Owned network namespace, netem only on the S3
port (SeaweedFS internal RPCs undelayed). With 5/20 ms per packet, Rust phase time
grows to about 140/485 ms per file versus 53-56/179-310 ms for experimental Go;
the RPC count dominates once RTT is milliseconds. Three files per arm only:
direction is clear, magnitudes are not.

**Receive-thread liveness.** A new native contract holds a real inode mutex,
queues an O_DIRECT read and requires an unrelated FUSE request to complete. The
retained classic dispatch passes; the archived first-poll mutation fails with a
2-second timeout because the sole receive thread blocks in the polled read. First
poll is therefore rejected as implemented, independently of performance noise.

## Next diagnosis

1. Reassess archived inline-read, lookup and observation-reuse runs descriptively;
   four-pair confidence intervals cannot turn them into adopted optimizations.
2. Run identical-binary controls with many checked writer files. Trace each RPC
   actor separately: LOOKUP, create race validation, commit validation, independent
   harness GET and cleanup DELETE. Preserve necessary durability and freshness.
3. Add 5/20 ms **per-packet** loopback netem only in an owned private network
   namespace. Measure request counts and latency sensitivity separately from
   uninstrumented comparisons. This is not a real S3 RTT or LXC acceptance.
4. Verify actual read-only target-container/service behavior when access is
   available. Do not shape host interfaces or replace a served mount.
