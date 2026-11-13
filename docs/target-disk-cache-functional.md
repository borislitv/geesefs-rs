# Target HDD functional checkpoint — 2026-11-11

**Not accepted for production.** Persistent-cache recovery and the explicit byte,
metadata and rsync controls pass, but the hourly serving replay contains truncated
responses. This is one operator-authorized, withdrawn-reader trial, not a paired
performance comparison or a complete filesystem-compatibility result.

## Frozen scope

- Application `91bdc2071e06b4f77bf6e94ac334c82621aacc87`, maintained Foyer
  `d60dd0462c1de1e0853590791371208b659c90bf`.
- Local Rust 1.99.1 default-jemalloc release binary SHA256
  `7a6f845ab6cddd92240bce3cb5968f63fd75abfb9751e1e16549dde3739d5572`.
- Linux `7.0.0-14-generic`, nginx 1.24.0, rotational HDD/ext4. The cache shares
  storage with existing service data; other host activity is not isolated.
- Unchanged 48-GiB total payload budget, 30-million-entry cap, disabled metadata
  byte limit, one-minute stat TTL and 64 negative entries per directory. Reader
  cgroup:
  144-GiB memory high, 160-GiB maximum, no swap, 32-CPU quota.
- Private 2-TiB cache file, regular-expression selection of `.deb`/`.rpm` files.
  Funding deducts 3,269,881,856 bytes from the existing payload budget. The
  8,388,608-record index cap still limits useful capacity independently of file
  allocation; this run does not populate or recycle a full 2-TiB inventory.
- Single trusted user, ordinary downloaded-copy semantics, and no extra GET solely
  for KMS reauthorization, as recorded in the [contract](disk-cache-design.md).
  Occasional disk-cache slowdowns are accepted for this functional trial.

## Workload and retained setup failures

A frozen hour from an independent reader contains 957,575 parsed records; 50
invalid request lines and one unusable-host line remain accounted separately.
The completed read-only replay examines every parsed record and finishes all
954,924 submitted requests in 3,607.391 seconds, receiving 1,087,606,805,068 body
bytes. It excludes 124 non-read methods, 2,166 non-origin targets and 361 admission
probes. Concurrency is 64, with the original logged-second pacing and bounded
backpressure; aggregate limits remain three hours and 3 TiB.

Original Range offsets, validators and request headers are unavailable. GETs
consume at most the recorded `bytes_sent`, then intentionally close a larger
body; that field includes headers, so this is an approximate body budget, not an
exact wire replay. Original 206 responses use a prefix Range and original 304s
use `If-None-Match: *`. The 28,486 intentional body cuts are not full-body passes.

Earlier attempts remain preserved: changing a live nginx shared-zone key rejected
reload despite a successful command exit; fail2ban blocked the single generator;
and a full-body replay incorrectly completed historically aborted large downloads.
Scoped client-only exceptions and new temporary zone names corrected the first
two causes. A peer-byte-checked prefix control validates the revised body budget.
The same binary, cache budgets and deadlines were retained throughout.

## Explicit functional results

- Independent peer rsync verifies 12 immutable files / 351,587,172 bytes, including
  SHA256, size, mode and mtime, both before and after restart. A repeated checksum
  sync transfers no files. Corruption of only a local downloaded copy is detected
  and repaired by delta transfer.
- Each of those two phases passes 61 HTTP checks: byte ranges, EOF 416, conditional
  304 and certificate-verified TLS. These checks do not validate every replay body.
- Separate direct-filesystem pressure completes 808 files / 137,435,934,585 bytes
  (127.997 GiB) in 2,047.956 seconds without transfer errors. Eight HEAD 404s and
  ten files outside the remaining size budget are retained. Four clients use an
  aggregate 64-MiB/s pacing limit without shrinking RAM caches.
- The 2-TiB file is allocated, and the first startup completes in 89.900 seconds,
  including 10.036 seconds allocation and 79.864 seconds recovery work.
- One ordinary unmount/restart preserves the same file and recovers 87,981 index
  records in 80.431 seconds, within the unchanged 120-second application limit.
  The mount appears after 2.10 seconds while recovery continues asynchronously.
  An initial harness assertion incorrectly expected immediate Ready; its failure
  is retained, and observation of the same process establishes completion without
  another restart, reset or longer deadline.
- Before and after restart, 64 prefix/tail probes over 32 preselected files match
  the population hashes. Both phases observe 149 delivered L2 hits / 19,445,834
  validated bytes. The post-restart phase also fetches 153,125,184 source bytes
  for misses/read-ahead; this is not a zero-source or fully resident probe.

