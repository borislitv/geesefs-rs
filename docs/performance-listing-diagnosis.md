# Listing diagnosis and reader readiness — 2026-11-02

**UNACCEPTED.** Go 0.43.9 is the primary comparison. The earlier unmerged Go
optimization experiment is excluded from acceptance; its original measurements
remain in the [historical read-shape checkpoint](performance-read-shape.md).
This diagnosis uses the unchanged `4a06749` runtime and frozen Rust executable
`8cd8d48f8a1546d54cf46e6e52252eaca10e6b4dac6317e37a65a5542a388c62`.
No runtime default or served mount changes.

## Why the default listing loses

The caller checks an absent index file and two synchronization markers at the
leaf and repository parent: five absent names per listing. With the default
`--negative-lookup-cache-size=0`, each Rust miss resolves the object, directory
marker and implicit-directory prefix: two HEADs and one LIST. Go 0.43.9 can
answer absence from a still-fresh complete directory listing.

Independent syscall/FUSE/request traces verify the mechanism. Every warm window
of eight listings contains 40 LOOKUPs, 200 OPENDIRs, 200 RELEASEDIRs and 24
READDIRPLUS operations in both implementations. Default Rust makes **80 HEADs
and 40 LISTs**; Go makes zero S3 requests. A separate trace of the existing
explicit Rust negative cache also makes zero, with the same FUSE counts.
READDIR time is small relative to default miss time. Instrumented elapsed times
are attribution evidence, not benchmark results.

## Eight-block configuration control

The uninstrumented design has eight rotated/reversed blocks, each with five
independent mounts: default Rust, Rust with `--negative-lookup-cache-size 64`,
an identical Rust64 A/A control, Go 0.43.9 and its identical A/A control.
All Rust arms use the same executable; only the explicit cache option differs.
The Go release source is `80c214ef14f620d93407454fc76ca01dd71fa25f`, rebuilt with
Go 1.25.9; this is not the installed reader executable or its configuration.

