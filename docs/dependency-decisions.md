# Dependency decisions

Use public maintained crates and supported APIs before custom infrastructure.
Cargo.lock pins exact public registry packages; no internal code is copied,
translated, vendored or linked. This package forbids unsafe code in every target.
License declarations are checked; the complete binary-distribution notice audit
remains outstanding. See LICENSE and NOTICE for source attribution.

## Selected mechanisms

| Concern | Selection and reason |
| --- | --- |
| FUSE | [fuser 0.18.0](https://docs.rs/fuser/0.18.0/fuser/) (MIT): protocol, callbacks and replies; no custom syscall/protocol stack. |
| S3 | [aws-sdk-s3 1.155.0](https://docs.rs/aws-sdk-s3/1.155.0/aws_sdk_s3/) and aws-config 1.13.0 (Apache-2.0): conditional ranges, streaming, retries, signing, profiles and multipart operations. Unneeded sigv4a is disabled. |
| Runtime/admission | [Tokio 1.54.1](https://docs.rs/tokio/1.54.1/tokio/) (MIT): owned semaphores, JoinSet, watch/mpsc, runtime and I/O; no custom executor. |
| Metadata cache | [Moka 0.12.16](https://docs.rs/moka/0.12.16/moka/) (MIT OR Apache-2.0): bounded weighted directory/preload caches with namespace/TTL validation. |
| Payload cache | [lru 0.19.0](https://docs.rs/lru/0.19.0/lru/) (MIT): synchronous `pop_lru` releases backing before required admission. Moka maintenance/policy changes cannot provide that boundary. |
| Streaming/owners | bytes 1.12.1, futures-util 0.3.34, tokio-util 0.7.20 (MIT OR Apache-2.0): immutable owners and maintained stream/I/O adapters. Small charged Block views retain complete backing permits. |
| Parsing/configuration | quick-xml 0.42.0 (MIT), clap/serde/serde_json/percent-encoding/thiserror (MIT OR Apache-2.0): bounded schema and compatibility adapters, not new parsers. |
| OS/allocator | rustix 1.1.5 (Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT) safe process APIs; tikv jemalloc 0.7 (MIT OR Apache-2.0) default allocator/statistics. No local unsafe/FFI. |
| System comparison | stats_alloc 0.1.10 (MIT) delegates to System; six requested-allocation counters are not native heap/RSS. peak_alloc was rejected because its realloc/calloc implementation changes the measured behavior. |
| Fixtures | tiny_http 0.12 (MIT OR Apache-2.0), rcgen/tempfile (MIT OR Apache-2.0), Moto and SeaweedFS (Apache-2.0); real HTTP/TLS boundaries plus faithful gated storage fakes. |
| CPU accounting | psutil 7.2.2 (BSD-3-Clause) process counters and owned cgroup `cpu.stat`; wall time is not CPU consumed. |
| Go heap accounting | Go's public `net/http/pprof` debug text exposes live `runtime.MemStats`. Python standard-library HTTP, response bounds and a narrow field adapter avoid implementing a heap-profile decoder or adding another runtime dependency. |

The pinned Go 1.25.9 [heap writer](https://github.com/golang/go/blob/go1.25.9/src/runtime/pprof/pprof.go)
prints `HeapAlloc` separately from sampled profile totals. Its
[HTTP handler](https://github.com/golang/go/blob/go1.25.9/src/net/http/pprof/pprof.go)
forces GC only when requested with a positive `gc` parameter; the benchmark never
sets it. Repeated bounded observations replace the former final `go tool pprof`
snapshot. Sampling overhead remains part of the observed server cost.

## Narrow application state

Inode lifetime, namespace bindings, dirty generations, flush ownership, sparse
overlays and fsync barriers are filesystem state, not replacement infrastructure.
Existing Tokio permits/watch/JoinSet bound work, coalesce overlapping fills and
cancel/drain jobs. Disjoint windows remain concurrent. Required and optional
admission differ: optional refill skips pressure, required FIFO must progress.
Contiguous consumption hints protect foreign unread cache backing only during
optional eviction; they never pin required admission. Reset adaptive Pattern on
zero-to-one open without discarding payload or active-reader history.

The idle flush-reserve cache policy uses the selected Tokio 1.54.1
[`RwLock`](https://github.com/tokio-rs/tokio/blob/tokio-1.54.1/tokio/src/sync/rwlock.rs)
and existing owned semaphore permits, not a new priority allocator. Concurrent
uploads hold shared `read_owned` activity guards; cache lending only attempts
`try_write` synchronously. The owned guard follows cloned upload budgets into
request bodies and child jobs. Lending must first acquire the unchanged flush
byte permits and may move only an exclusively cache-owned complete allocation.
An owned reply must copy loaned bytes into its already admitted data allocation;
it cannot export a revocable flush owner. Required admission alone initiates
loans; optional refill retains its nonblocking data-lane policy. Ownership tests
and measured whole-filesystem acceptance remain separate gates.

## Applicable read-path upstream findings

- [Mountpoint prefetch](https://github.com/awslabs/mountpoint-s3/blob/f58dbc52fec09394ee60d285bec1c3e082f06391/mountpoint-s3/src/prefetch.rs#L218-L289)
  prepares future work before buffered return. Merged [#556](https://github.com/awslabs/mountpoint-s3/pull/556)
  addresses [#488](https://github.com/awslabs/mountpoint-s3/issues/488) reordered
  reads; merged [#505](https://github.com/awslabs/mountpoint-s3/pull/505) retains
  cancellation handles. Reuse the scheduling principle, not an unbounded queue.
- [Goofys #38](https://github.com/kahing/goofys/pull/38) and
  [RocksDB #9674](https://github.com/meta/rocksdb/pull/9674) overlap consumption and
  refill. [S3FS #214](https://github.com/s3fs-fuse/s3fs-fuse/issues/214) is merely
  closed as a duplicate of [#95](https://github.com/s3fs-fuse/s3fs-fuse/issues/95),
  not proof of a delivered fix. [RocksDB #13774](https://github.com/meta/rocksdb/issues/13774)
  is a cancellation warning, not an applicable delivered solution.
- [bytes #840](https://github.com/tokio-rs/bytes/issues/840) and
  [#856](https://github.com/tokio-rs/bytes/pull/856) concern consumed shared-owner
  retention. The selected Tokio StreamReader retains an empty Bytes until the
  next poll. Use maintained futures `into_async_read` plus Tokio compat to release
  fully consumed chunks promptly. No transport fork/pool is required.
- Tokio `copy_buf`/`take` and Vec's writer initialize only copied bytes. Preserve
  exact length/EOF and SDK whole-body checksum barriers when publishing fragments.
  Split existing permits to actual backing allocations, not visible slices.
- [Moka #498](https://github.com/moka-rs/moka/issues/498) is a maintenance/tail
  report. [#412](https://github.com/moka-rs/moka/issues/412) concerns an async loop
  fixed before selected sync 0.12.16; it is not this workload's diagnosis.
- Public Go reference `5c0884c9e8799bbab93e2d4102fdc15e9c079e0d`
  (Apache-2.0) initializes history in `core/file.go::NewFileHandle` on every open.
  Its active requested ranges do not guarantee preservation of every future unread
  byte. No Go SDK implementation is vendored.

## Backend adapters and rejected alternatives

- aws-smithy-runtime-api public identity/auth traits retain SDK request ownership
  for metadata-token IAM. Reqwest 0.12 (MIT OR Apache-2.0) owns metadata HTTP/TLS
  and the signed extended-list/PATCH adapter; serde_json/quick-xml own syntax.
- [s3s-sigv2 0.17.0](https://docs.rs/s3s-sigv2/0.17.0/) (Apache-2.0) supplies
  SigV2 canonicalization/HMAC. base64/md-5 provide SSE-C primitives. No handwritten
  cryptography or second credential chain. Credentials-process is disabled because
  the inspected SDK shell execution lacks the required bounded kill-on-drop.
- For explicit no-verify-ssl only, a narrow Smithy public-trait adapter uses
  reqwest streaming. Evaluated aws-smithy-http-client-reqwest 0.1 buffers complete
  responses; smithy-transport-reqwest 0.1 lacks custom TLS configuration. Default
  verification and IAM metadata TLS remain enabled.
- MIME policy uses maintained `mime` syntax and a bounded Unix/default adapter;
  mime_guess does not match Go's defaults/database precedence. Default associations
  derive from public Go `mime/type.go` and `type_unix.go` (Go Authors, BSD-3-Clause).
  unicode-general-category (Apache-2.0) supplies metadata printing categories.
- http-body/Smithy retryable bodies frame configured large upload parts without
  whole-part buffering. A narrow exact-length charged adapter is application
  materialization; HTTP/retries/checksums stay library-owned. crc32fast independently
  validates test wire trailers.

## Investigation boundary

The selected SDK already contains [interceptor optimizations #4597](https://github.com/smithy-lang/smithy-rs/pull/4597)
and [BDD endpoint improvements #4595](https://github.com/smithy-lang/smithy-rs/pull/4595),
including a default resolver cache. Endpoint params include operation-specific
key/prefix values; do not invent a bucket-only cache or remove auth properties.
Client-wide SDK hooks and a transparent default-resolver timing adapter isolate
parts of the observed cost without replacing signing, transport or endpoint logic.
The measured setup/signing/deserialization work persists without external
perf/BPF observers; no single applicable upstream defect is established.

Extra initial-read client time is not yet an attributed library defect. Check
exact versions, profiles, upstream issue applicability and supported APIs before
another workaround. Do not remove integrity, freshness, ownership or durability
contracts for speed. [FUSE passthrough](fuse-passthrough.md) and disk caching remain
deferred. Keep new findings concise; detailed profiles/results stay untracked.

The subsequent fixed-binary warm-random controls and FUSE/GET joins show repeated
Rust cache misses where both Go references issue no GET. This is an application
capacity-policy difference, not evidence of a single SDK defect. GeeseFS
[#70](https://github.com/yandex-cloud/geesefs/issues/70) explains that its payload
limit excludes metadata/runtime overhead; lowering that limit is not a fix here.
[#82](https://github.com/yandex-cloud/geesefs/issues/82)'s metadata-cache limit was
delivered in 0.37.0. [#117](https://github.com/yandex-cloud/geesefs/issues/117)
concerns deferred disk caching; closed
[#172](https://github.com/yandex-cloud/geesefs/issues/172) has no maintainer
diagnosis or delivered correction. SDK
[#1349](https://github.com/awslabs/aws-sdk-rust/issues/1349)'s large-object/network
concurrency discussion is not a ready-made small-random latency fix.
[#958](https://github.com/awslabs/aws-sdk-rust/issues/958)'s TCP_NODELAY setting is
already enabled by the selected transport. No dependency replacement follows
from these discussions.

The 2026-10-31 native FUSE investigation attributes a 30-second wait to application
admission: an in-flight fill publishes reclaimable cache owners after the initial
sweep, but waiting for permits alone cannot reclaim them. Tokio
[#6450](https://github.com/tokio-rs/tokio/issues/6450) documents FIFO behavior,
not an applicable semaphore defect. Reuse selected Tokio 1.54.1
[`Notify`](https://docs.rs/tokio/1.54.1/tokio/sync/struct.Notify.html):
`notify_waiters` covers notification futures created before the cache check even
when not yet polled. Keep the same pinned reservation instead of cancelling and
rejoining the FIFO queue. No dependency replacement or new allocator is needed.
GeeseFS [#28](https://github.com/yandex-cloud/geesefs/issues/28) discusses slow
listing/HEAD work; the measured repeated LISTs here instead follow our global
directory-page epoch invalidation after unrelated writes.
