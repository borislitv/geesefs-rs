# Loaded read shape and bounded native control — 2026-11-02

**UNACCEPTED.** These are diagnostic measurements of the unchanged `4a06749`
runtime, not a performance optimization, a memory plateau or production acceptance.
Detailed commands, inputs, object names, operational traces and receipts remain
private. The [earlier target preflight](performance-target-preflight.md) retains
its actual-kernel client contracts and initial idle-reader scope.

**Reference update:** Go 0.43.9 is now the primary acceptance reference. The
unmerged Go optimization experiment is historical evidence only. This page
preserves the original three-implementation measurements; the subsequent
[listing diagnosis and configuration control](performance-listing-diagnosis.md)
explain the default-Rust loss and its freshness tradeoff.

## Loaded-neighbor evidence and transformation

A bounded live capture now includes nginx access/error logs and routing, rsync
module configuration/journals, golist container logs/backend mapping and the actual
GeeseFS reader's service journal. The mount unit itself had no reader samples;
process/cgroup attribution identifies the correct journal. Reader identity,
configuration and mounts are unchanged across collection. No logging is enabled
and no served service is restarted.

A completely covered 240-second interior window has 88,506 HTTP requests. Only
2,712 payload requests and eight listing requests map to the common filesystem;
2,709 payload requests enter the eligible method/status cohort. Direct-backend,
origin/cache, other-vhost and other-tree requests remain separate. Eight GeeseFS
samples show nonzero reads. The matching rsync journals identify sessions/modules,
not complete per-file reads; golist does not log every successful request.

HTTP bytes include headers, and logs omit exact Range offsets and object
generations. Accordingly, the replay uses 44 generated objects totaling
67,248,128 bytes, 256 transformed HTTP operations in four lanes, eight generated
listing directories and an eight-file rsync subset. Size classes, offsets,
contents, directory shapes and rsync lists are synthetic, with independent byte
oracles. It is downscaled and unpaced, not an exact production replay.

## Completed isolated comparison

Eight rotated blocks have five independent mounts each: Rust, the historical Go
experiment, Go 0.43.9 and an identical-executable control for each Go arm. Every mount
has one cold and five warm cycles. HTTP, golist and rsync phases run sequentially;
the HTTP phase has four concurrent lanes. This is not simultaneous production
traffic from all three clients.

The filesystem/client group has eight CPUs and 16 GiB; the disposable SeaweedFS
S3 backend has 16 CPUs and 12 GiB under a 24-CPU/28-GiB parent, with
`GOMAXPROCS=8`. All arms use buffered read-only mounts, a 64 MiB application
budget, 100,000 entries, a 60-second stat TTL and eight runtime workers. Go
GC/soft-memory policy is not forced. Private mount/network namespaces isolate
the loopback backend. The clients are nginx 1.28.3, the privately verified target
golist executable using its local backend, and rsync.
The nginx fixture disables open-file caching; it is not the complete served config.

The current Rust release SHA256 is
`8cd8d48f8a1546d54cf46e6e52252eaca10e6b4dac6317e37a65a5542a388c62`.
The historical Go experiment is `5c0884c9e8799bbab93e2d4102fdc15e9c079e0d`;
Go 0.43.9 is `80c214ef14f620d93407454fc76ca01dd71fa25f`. Both are rebuilt with
Go 1.25.9. The former's [PR239](https://github.com/yandex-cloud/geesefs/pull/239)
is closed without merge; it is excluded from acceptance comparisons.

All 40 mounts complete: 61,440 HTTP checks, 3,840 rsync file checks and 1,920
complete listing checks. Full/partial HTTP bytes and rsync full/delta results are
independently checked; each arm preserves all backend generations and bytes.
Workers/daemons exit normally without high/max/OOM memory-event growth. A changed
backend byte fails the checksum oracle. Original conditional-fixture setup errors
remain recorded; the final generated fixture uses one independently read backend
LastModified for all arms, not a claim of production metadata parity.

### Warm endpoint means

These are means of eight mount endpoints. Throughput is verified HTTP body bytes
divided by HTTP phase time; latency endpoints average per-cycle quantiles within
a mount, not pooled population percentiles. Listing/rsync values are phase means.

| Endpoint | Rust | Historical Go experiment | Go 0.43.9 |
| --- | ---: | ---: | ---: |
| HTTP MiB/s | 472.43 | 642.10 | 245.17 |
| HTTP p50 ms | 3.437 | 0.818 | 8.293 |
| HTTP p95 ms | 12.401 | 9.775 | 24.420 |
| HTTP p99 ms | 26.877 | 12.861 | 37.530 |
| Generated listing ms | 18.027 | 3.788 | 3.773 |
| Rsync phase ms | 98.546 | 110.083 | 112.666 |
| Five warm cycles, total seconds | 4.687 | 3.439 | 6.961 |

