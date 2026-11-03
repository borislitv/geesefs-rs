# Metadata expiry: request cost and reference freshness — 2026-11-01

Status: **UNACCEPTED**. This is instrumented attribution and a compatibility
observation, not a runtime optimization or a new performance checkpoint. Runtime
remains `1b13033`; all 85 frozen v8 inputs and original executable hashes match.
The [current mixed rates, tails, CPU and memory](performance-observation-age.md)
retain their losses and scope.

## Expiry capture

Use the unchanged Rust v8 and experimental/packaged Go binaries identified in the
current checkpoint, Linux 7.0, SeaweedFS 4.51, eight workers, 64 MiB payload budget,
100,000 entries, 1,024 metadata files and the checked 256 MiB payload. Each separate
mount runs 14 metadata traversals paced at 10 seconds, with the default 60-second
stat TTL. CPU is equally unbounded; filesystem/client, backend and owned-parent
memory ceilings remain 16/12/32 GiB. All 42 phases / 43,008 entries and three final
full-byte/generation checks pass. This metadata-only trace is distinct from the
earlier mixed workload and is not an acceptance timing run.

Capture bidirectional loopback HTTP with full snap length and sample each daemon
at 999 Hz using `perf record --clockid mono -e cpu-clock --call-graph dwarf,8192`.
Join calibrated monotonic phase clocks to TCP/HTTP request/response frames through
standalone TShark 4.6.4, rather than a custom TCP reassembler. Flat-prefix and marker
queries are LIST operations too; classifying only delimiter queries mislabels Go
preload scans as payload GETs. Captures report zero kernel drops, no truncated
packets and matched responses; profiles are readable with no reported lost samples.
An intentional SIGINT can give perf exit -2 and still produce a valid recording.

The table includes every phase with HTTP traffic; all other phases have none.
Cycle 0 is initial traversal, not sustained expiry. CPU is consumed time and can
exceed wall time. In-flight union is wall time with at least one outstanding HTTP
operation; it is **not** daemon off-CPU time. TShark completion includes response
body reassembly for LIST. CPU, wire intervals and scheduler work overlap and must
not be added as disjoint latency components. Streams are connections used in the
phase, not necessarily newly opened connections.

| Binary | Cycle | HEAD | LIST | Wall ms | Daemon CPU ms | Cgroup CPU ms | In-flight union ms | HTTP pooled p99 ms | Peak in-flight | Streams | CPU samples |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| rust-candidate | 0 | 1026 | 3 | 212.22 | 240.00 | 263.30 | 155.68 | 4.175 | 19 | 55 | 226 |
| rust-candidate | 6 | 2 | 1 | 40.84 | 20.00 | 20.77 | 22.42 | 20.497 | 1 | 1 | 12 |
| rust-candidate | 7 | 1024 | 3 | 166.54 | 220.00 | 226.03 | 145.30 | 2.145 | 26 | 48 | 207 |
| rust-candidate | 12 | 2 | 0 | 14.24 | 20.00 | 14.29 | 2.19 | 1.344 | 1 | 1 | 7 |
| rust-candidate | 13 | 0 | 1 | 37.88 | 10.00 | 21.54 | 18.73 | 18.728 | 1 | 1 | 14 |
| go-experimental | 0 | 0 | 4 | 250.94 | 340.00 | 357.49 | 142.42 | 60.513 | 1 | 1 | 322 |
| go-experimental | 7 | 2 | 4 | 169.13 | 250.00 | 264.61 | 59.46 | 27.639 | 3 | 3 | 241 |
| go-packaged | 0 | 0 | 4 | 214.99 | 310.00 | 331.22 | 127.67 | 58.185 | 1 | 1 | 305 |
| go-packaged | 7 | 2 | 4 | 157.96 | 220.00 | 219.10 | 56.58 | 27.889 | 3 | 3 | 198 |

Rust cycle 7 has 207 samples / 207.21 ms of sampled CPU period. Exclusive leaf
categories include SDK/signing/runtime 67.07 ms, kernel 51.05 ms, HTTP/URL 16.02 ms,
allocator 9.01 ms and unresolved 29.03 ms. The SDK category includes orchestration,
headers, signing and decoding; it does not prove that all its cost is setup or
identify one removable application allocation. Cached phases have only 2–3 Rust
samples even at 999 Hz. The prior 99 Hz trace had at most 23 samples per phase.
Neither supports precise cached-path subroutine attribution. Fewer RPCs alone do
not establish lower total CPU: both Go implementations still decode listings and
perform runtime/GC work. No new process-memory or plateau claim follows here.

