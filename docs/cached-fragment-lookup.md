# Cached-fragment lookup continuation

## Mechanism and preserved contract

The [body-prefix matched run](streamed-read-body.md) improves p99 against its
exact Rust prior while regressing throughput, CPU and final-fifth RSS. Separate
validated profiles and code inspection identify a concrete cache bookkeeping
cost: a request spanning small fragments first searches older predecessors for a
single block covering the whole reply, then repeats coverage lookup to compose it.

`ReadCache::read` now locates the starting byte once. If its immutable block covers
the full requested range, return the same charged view. Otherwise join subsequent
same-inode/generation fragments into the already admitted output. A gap still
returns a miss; byte bounds and complete output remain mandatory. This removes
the futile whole-span predecessor scan without replacing Moka, changing the cache
budget or changing conditional GET publication/checksum semantics.

Admission and pressure reclamation remain before borrowing cached allocations.
Queued fragments and views retain complete GET admission and physical allocation
ownership. Writer materialization and generation changes retain their existing
paths. No allocator tuning, extra GET, detached producer or dependency is added.

## Verification

The unchanged-source baseline and changed source pass isolated Kubernetes
verification: 178 Rust cases with no failures/ignored cases, 14 source/oracle and
23 benchmark Python cases, formatting, warning-denying all-target/all-feature
Clippy and the 336-package public-source audit. The restored 56-case affected
suite preserves real HTTP cached-span/generation checks, pinned-view ownership,
FIFO-pressure progress, concurrent windows and cancellation.

Seven independently inserted faults still fail intended behavior assertions:
deferred publication, missing full-GET charge, lost cached-span coverage, premature
checksum publication, ignored short reads, hidden late failures and lost offsets.
One selected cache completion equality mutant is killed. The tool scans 27 sites;
the other 26 were not executed. Package clean is explicit during mutations and
before final full verification to avoid stale-mtime artifacts.

The [quality receipt](cached-fragment-lookup-quality-20261027.json) binds actual
source/fault/tool hashes and check counts. Cached composition remains CC 6;
filesystem read-inner CC 49, three prior duplication candidates and unmeasured
coverage/CRAP remain open. No private-method call-count or artificial timing test
is presented as a functional regression: the retained matched loss is the
performance red; the matched repeat below checks this candidate.

## Matched native comparison

Frozen source `3b94a76abc422f6e944c9e596fa971cf19b03858` completed forty phases
in 762.040 seconds against exact body-prefix `20a9dcd` and both unchanged Go
references. Only `src/cache.rs` differs between the 25 compiled input hashes.
All 120,000 full-byte/metadata checks pass; native conditional verification of
3,000 objects and all 97 exact listings confirms unchanged generations. After
collection, all 460 indexed artifacts, 280 CPU windows, memory summaries and
both compiled-input commits were independently verified.

Same 8,298,727,528 synthetic bytes, 2 CPU/4 GiB filesystem quota, separate
4 CPU/6 GiB backend, 512 MiB payload budget, 100,000 entries and four 128 KiB
readers. The default allocator and complete xattr/forced-stat work are unchanged.
Fresh is a new process/mount; warm revisits the complete dataset in that mount.
Host/backend caches are not controlled or globally dropped.

Five-phase means, decimal MB; daemon RSS is peak / mean final fifth, **not a
stationary steady state**. CPU covers the whole filesystem cgroup, including
clients/controller. The [numeric receipt](cached-fragment-lookup-results-20261027.json)
also preserves p50/p95, dispersion, individual CPU roles, allocator, client,
cgroup/file/kernel and separate backend memory/CPU.

| Implementation/state | Read MB/s | Daemon RSS peak / steady MB | Whole cgroup CPU s | Read p99 ms | Listing s |
| --- | ---: | ---: | ---: | ---: | ---: |
| Go experimental fresh | 550.378 | 1312.101 / 1223.334 | 32.945 | 12.995 | 0.515 |
| Go experimental warm | 546.495 | 1260.276 / 1220.825 | 32.214 | 12.978 | 0.034 |
| Go packaged fresh | 533.273 | 1303.464 / 1211.321 | 33.932 | 13.494 | 0.547 |
| Go packaged warm | 537.664 | 1299.390 / 1185.226 | 32.529 | 13.074 | 0.031 |
| Rust cache lookup fresh | 511.121 | 561.243 / 437.330 | 34.990 | 14.231 | 1.530 |
| Rust cache lookup warm | 497.282 | 566.109 / 463.358 | 34.613 | 14.337 | 0.023 |
| Rust body prior fresh | 473.424 | 558.902 / 431.998 | 37.727 | 16.315 | 1.618 |
| Rust body prior warm | 471.799 | 562.294 / 451.894 | 36.303 | 16.029 | 0.021 |

Against matched prior Rust, mean throughput rises 7.96% fresh / 5.40% warm,
whole CPU falls 7.25% / 4.66%, and p99 falls 12.77% / 10.56%. All five fresh
repetitions favor throughput/CPU/p99, but warm throughput and CPU favor the
candidate in only three of five; warm p99 favors four. No statistical significance
or uniform warm win is claimed. P50 and p95 means also improve versus the prior.

Adverse results remain visible: peak RSS increases about 0.42% / 0.68%, and
final-fifth RSS increases about 1.23% / 2.54%. Warm metadata roundtrip/listing
means also increase. Both Go references still win throughput, whole CPU, p95/p99
and cold listing. Rust's lower process RSS and p50 do not establish a whole-memory
or all-metrics win: file cache fills most of the cgroup allowance. Go heap series,
backend request metrics, representative writer/random/HTTP workloads and broader
compatibility remain open. Prior adverse comparisons are retained; overall
compatibility and comparative performance remain **UNACCEPTED**.

## Separate profile follow-up

After all comparison timing/export/collection, three owned-PID perf diagnostics
sample exact Go experimental, body-prefix prior and cache-lookup executables at
149 Hz. Each completes all 3,000 file checks; fresh native conditional verification
then passes for all objects and 97 listings. Nonempty readable profiles, no lost
samples, exact executable/sample/report hashes and requested SIGINT stops are
verified in the [profile receipt](cached-fragment-lookup-profiles-20261027.json).
These instrumented runs are not extra performance-comparison repetitions.

The selected cache leaf shares fall from 1.78% to 0.84% for BTree range search,
1.10% to 0.36% for covering, 0.93% to 0.56% for policy maintenance and 0.55% to
0.28% for recency updates. Bucket lookup remains visible at 1.27% / 1.25%; it is
not eliminated. Missing entries below the 0.1% report threshold are never zero.
Kernel copying and page clearing remain visible: page-clear share is 2.85% in
the candidate versus 0.85% in Go. These single sampled shares neither quantify
absolute CPU differences nor establish an allocator/SDK/network ceiling.

The narrower cache hypothesis has a measured benefit, while the remaining actors
include allocation/retention, per-fragment publication/maintenance and metadata
resolution. Isolate their costs against this frozen executable before selecting
another source change; retain complete generation/charge/checksum/EOF contracts.
