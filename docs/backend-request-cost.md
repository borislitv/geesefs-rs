# Native backend request and payload cost

## Restore the missing metrics boundary

The pinned SeaweedFS 4.51 `mini` entry point accepts `s3.metricsPort` but starts
its shared Prometheus listener from `metricsPort`. The prior fixture passed the
former. A live baseline probe confirms healthy signed S3 listing while the metrics
endpoint refuses connection with errno 111. Historical missing measurements remain
missing; do not retroactively turn them into zeros.

Use `-metricsPort=9328` and require the S3 request metric before declaring startup
ready. This is the supported upstream flag, not a custom server/exporter. Verify
against exact upstream commit `153ab668c99731a6d29492bf6c61d0b4fa701101`:

- [mini flags and shared listener](https://github.com/seaweedfs/seaweedfs/blob/153ab668c99731a6d29492bf6c61d0b4fa701101/weed/command/mini.go).
- [action labels and traffic accounting](https://github.com/seaweedfs/seaweedfs/blob/153ab668c99731a6d29492bf6c61d0b4fa701101/weed/s3api/stats.go).
- [HEAD and GET share the GET action label](https://github.com/seaweedfs/seaweedfs/blob/153ab668c99731a6d29492bf6c61d0b4fa701101/weed/s3api/s3api_server.go).
- [object payload writes, including partial writes](https://github.com/seaweedfs/seaweedfs/blob/153ab668c99731a6d29492bf6c61d0b4fa701101/weed/s3api/s3api_object_handlers.go).

The corrected fixture passes native checks and serves metrics with HTTP 200.
All fourteen source/oracle and twenty-three benchmark Python cases pass in
Kubernetes. The fixture source SHA256 is
`246e2c3994fa07e8357e8557d2c0fb770ea3ab54dac28ff1fc4971eb17a50150`.
The flag wiring is checked against the actual pinned server, rather than a mock
assertion about command spelling.

## Exact-binary request inventory

One fresh/warm mount per frozen executable completes eight phases in 215.413 s.
All 24,000 full-file checks, before/after native hashes, 97 listings, 56 CPU windows,
99 artifact hashes and sixteen metrics scrapes reconcile. The fixture is still
3000 files / 8,298,727,528 bytes with manifest
`7bd537c12c702a614b51383a28ed167c86dbf2d964696525cdf8b4c2871555a1`.
Budgets, kernel, 2 CPU/4 GiB filesystem/client and 4 CPU/6 GiB backend quotas,
512 MiB payload, 100000 entries, four readers and complete metadata checks remain.

Use the public `prometheus-client` 0.23.1 parser for counter snapshots. Require
monotonic counters from the same live backend; do not mask resets or missing
scrapes. `GET` action counts include HEAD, so GET-labelled HTTP 200 is **not** a
count of full-body downloads. HTTP 206 identifies range responses. TrafficSent
counts written object payload, including partial writes, excluding LIST metadata
and HTTP headers.

The [full numeric receipt](backend-request-cost-20261028.json) retains CPU, memory,
latency and counter scopes. Decimal MB; RSS is peak / final-fifth mean, not a
stationary plateau. These are single-pass diagnostic values, **not repeated
timing acceptance**.

| Implementation/state | Payload bytes sent | Excess vs logical data | Range responses | LIST-labelled 200 | Daemon RSS peak / final-fifth MB | Whole cgroup CPU s | Read p99 ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Go experimental fresh | 8298727528 | 0.000% | 3884 | 7 | 1254.257 / 1222.578 | 41.825 | 19.322 |
| Go experimental warm-revisit | 8298727528 | 0.000% | 3885 | 0 | 1292.894 / 1237.133 | 41.927 | 20.400 |
| Go packaged fresh | 8298727528 | 0.000% | 3884 | 7 | 1274.122 / 1199.364 | 41.424 | 19.071 |
| Go packaged warm-revisit | 8298727528 | 0.000% | 3884 | 0 | 1328.112 / 1242.248 | 42.475 | 19.671 |
| Rust jemalloc fresh | 8620399499 | 3.876% | 3994 | 213 | 555.368 / 445.426 | 39.550 | 19.230 |
| Rust jemalloc warm-revisit | 8562870209 | 3.183% | 3898 | 0 | 555.999 / 477.730 | 39.747 | 20.856 |
| Rust System/tracked fresh | 8698409547 | 4.816% | 3960 | 213 | 576.242 / 556.954 | 37.401 | 18.372 |
| Rust System/tracked warm-revisit | 8589555003 | 3.504% | 3851 | 0 | 593.179 / 591.741 | 40.364 | 23.217 |

Both Go references transfer exactly the logical dataset. Rust transfers an extra
264–400 MB, depending on state/allocator; this does not by itself identify which
cache/admission/read-ahead path caused the repeated or unused payload. Fresh
Rust also has 3096 GET-labelled 200 and 96 GET-labelled 404 responses, versus
3000 and zero for Go. These are native request counters, not filesystem failures:
all logical byte/metadata oracles pass, and directory existence probes can return
404 legitimately. Do not remove required type/metadata checks to match the counts.

Continue from this measured extra work. A cache-only TinyLFU test does not establish
the filesystem cause: the hard byte-admission path can replace the entire cache
before Moka reaches its own capacity limit. Trace the ownership of admitted GETs,
independent fragment backings and pending reads before changing another policy.
Default allocator, byte limits, concurrency, integrity and production acceptance
are unchanged. Go heap series and the full representative matrix remain open.

The subsequent [fragment-admission transfer](fragment-admission-transfer.md)
reproduces retained permits for a freed independent prefix and checks a minimal
ownership correction before its matched performance comparison.
