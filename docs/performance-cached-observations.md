# Rejected cached-observation reuse experiment — 2026-11-01

Status: **REJECTED**. The trial removes a demonstrated redundant metadata
reconstruction, but does not establish a reproducible joint speed/memory win.
Runtime and tests are restored to observation-age source `1b13033`; its
[current mixed checkpoint](performance-observation-age.md) remains applicable.

## Mechanism and contract verification

Cached directory selection clones an existing inode's HEAD. After standard LIST
or type resolution, publication constructs another inode, decodes the same user
metadata again and replaces the original clean state. A public `directory_chunk`
regression first fails because unchanged encoded and decoded metadata allocations
lose their original owner, despite unchanged bytes, permissions and observation
time and no additional HEAD. This proves redundant reconstruction, not its share
of sustained CPU or process memory. Ordinary payload-cache hits bypass
`read::snapshot`; snapshot copies are not a per-hit diagnosis.

The trial distinguishes cached observations from new LIST metadata and reuses the
same live name binding at publication. The first strong-Arc version fails a
namespace-pressure regression: selection pins the eviction victim and lookup
returns ENOMEM. The corrected trial uses supported standard-library `Weak`
references plus the existing bounded fallback observation. Evicted nodes can be
reinstalled with a new inode number, without an additional HEAD or renewed TTL.
There is no TTL, negative-cache, payload-budget, validation or durability tuning.

Both experimental allocator suites pass 262 Rust cases, including ten explicit
native directory cases, and 75 benchmark Python cases. Matching release/default
and System-instrumented native client variants pass available rw/ro, rsync/nginx,
metadata, full-byte/Range/EOF, mmap and durability/failure contracts. Clippy,
formatting, public-source audit and duplicate checks pass. Three selected
type-resolution mutations are killed. Whole-file CRAP and broad mutation gates
remain open. The rejected trial's 262-case results do not replace the retained
source's 260-case verification.

## Source and control

- Prior correctness-equivalent runtime: `1b13033c3f161b9502c7a23870b645ca871ab2e3`,
  binary SHA256 `df24b6bdd42920b7989db2ed7c44855b9fc2bb1cf53292ee60ce023eb42138a8`.
- Rejected trial based on `6e5d5d3aabc01a0e5d3760c52e6367e3606f5fb9`, binary SHA256
  `c250f8f9aae89a1aed4a197c63fbbad1032ad29669e156b23e7da6edd3592243`.
  Only `src/filesystem.rs` differs among runtime inputs; all 85 build/test inputs
  are frozen with the trial. Neither experimental runtime nor tests are delivered.
- Same Linux VM, SeaweedFS 4.51, eight workers, 64 MiB payload budget, 100,000-entry
  limit, 256 MiB checked payload and 1,024 metadata entries. Equal unbounded CPU;
  filesystem/client, backend and owned parent memory limits are 16/12/32 GiB.
- Four rotated pairs, 1,000 metadata traversals per mount. All 8,000 phases,
  8,192,000 entries and eight final full-byte/generation checks pass. Owners and
  subprocess exits are clean; selected quota/reclaim/high/max/OOM deltas are zero.
  Builds, quality checks and profiles do not overlap the measured windows.

These are short metadata-only mounts, approximately 11 seconds of measured work
each. They do not establish sustained expiry behavior, other-workload performance
or a comparison against Go. Rates and pooled nearest-rank tails use cycles 1–999;
CPU and memory cover the whole measured interval, including its initial phase.
Sampled peaks and temporal latter-half means are not plateaus. Combined RSS is
pointwise daemon plus client; RSS, allocator and cgroup measurements have distinct
scopes and must not be added together. The host remains shared.

## All paired measurements

