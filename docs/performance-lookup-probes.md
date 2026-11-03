# Suspended lookup-probe overlap experiment — 2026-11-01

Status after 2026-11-02 review: **SUSPENDED / INCONCLUSIVE; runtime changes absent**.
The [revised protocol](performance-methodology.md) supersedes the original strict
four-pair rejection below; adverse samples remain intact, not established causal
losses. The retained runtime at the original checkpoint remains
`1b13033c3f161b9502c7a23870b645ca871ab2e3`, with its
[current mixed-control results](performance-observation-age.md) still **UNACCEPTED**.
This short experiment does not establish a speed or memory improvement over Go.

## Cause, experiment and decision

Checked writer traces contain three distinct validation actors: kernel/FUSE
LOOKUP performs object HEAD, marker HEAD and implicit-directory LIST; create
validation repeats those three requests; successful PUT retains post-commit HEAD.
The fixture independently GETs/checks the written bytes, then unlink DELETEs them.
Linux 7.0 `fuse_atomic_open` and fuser 0.18.0 keep LOOKUP and CREATE separate.
Skipping create validation changes remote-create race behavior; retaining a miss
changes the default zero negative-cache policy. Neither shortcut is applied.

The experiment overlaps marker HEAD and implicit LIST after a missing object HEAD,
using Tokio 1.54.1 `select!` only when an existing metadata-resolution slot is
spare. Marker metadata/errors retain priority; marker success cancels an irrelevant
LIST, cancellation owns both futures, and occupied slots retain serial progress.
No configured TTL, dataset, payload budget, integrity or durability check changes.

The initial public regression fails on zero LIST starts before a gated marker
response, versus one required start. A real SDK/HTTP fixture withholds marker HEAD
until LIST arrives; deliberately serializing the probes fails that test. Returning
the completed empty LIST instead of authoritative marker data also fails. Both
allocator suites pass 268 experimental Rust cases and 75 benchmark Python cases;
three matching native client variants pass available contracts. Clippy, formatting,
public-source audit and duplicate checks pass. New helpers have CRAP 4–6, but
whole-file CRAP still fails. One selected implicit-presence mutation is killed;
this is not broad mutation acceptance.

Four rotated writer pairs nevertheless lose throughput once, p95/p99 three times,
and peak/late daemon RSS in all four repetitions. Late live allocation, combined
RSS and cgroup memory also increase in every repetition. The original decision
restored all runtime/test inputs to the verified prior source. Experimental tests
and frozen sources remain with ignored evidence; the retained suite has 260 Rust
cases, not the experiment's 268.

The unresolved premise is that overlapping existing requests is sufficient for a
joint throughput/tail/CPU/memory improvement. It removes neither validation actor
nor RPC: a separate checked three-cycle trace still records 15 HEADs, six LISTs
and three each PUT/GET/DELETE. The overlap mechanism is verified; the allocation
or connection lifetime responsible for the RSS increase is not yet attributed.
Do not substitute another scheduling change for that missing ownership analysis.

## Source and measurement scope

- Prior release SHA256:
  `df24b6bdd42920b7989db2ed7c44855b9fc2bb1cf53292ee60ce023eb42138a8`.
- Rejected trial SHA256:
  `30c2c29504c22516b4252f099353fc7a8903b13f2918970338894e35f6c5c2e5`.
  It is an uncommitted experiment based on documentation commit `3f46ecd`;
  86 frozen source/build/test inputs are retained. Only `src/filesystem.rs`
  differs among runtime inputs. It is not a delivered release.
- Same Linux VM/SeaweedFS 4.51, eight workers, unchanged 64 MiB payload budget and
  100,000-entry limit; equal unbounded CPU and 16/12/32 GiB filesystem/client,
  backend and parent memory limits. Four repetitions alternate prior/trial order.
- Each mount performs 150 checked 1 MiB writer operations. All 1,200 phases and
  eight final payload full-byte/generation checks pass. Owned mounts/PIDs exit
  cleanly; selected interval quota/reclaim/high/max/OOM counters do not increase.
  No build, quality check or profile overlaps measured windows. Shared-host
  variance and the short fixed-work scope remain explicit.
- Rate is total bytes divided by total phase time, including create, remote
  verification and unlink. Latency samples cover write+fsync, not the earlier
  create/lookup portion; percentiles pool all 150 calls per mount. Cgroup CPU
  normalized per operation is its table total divided by 150. There are no Go
  or representative sustained mixed comparisons for this rejected trial.