### Memory and CPU scopes

Peaks are sampled phase maxima including the cold cycle, averaged across mounts;
late values average the latter halves of the five warm cycles. They are neither
kernel lifetime peaks nor evidence of a stable bound. Summed RSS is not PSS, and
cgroup charges are a different scope from summed process residency. Go heap was
not collected. Rust live allocation mean peak/late is 75.81/74.92 MiB over its
complete mount intervals.

CPU seconds cover the same five-cycle mix: 1,280 HTTP operations, 40 listing
checks and 80 rsync file checks. The Python client row excludes its rsync children;
whole-cgroup CPU includes those children and the serving processes. Normalized
cost below includes all phases of that fixed mix, not only HTTP processing.

| Endpoint | Rust | Historical Go experiment | Go 0.43.9 |
| --- | ---: | ---: | ---: |
| Daemon RSS peak / late MiB | 87.94 / 87.55 | 112.65 / 110.46 | 119.66 / 118.94 |
| All-process RSS peak / late MiB | 162.76 / 147.99 | 188.28 / 174.12 | 194.47 / 179.50 |
| Cgroup peak / late MiB | 164.15 / 131.52 | 185.56 / 171.70 | 193.12 / 178.24 |
| Cgroup file / kernel late MiB | 17.57 / 2.37 | 60.63 / 2.61 | 59.31 / 2.57 |
| Daemon user / system CPU s | 0.670 / 1.450 | 0.368 / 1.095 | 1.658 / 1.476 |
| Python client user / system CPU s | 1.710 / 0.781 | 1.606 / 0.704 | 1.875 / 0.870 |
| Nginx user / system CPU s | 0.032 / 1.454 | 0.029 / 1.581 | 0.031 / 1.774 |
| Golist user / system CPU s | 0.028 / 0.061 | 0.021 / 0.051 | 0.022 / 0.050 |
| Cgroup user / system CPU s | 2.548 / 3.994 | 2.155 / 3.683 | 3.672 / 4.466 |
| Daemon CPU ms per HTTP op of fixed mix | 1.656 | 1.143 | 2.448 |
| Cgroup CPU ms per HTTP op of fixed mix | 5.110 | 4.561 | 6.358 |
| Cgroup throttled seconds | 0 | 0 | 0 |

### Whole-mount screening

Cost ratios below one favor Rust; HTTP throughput is inverted into cost. The
[paired protocol](performance-methodology.md) uses eight real controls per
reference, mount bootstrap confidence intervals and no invented practical margin.

| Cost endpoint | Versus historical experiment: ratio [95% CI], screen | Versus Go 0.43.9: ratio [95% CI], screen |
| --- | --- | --- |
| HTTP throughput cost | 1.366 [1.244, 1.504], regressed | 0.521 [0.495, 0.553], improved |
| HTTP p50 | 4.181 [3.806, 4.608], regressed | 0.412 [0.392, 0.434], improved |
| HTTP p99 | 1.693 [1.239, 2.600], regressed | 0.580 [0.436, 0.902], improved |
| Daemon late RSS | 0.793 [0.787, 0.798], improved | 0.736 [0.733, 0.740], improved |
| Cgroup late memory | 0.766 [0.760, 0.771], improved | 0.738 [0.733, 0.742], biased controls |
| Daemon CPU | 1.448 [1.362, 1.538], regressed | 0.676 [0.657, 0.696], improved |
| Cgroup CPU | 1.118 [1.061, 1.180], inconclusive | 0.803 [0.788, 0.820], improved |
| Generated listing latency | 4.774 [4.566, 5.006], regressed | 4.784 [4.579, 5.031], regressed |
| Rsync phase latency | 0.894 [0.857, 0.938], improved | 0.874 [0.837, 0.910], improved |

The kernel-memory comparison against experimental Go also has biased controls.
The experimental-Go losses remain historical, not current acceptance targets.
The default-Rust listing loss to Go 0.43.9 remains measured; the subsequent
diagnosis explains it without silently changing policy. Short windows, synthetic directory/rsync shapes
and the workload transformation prevent sustained or production acceptance.
Sampler/allocator observer overhead is not separately isolated in this fixture.

## Bounded native-S3 LXC attempt

Three authorized idle LXCs run the same frozen binaries on
`5.15.179-15.2mds`, installed nginx 1.24.0 and rsync 3.2.7. Each private-mount unit
has eight CPUs and 16 GiB. Five-arm blocks stay on one host, rotating blocks across
the three hosts. Four frozen small archives total 951,985 bytes; each cycle has
64 HTTP payload operations and full/delta rsync checks of the four files.