| Pair | Binary | Metadata ops/s | p50 ms | p95 ms | p99 ms | Maximum ms | Daemon user/system CPU s | Client user/system CPU s | Cgroup CPU s |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 0 | Prior 1b13033 | 142685.92 | 0.00346 | 0.00458 | 0.01775 | 2.03684 | 2.07/0.60 | 2.55/3.24 | 8.46 |
| 0 | Rejected trial | 139761.43 | 0.00347 | 0.00560 | 0.01821 | 3.91633 | 2.10/0.65 | 2.63/3.32 | 8.70 |
| 1 | Prior 1b13033 | 139172.24 | 0.00347 | 0.00569 | 0.01820 | 3.38000 | 2.05/0.65 | 2.58/3.37 | 8.65 |
| 1 | Rejected trial | 144538.75 | 0.00345 | 0.00445 | 0.01767 | 1.95915 | 2.03/0.61 | 2.55/3.23 | 8.41 |
| 2 | Prior 1b13033 | 141811.38 | 0.00346 | 0.00485 | 0.01774 | 1.90814 | 2.00/0.65 | 2.49/3.31 | 8.45 |
| 2 | Rejected trial | 143491.90 | 0.00346 | 0.00445 | 0.01775 | 1.63004 | 1.99/0.62 | 2.53/3.25 | 8.40 |
| 3 | Prior 1b13033 | 140662.44 | 0.00347 | 0.00447 | 0.01817 | 3.15097 | 2.13/0.63 | 2.52/3.29 | 8.57 |
| 3 | Rejected trial | 139239.93 | 0.00349 | 0.00482 | 0.01747 | 2.23201 | 2.16/0.66 | 2.56/3.30 | 8.68 |

| Pair | Binary | Daemon RSS peak/late MiB | Combined RSS peak/late MiB | Cgroup peak/late MiB | File late MiB | Kernel late MiB | Allocator peak/late MiB |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 0 | Prior 1b13033 | 31.79/30.36 | 86.34/84.92 | 63.61/62.26 | 0.23 | 2.29 | 24.41/23.28 |
| 0 | Rejected trial | 30.50/29.75 | 85.11/84.36 | 62.64/58.79 | 0.23 | 2.31 | 24.02/23.63 |
| 1 | Prior 1b13033 | 30.41/29.36 | 84.95/83.90 | 63.82/62.67 | 0.21 | 2.27 | 23.69/22.50 |
| 1 | Rejected trial | 30.32/29.29 | 84.88/83.85 | 63.24/61.86 | 0.22 | 2.27 | 23.14/22.68 |
| 2 | Prior 1b13033 | 29.86/28.86 | 84.43/83.43 | 61.79/60.62 | 0.22 | 2.24 | 23.32/22.31 |
| 2 | Rejected trial | 31.84/30.82 | 86.40/85.39 | 63.53/62.43 | 0.22 | 2.30 | 23.80/23.49 |
| 3 | Prior 1b13033 | 30.52/29.24 | 85.03/83.75 | 63.12/61.75 | 0.22 | 2.26 | 23.32/22.54 |
| 3 | Rejected trial | 31.07/29.40 | 85.64/83.98 | 62.10/60.53 | 0.23 | 2.27 | 23.36/22.62 |

Throughput and pooled p99 each lose in two pairs; daemon/cgroup CPU also lose
twice. Late live allocator memory increases in all four pairs, while RSS is not a
reproducible win. These measurements do not establish why the allocator grows or
whether the mechanism could help a different, properly measured expiry workload.
The trial is discarded for lack of joint evidence, not as proof that removing
metadata copies can never help. Removing a page-rebuild allocation is insufficient
evidence of faster sustained cached-stat service or fewer SDK requests.

## Evidence and remaining work

Ignored evidence under `benchmark-results/private/vm-20261031-integrated/` retains
`candidate-v10`, both `quality-cached-observation-*-v1` source/coverage bundles,
three matching native variants, meaningful owner/pinning RED logs, three killed
mutations, `v8-v10-metadata-pair-v1` raw/reduced/verified results and
`cached-observation-restored-sealed-v1/report.json`. The initial exact-name test
filter selected zero cases and the first mutation scan selected zero sites; both
are preserved as setup limitations, not regression or mutation coverage.
Every retained runtime/test input is checked against the original 85-input v8
manifest after restoration. Detailed commands and reasons are in `actions.md`.

The next attribution must isolate actual expiry/setup work and its retained
owners before another optimization: the earlier calibrated expiry trace contains
1,002 HEADs in one metadata phase. A short cached traversal cannot measure that
request burst. Do not return to extending metadata freshness, removing create or
post-commit verification, caching default misses, or tuning budgets/allocators.
Current Go-relative metadata/writer/process-memory losses, whole-file quality,
positive Linux LINK and actual-service compatibility remain open.