- Memory peaks are sampled; late means the temporal latter half of each short
  window, not a plateau. Combined RSS is pointwise daemon+client. Process RSS,
  allocator live bytes and cgroup charges differ in scope and are not additive.

## Complete paired measurements

| Repetition | Binary | MiB/s | p50 ms | p95 ms | p99 ms | Maximum ms | Daemon user/system CPU s | Client user/system CPU s | Cgroup CPU s |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 0 | Prior 1b13033 | 27.07 | 19.94917 | 27.42853 | 28.91041 | 29.44733 | 0.66/0.42 | 0.75/0.17 | 1.99 |
| 0 | Rejected trial | 25.16 | 23.85454 | 28.24535 | 29.42366 | 29.68227 | 0.70/0.47 | 1.00/0.20 | 2.36 |
| 1 | Prior 1b13033 | 24.66 | 23.35255 | 27.57523 | 28.85994 | 29.63367 | 0.72/0.41 | 0.90/0.19 | 2.22 |
| 1 | Rejected trial | 26.95 | 20.84214 | 27.99307 | 29.00389 | 43.56734 | 0.68/0.36 | 0.88/0.19 | 2.10 |
| 2 | Prior 1b13033 | 25.64 | 21.69411 | 27.61083 | 28.84979 | 29.00743 | 0.68/0.42 | 0.86/0.17 | 2.14 |
| 2 | Rejected trial | 26.84 | 21.59025 | 27.74393 | 30.18251 | 48.00831 | 0.69/0.40 | 0.84/0.18 | 2.11 |
| 3 | Prior 1b13033 | 25.04 | 20.57096 | 27.29297 | 67.90348 | 108.07506 | 0.73/0.40 | 0.91/0.19 | 2.23 |
| 3 | Rejected trial | 29.10 | 19.25647 | 26.91688 | 27.94591 | 28.71737 | 0.60/0.37 | 0.78/0.14 | 1.90 |

| Repetition | Binary | Daemon RSS peak/late MiB | Combined RSS peak/late MiB | Cgroup peak/late MiB | File late MiB | Kernel late MiB | Live allocator peak/late MiB |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 0 | Prior 1b13033 | 30.78/29.50 | 87.93/86.65 | 64.92/62.29 | 0.65 | 0.96 | 20.23/18.96 |
| 0 | Rejected trial | 33.07/30.90 | 90.12/88.04 | 66.20/62.82 | 0.90 | 0.98 | 19.83/19.00 |
| 1 | Prior 1b13033 | 30.95/30.31 | 88.11/87.44 | 64.83/62.54 | 0.68 | 0.97 | 19.55/18.99 |
| 1 | Rejected trial | 32.66/31.83 | 89.86/89.03 | 66.21/63.91 | 0.83 | 0.97 | 19.83/19.19 |
| 2 | Prior 1b13033 | 30.54/28.60 | 87.47/85.68 | 65.03/61.05 | 0.65 | 0.99 | 20.23/18.91 |
| 2 | Rejected trial | 33.11/31.77 | 90.24/88.88 | 67.00/63.97 | 0.65 | 0.98 | 19.77/19.09 |
| 3 | Prior 1b13033 | 29.56/27.84 | 86.52/84.93 | 62.97/59.66 | 0.66 | 0.96 | 19.78/18.90 |
| 3 | Rejected trial | 32.65/31.56 | 89.72/88.64 | 65.01/62.30 | 0.66 | 0.97 | 20.79/19.12 |

## Evidence and remaining work

Ignored `benchmark-results/private/vm-20261031-integrated/` retains the exact trial
sources, binary manifests, RED/fault receipts, both allocator coverage/native
variants, `v8-v9-writer-pair-v1/{report.json,derived-v1.json,memory-v1.json}`, the
separate calibrated request/CPU trace and pre-/post-restoration seals. Raw data,
not rounded table cells, determines each comparison. All 16 table rows are checked
against those reductions.

The retained source's [current results](performance-observation-age.md) and
[remaining performance/compatibility/quality gates](next-work.md) are unchanged.
The next optimization needs attributed request/setup or owner lifetime cost,
meaningful regression sensitivity and joint measurements; neither a narrow
mechanism test nor an isolated favorable repetition closes acceptance.