The [same transformed workload and resource placement](performance-read-shape.md#completed-isolated-comparison)
are retained: SeaweedFS, one cold/five warm cycles, 64 MiB payload budget,
100,000 entries, 60-second stat TTL, eight runtime workers, eight CPU/16 GiB
filesystem/client group and 16 CPU/12 GiB backend under a 24 CPU/28 GiB parent.
All 40 mounts finish: **61,440 HTTP, 3,840 rsync-file and 1,920 listing checks**,
independent final backend byte/generation checks, normal worker/daemon exit and
unchanged high/max/OOM counters. Owned tasks, cgroups and mounts are removed.

Values are means of eight mount endpoints. Latencies average cycle endpoints,
not pooled percentiles. Sampled peaks include cold phases; late gauges use the
latter halves of warm phases. CPU covers the same fixed mix of 1,280 HTTP,
40 listing and 80 rsync-file checks per mount, not HTTP alone.

| Endpoint | Default Rust | Explicit Rust64 | Go 0.43.9 |
| --- | ---: | ---: | ---: |
| Warm listing ms | 18.493 | 3.095 | 4.190 |
| HTTP MiB/s | 457.28 | 777.29 | 223.88 |
| HTTP p50 / p95 / p99 ms | 3.594 / 12.061 / 31.626 | 0.810 / 8.006 / 22.240 | 8.620 / 27.625 / 55.298 |
| Rsync phase ms | 106.561 | 103.511 | 116.682 |
| Daemon RSS peak / late MiB | 87.98 / 87.46 | 87.48 / 87.20 | 119.67 / 118.66 |
| All-process RSS peak / late MiB | 161.80 / 146.18 | 160.90 / 147.75 | 193.10 / 177.20 |
| Cgroup peak / late MiB | 161.96 / 129.97 | 160.25 / 130.54 | 190.84 / 175.56 |
| Cgroup file / kernel late MiB | 17.24 / 2.38 | 17.14 / 2.46 | 59.40 / 2.58 |
| Daemon user / system CPU s | 0.676 / 1.510 | 0.228 / 1.294 | 1.720 / 1.561 |
| Cgroup user / system CPU s | 2.544 / 4.178 | 2.007 / 3.791 | 3.769 / 4.776 |

[Whole-mount bootstrap screening](performance-methodology.md), cost below one
favors the numerator:

| Listing comparison | Cost ratio [95% CI] | A/A CI | Screen |
| --- | --- | --- | --- |
| Default Rust / Go 0.43.9 | 4.397 [4.100, 4.725] | [0.931, 1.038] | Regressed |
| Explicit Rust64 / Go 0.43.9 | 0.738 [0.705, 0.773] | [0.931, 1.038] | Improved |
| Explicit Rust64 / default Rust | 0.168 [0.161, 0.176] | [0.920, 1.008] | Improved |

Rust64 A/A controls the changed configuration, not default-zero variance.
No practical non-inferiority margin is invented. In this new run the Go daemon
RSS A/A interval excludes one: the lower raw Rust RSS is **not a new controlled
memory-superiority verdict**. Rust64/default p99 is within noise, not proven
non-inferior. Summed RSS is not PSS; Go heap is missing, not zero. Short windows,
small namespaces and private orchestration do not establish a memory plateau,
the public generic matrix or whole-target performance acceptance.

## TTL parity and accepted negative-cache behavior

The operator accepts Go 0.43.9's cache behavior as the compatibility target.
Use `--negative-lookup-cache-size=64 --stat-cache-ttl=1m` for Rust's read-only
Go-parity comparisons. Both CLI metadata TTL defaults are one minute, and every
arm in the main comparison uses that TTL. The number 64 bounds misses per
directory; it is not seconds, and Rust currently has no separate negative TTL.
Go answers absence from a fresh directory listing; Rust timestamps individually
verified misses. The implementation and expiry anchors differ, so equal TTLs do
not mean identical cache internals.

A separate real-FUSE read-only probe first lists a directory and observes the
same three absent names, then creates them through the disposable S3 backend.
With a five-second natural TTL, default Rust sees all three immediately;
Rust64 and Go 0.43.9 still return ENOENT. After expiry, all three implementations
return the exact independently generated bytes. Removing the Rust64 option
causes the visibility oracle to fail; restoring it passes again.

Five seconds belongs only to this separate expiry probe, not the main benchmark
or a new runtime default. The shared bounded external-create delay is accepted
Go-parity behavior, not a new correctness blocker for negative caching. More
granular TTL controls can be added later. **The cache-size default remains zero**;
the measured comparison enables 64 explicitly and changes no runtime source.

## Loaded reader memory: established scope and remaining blocker

The existing Go 0.43.9 reader is built with Go 1.25.14 and `CGO_ENABLED=0`, and
runs with a 32 GiB payload-cache limit, 20 million entries and
`GOMEMLIMIT=128GiB`. Read-only observations preserve its executable, arguments,
runtime settings and mounts. Across two process/cgroup snapshots:

| Memory scope | Before | After |
| --- | ---: | ---: |
| Process anonymous PSS, GiB | 216.96 | 217.17 |
| Reader cgroup file memory, MiB | 1.44 | 1.44 |
| Reader cgroup kernel memory, MiB | 474.84 | 475.27 |

The large footprint is anonymous process memory, not kernel file cache.
Thirty-five recent journal samples report 14,404,220–14,406,059 live metadata
entries and zero sampled evictions. The release's `MetaEvictor` skips eviction
below its entry-count threshold; stat TTL alone does not remove those entries.
The data-cache limit does not include all metadata/runtime memory, and
[Go's memory limit is soft](https://go.dev/doc/gc-guide#Memory_limit).
Those policies explain why the configured numbers are not hard RSS bounds;
they do **not** assign all 217 GiB to metadata or prove a leak.

The visible container root has a 420 GiB cap and 263.14–265.15 GiB charged during
capture; the reader leaf's `max` is not unlimited host memory. Leaf pressure/OOM
events do not grow. These snapshots are not a current side-mount admission test.
No reader-owned listener exists at the configured profiling port, including in
its network namespace. The bounded startup-error journal search hits its deadline.
Live heap versus retained arenas/payload/metadata remains **unattributed** until
a functioning reader-owned heap/MemStats endpoint is available; no forced GC or
reader restart is used to manufacture that evidence.

## What still prevents a serving-equivalent side mount

The Go serving reader already uses `allow_other`. The gap below is in the
measured Rust rewrite, not a fault in the working Go mount or an architectural
restriction on serving. An isolated Rust mount for its owner already works; that
does not establish access for nginx/rsync service users. The frozen Rust binary
rejects `allow_other`, `dev`
and `suid`. A disposable supported mount reads correct bytes as owner but gives
UID 65534 `EACCES` for a `0644` file under a `0755` root. An inherited mount-root
descriptor excludes private ancestor permissions. The adapter leaves fuser's
ACL at `Owner`; fuser 0.18 already exposes `SessionACL::All`, `Dev` and `Suid`.
This is an application integration gap, not an upstream API deficiency. Go's
`--pprof` flag is also unsupported; it is a telemetry/CLI difference.

Rust's namespace limit counts nodes rather than bytes. At capacity it scans for
an unreferenced, unopened, clean inode and returns `ENOMEM` if none is evictable;
Go can evict expired kernel-referenced entries. Millions of pinned names and
full-key/metadata retention therefore need a realistic working-set contract,
not an assumption that the payload budget bounds process memory. Exact target
profile/signing/extended metadata, non-owner clients, TTL/error behavior and
long-lived memory must be checked before serving replacement. The
[actual-LXC subset](performance-target-preflight.md) and broader writer/CLI gaps
retain their separate scope.