## Mode-only mutation at an unchanged object ETag

A separate native S3/FUSE probe uses the same three executables, permissions and
mtime enabled, equal **4-second** stat TTL, 64 MiB budget and 100,000 entry limit.
This shorter TTL belongs only to the compatibility probe above disposable data;
it does not replace the unchanged-default performance comparison.

For each binary and ordering, put a 20-byte file with decimal POSIX `mode=33184`
(`S_IFREG | 0640`), stat it, then replace only metadata with `mode=33152`
(`S_IFREG | 0600`) using identical bytes. Independent S3 HEAD verifies unchanged
ETag/size and the changed metadata. Wait at least 4.5 seconds after the initial
stat, then either stat directly or enumerate the directory before stat. Verify
full bytes independently of the mode assertion. As a positive control, change one
byte at the same length and set `mode=33216` (`S_IFREG | 0700`), verify a changed
ETag, wait another 4.5 seconds and repeat stat/full-byte checks.

| Binary | Ordering after expiry | Initial mode, expected 0640 | Unchanged-ETag mode, expected 0600 | Changed-ETag control, expected 0700 |
| --- | --- | ---: | ---: | ---: |
| rust-candidate | stat-only | 0640 | 0600 | 0700 |
| rust-candidate | list-before-stat | 0640 | 0600 | 0700 |
| go-experimental | stat-only | 0644 | 0644 | 0700 |
| go-experimental | list-before-stat | 0644 | 0644 | 0700 |
| go-packaged | stat-only | 0640 | 0640 | 0700 |
| go-packaged | list-before-stat | 0640 | 0640 | 0700 |

All six changed-ETag controls pass, all bytes match and all daemons exit cleanly.
Both Go stat-only traces receive the new `0600` metadata in a successful HEAD but
return the prior mode. The list-before-stat traces issue LIST without a file HEAD;
Rust issues HEAD and returns `0600`. Experimental Go's initial flat preload
contains no user metadata and initially returns default `0644`; that initial
failure is retained rather than omitted. The same mode outcomes recur in the
earlier two probe runs.

Exact public source for both pinned Go revisions explains why equal stat TTL
does not imply equal permission freshness:

- `core/handles.go::SetFromBlobItem` applies metadata inside the ETag/size-change
  branch and advances `AttrTime` afterwards. An unchanged content ETag cannot
  validate arbitrary user metadata.
- `core/dir.go::{LookUp,LookUpCached}` can satisfy names from preload/listing;
  standard LIST lacks user metadata, so it cannot prove unchanged permissions.
- `core/goofys_fuse.go::GetInodeAttributes` returns cached attributes and gives
  them another kernel TTL without fetching metadata itself.

These are observations of a disposable SeaweedFS fixture, not an actual-service
or universal Go failure claim. A mode-only failure even without explicit listing
also means that LIST renewal alone is not the complete explanation. Preserve the
reference's recorded performance; do not reinterpret its lower request count as a
correctness-equivalent way to remove required Rust HEADs.

## Outcome and evidence scope

Retain the observation-age correction and unchanged runtime. Reinspection of
`aws-sdk-s3 1.155.0` finds generated operation orchestration/plugin assembly is
crate-private, not a supported prepared-operation interface. Public SDK
[#1349](https://github.com/awslabs/aws-sdk-rust/issues/1349) discusses bulk transfer
and concurrency; it is not a delivered fix for this measured metadata setup cost.
Do not bypass signing, checksums, exact metadata or validation to remove work.
Future setup/retained-owner changes need an attributable supported API, a
failure-sensitive contract test and source-bound repeated joint measurements.

Raw command receipts, binary/source hashes, profiles, captures and reducers remain
in ignored `benchmark-results/private/vm-20261031-integrated/`: `v8-expiry-wire-v1`
with `wire-attribution-v3`, `reference-freshness-v3` and
`expiry-evidence-sealed-v3`. Preserve the earlier partial-capture verification
failure: zero reported kernel drops did not guarantee the last buffered packets
were drained. The final probe uses supported tcpdump immediate mode, a bounded
post-unmount drain and matched request/response witnesses. Go's concurrent marker
probes can finish after stat returns; count started requests without pretending
they are all synchronous dependencies. The existing retained-source 260-case
allocator verification and native regression scope remain unchanged. Compatibility,
whole-file quality and joint performance acceptance remain open.