A shared anonymous read-only transport limits all hosts to four upstream starts
per second, two in flight, 128 MiB of application response bodies and 1,536
requests. It forwards no credentials, performs no retries or redirects, and
permits only the frozen payloads and bounded prefix metadata. Application-body pacing is capped at
2 MiB/s; these are not wire-byte bounds. Earlier-attempt consumption carries
forward. Twelve local-wire tests pass; ten relevant injected guard faults and
two removed byte-oracle assertions are rejected. Cold time is limiter-paced and
is not unconstrained native throughput.

The first smoke exposes a guard setup error: the SDK's normal
`x-id=GetObject` query was rejected locally. A failing local-wire regression
precedes its allowlist correction. Experimental Go also returns 200 instead of
the expected mtime-based 304. The subsequent comparable payload cohort explicitly
tests GET/HEAD/Range, with metadata recorded separately rather than weakening the
mtime expectation or changing the filesystem.

The corrected attempt stops on one 15-second upstream HEAD timeout in the
Go 0.43.9 control arm, not a Rust crash or byte-check failure. No upstream
429/5xx is observed; this is not proof of storage overload. Both attempts together
use 507 requests and 37,432,447 response bytes, with peak concurrency two and
observed start gaps at least 0.250 s. No new upstream work is started after the
failure is recognized, and no retry attempt follows it.

Of 30 attempted paired mounts, 29 finish correctly; three preceding smoke mounts
also pass. Only five complete five-arm blocks remain: all their metric screens
are **insufficient pairs**, even when a descriptive interval looks favorable.
The full eight-block design and final generation HEADs are incomplete. All
successful mounts contribute 11,520 HTTP and 1,440 rsync file checks; measured warm
cycles of the five complete blocks start zero upstream requests of their own.

| Five-block descriptive warm mean | Rust | Historical Go experiment | Go 0.43.9 |
| --- | ---: | ---: | ---: |
| HTTP MiB/s | 210.38 | 204.15 | 204.98 |
| HTTP p50 ms | 1.658 | 1.677 | 1.712 |
| HTTP p95 ms | 5.275 | 5.342 | 5.252 |
| HTTP p99 ms | 7.828 | 8.051 | 7.577 |
| Rsync phase ms | 50.999 | 51.554 | 51.243 |
| Five warm cycles, total seconds | 0.733 | 0.745 | 0.741 |
| Daemon RSS peak / late MiB | 20.20 / 20.12 | 40.74 / 40.55 | 41.66 / 41.45 |
| All-process RSS peak / late MiB | 68.38 / 56.77 | 89.18 / 77.65 | 89.87 / 79.29 |
| Cgroup peak / late MiB | 32.87 / 30.45 | 38.30 / 35.61 | 38.36 / 36.09 |
| Cgroup file / kernel late MiB | 1.99 / 1.13 | 2.95 / 1.22 | 2.75 / 1.24 |
| Daemon user / system CPU s | 0.022 / 0.062 | 0.052 / 0.056 | 0.054 / 0.058 |
| Python client user / system CPU s | 0.384 / 0.076 | 0.402 / 0.062 | 0.398 / 0.064 |
| Nginx user / system CPU s | 0.010 / 0.054 | 0.010 / 0.054 | 0.012 / 0.050 |
| Cgroup user / system CPU s | 0.470 / 0.231 | 0.510 / 0.215 | 0.514 / 0.212 |
| Daemon CPU ms per HTTP op of fixed mix | 0.262 | 0.338 | 0.350 |
| Cgroup CPU ms per HTTP op of fixed mix | 2.193 | 2.268 | 2.269 |
| Cgroup throttled seconds | 0 | 0 | 0 |

Native peaks/late samples use the same cold-inclusive/warm-latter-half scopes as
above. CPU covers 320 HTTP operations and 40 rsync file checks per mount; the
Python process excludes rsync-child CPU, which is included in the cgroup. CPU
ticks are coarse relative to these short windows. Native allocator/Go-heap
endpoints have not been reconciled, and this cohort has no listing/golist phase.

These tiny resident objects do not establish a memory plateau or general speed
superiority. Shared transport/SSH memory is outside the remote groups; cgroup
file-page ownership can also differ from process RSS. Repeated four-object mtime
snapshots match the independently read stored metadata for Rust 28/28 times,
experimental Go 0/52 including its control, and packaged Go 37/48 including its
control. Metadata parity remains separate from byte correctness and payload speed.

All three hosts preserve served-reader PID/hash/argv/runtime, mount identities and
fstab. Owned workers, private files and forwarding are removed after the stopped
attempt. The private diagnostic runner does not close the public generic native
matrix's integration gap or the wider compatibility/quality gates.