The original reader ends with zero adapter storage/decode errors, six optional
read-timeout fallbacks, 917,826 best-effort demotion drops/rejections and
12,390,854,656 engine-accounted written bytes. Drops are not data loss from the
authoritative store, and written bytes are not populated file capacity.

## Serving failures that remain open

The completed replay records seven truncated bodies and eleven socket-read
timeouts. Five truncations route through the changed common FUSE mount; two route
through other mounts. The eleven timeouts concern other local/proxy roots. HTTP
outcomes also include 4,692 responses with status 429, three 502s and two 504s.
Existing partial-reader proxies, differing site inventory and changing remote
state prevent attributing every status difference to this reader.

Four common-mount `.apk` truncations coincide with nginx `sendfile()` EIO and a
reader READ `EAGAIN / admission_full` event. Later full reads of these four files
match an independent peer; that does not erase the original failures. Reader
counters retain 31 READ EAGAINs, one READ timeout and one LOOKUP timeout. The
immediate rejection in `Fuse::spawn_traced` predates this PR, but the visible
failures still block acceptance. Attribution of every truncation is incomplete.

The application uses fuser 0.18.0. Its supported
[background/congestion controls](https://docs.rs/fuser/0.18.0/fuser/struct.KernelConfig.html)
do not by themselves bound all foreground requests. Linux 7.0's
[FUSE read completion](https://github.com/torvalds/linux/blob/v7.0/fs/fuse/file.c)
passes errors to the
[buffered-folio completion path](https://github.com/torvalds/linux/blob/v7.0/fs/iomap/buffered-io.c).
Resolve native foreground backpressure and post-header failure behavior with a
causal regression before another acceptance claim; do not hide it by raising
admission limits or releasing actual-work ownership early.

## Resource observations

The table describes the **reader only** during the completed hourly replay.
Samples are approximately five seconds apart; late values are medians of the
last ten minutes. CPU includes concurrent population and metadata work. These
are observations, not proof of a plateau or an improvement over Go.

| Reader measurement | Sampled peak, GiB | Late median, GiB |
| --- | ---: | ---: |
| RSS | 48.0645 | 47.3964 |
| jemalloc allocated | 45.8524 | 45.6887 |
| Cgroup memory | 59.7198 | 51.6566 |
| Cgroup file memory | 12.4966 | 3.7527 |
| Cgroup kernel memory | 0.5522 | 0.4877 |

The process VmHWM is 48.0820 GiB and the cgroup high-water mark 59.7946 GiB;
these retain whole-mount lifetime scope, including earlier attempts. During the
hour, daemon user/system CPU deltas are 625.97/919.53 seconds and cgroup CPU is
1,545.499 seconds, with no throttling or memory high/max/OOM events. Initial FUSE
READ demand totals 272.333 GiB, giving 5.675 daemon CPU seconds per GiB at that
boundary, including metadata overhead; this is not a payload-only benchmark.
The namespace reaches 2,173,999 entries, well below the configured cap.

Do not omit consumer/kernel costs: a snapshot 17 seconds before completion shows
nginx cgroup memory **436.646 GiB**, including **428.815 GiB file memory** and
7.209 GiB kernel memory. That service also reads other mounts and proxies, and
earlier attempts warmed caches; this cannot all be attributed to L2. Separate
golist, rsync and collector cgroups use 0.526, 0.001 and 0.015 GiB at that point.
At that snapshot, nginx cumulative user/system CPU is 656.143/2,143.105 seconds
since consumer startup, including earlier attempts; it is not an isolated hourly
delta. Generator user/system CPU is 1,435.376/480.181 seconds at the same point;
its final accounting reports 1,923.168 total CPU seconds and about 314 MiB peak
cgroup memory. Aggregate consumer process RSS was not sampled.

The short post-restart reader observation has a cold L1 and only 1,149 namespace
entries. Its lower RSS is not a memory win. There is no matched Go 0.43.9 or A/A
arm, no large-namespace limit test, no full-capacity target recycling, and no
power-loss or ongoing-device-error injection in this trial.

## Final state and evidence

Temporary nginx configuration and client-only fail2ban exceptions are restored.
The admission marker stays absent, local and independent health checks return
404, and the owned sampler stops normally. The tested reader and private cache
remain available for the operator's decision; traffic admission was not restored.

Raw logs, commands, failed attempts, hashes, 1,518 monitoring samples and consumer
snapshots remain in private ignored evidence. All 37 copied top-level remote
evidence files match their origin hashes. No credentials or operational host
identities are included here. Commit `420f303` recorded this checkpoint without
runtime changes. The subsequent [admission correction](testing.md#fuse-admission-backpressure--2026-11-12)
reproduces and removes immediate overload EIO in local buffered-read/sendfile
fixtures. It was not part of this trial; unit/native/CI results do not rewrite
the serving failures or substitute for sustained target verification.
