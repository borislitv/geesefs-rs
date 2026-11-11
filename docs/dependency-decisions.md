# Dependency decisions

Use public maintained crates and supported APIs before custom infrastructure.
Cargo.lock pins exact public registry packages and the explicitly authorized
public Foyer correction below; no internal code is copied,
translated, vendored or linked. This package forbids unsafe code in every target.
License declarations are checked; the complete binary-distribution notice audit
remains outstanding. See LICENSE and NOTICE for source attribution.

## Throttling in the existing extension adapter — 2026-11-08

The supported aws-smithy-types 1.9.0 RetryConfig API already provides exponential
backoff with jitter for SDK requests; retain that implementation. Extended LIST
uses the existing reqwest adapter because the S3 SDK does not expose ext-v1.
Adjust its existing retry loop with getrandom 0.4.3 (already locked) and Tokio
sleep/timeout, without a second retry framework or new dependency. The policy
follows the public Apache-2.0 Go GeeseFS 0.43.9 AWS SDK configuration: throttling
starts at 500 ms with jitter and doubles between retries. Numeric Retry-After
cannot extend the aggregate deadline. This is an independently implemented
policy adapter, not a replacement HTTP/signing/retry framework.

Sources: [Go SDK retryer](https://github.com/aws/aws-sdk-go/blob/v1.55.8/aws/client/default_retryer.go),
[Go configuration](https://github.com/yandex-cloud/geesefs/blob/80c214ef14f620d93407454fc76ca01dd71fa25f/core/cfg/conf_s3.go),
[Rust RetryConfig API](https://docs.rs/aws-smithy-types/1.9.0/aws_smithy_types/retry/struct.RetryConfig.html).

## Selective disk payload cache qualification — 2026-11-09

Evaluate released [Foyer 0.22.6](https://docs.rs/foyer/0.22.6/foyer/) (Apache-2.0)
first as a development-only dependency. Its supported disk-only writer, single-file
device, region reclamation, checksum validation and recovery avoid a custom disk
format or one inode per small payload block. **The released engine fails the
reinsertion/recovery gate and is not selected for runtime.**
Qualification must establish memory ownership, recency adaptation, fixed-capacity
reuse, error/cancellation behavior and persistent inventory costs. Default FIFO
eviction is not the agreed approximate-LRU policy. The application retains its
existing charged L1; missing TTL support is irrelevant because payload TTL was
explicitly removed. Keep raw receipts outside tracked source.

`tests/disk_cache_engine.rs` qualifies the public API with a 16-MiB container,
1-MiB regions, 128-KiB records, 512-KiB configured buffer pool and one
flusher/reclaimer. The disk-only roundtrip/reopen and fixed-container recycling
checks pass. Default eviction loses a continuously read record at cold-scan step
106. The test-only `StorageFilterCondition`/`EvictionPicker` adapter uses the
already selected `lru` crate for at most 64 reference hashes and limits copied
records to one quarter of each reclaimed region. The hot record survives the
scan, and becomes evictable when access stops. This does not establish an
aggregate in-flight memory bound or an accepted runtime policy.

The combined persistence test fails: after inserting keys 0–255 while repeatedly
reading key 0, keys `[0, 210, 211, 212, 213, 214]` are readable after `close()` but
missing after reopen. This reproduces on both tmpfs and local ext4. The four-test
qualification result is **three passed, one failed**; the failing test is retained
without ignoring it or changing its expected behavior.

The crate's recorded upstream revision is
`dd46245c45071d1036331e4e2c48e15386017b96` (tag `v0.22.6`).
[Reinsertion preserves the original sequence](https://github.com/foyer-rs/foyer/blob/dd46245c45071d1036331e4e2c48e15386017b96/foyer-storage/src/engine/block/reclaimer.rs#L137-L159),
whereas [recovery stops a region when sequences decrease](https://github.com/foyer-rs/foyer/blob/dd46245c45071d1036331e4e2c48e15386017b96/foyer-storage/src/engine/block/recover.rs#L164-L198).
An isolated checkout at that exact revision reproduces the failure. Temporarily
removing only the decreasing-sequence cutoff makes the same test pass; restoring
the source restores the same missing-key failure. The checkout was restored.
This was a causal diagnostic, **not the adopted fix**: the cutoff also distinguishes
old region tails. The maintained correction below preserves that distinction.

At inspection, [v0.22.6 remains the latest release](https://github.com/foyer-rs/foyer/releases/tag/v0.22.6).
The newer [recovery optimization](https://github.com/foyer-rs/foyer/commit/4e5ca058d3552d963cdc47289094bdd118ea4584)
addresses [large-index costs](https://github.com/foyer-rs/foyer/issues/1295), but retains
the decreasing-sequence cutoff; it does not resolve this incompatibility.
The released result is retained independently of the maintained fork's subsequent
qualification. Filesystem integration and performance acceptance remain separate.

The operator subsequently authorized a maintained fork and other engine trials
without another decision checkpoint. Development and PR delivery are confined to
the operator's repositories; external upstream submissions require a separate
explicit decision. Remain on the maintained fork until equivalent upstream fixes
are independently verified. The first correction
will separate physical region incarnation from logical entry sequence: checksum-
protected blob indexes carry a shared random incarnation for each new region.
Recovery/reclamation must reject a previous incarnation's tail while allowing old
logical sequences from valid reinsertions. Preserve legacy decoding and make old
readers reject the new index format safely. This narrow format correction is
necessary because the released picker/filter APIs cannot fix recovery's sequence
assumption; it is not a replacement storage engine. Keep legacy/reuse/corruption
tests and the unmodified released RED as independent controls.

Select public fork revision `d60dd0462c1de1e0853590791371208b659c90bf`
for the isolated adapter, maintained in [fork PR 1](https://github.com/borislitv/foyer/pull/1).
The released crate remains unsuitable. Corrections separate physical region
incarnation from logical sequence, join shutdown ownership, validate recovered
record geometry, bound CLOCK index slots and stream recovery. A circular
tombstone log now persists the maximum sequence whose invalidation history was
actually overwritten; otherwise completed deletions could resurrect after wrap.
Unwrapped logs retain old completed records. Unsafe history returns an error,
without automatically clearing inventory. Older readers must not open upgraded logs.

Guarded callers use `Store::{try_enqueue, try_delete, load, wait_guarded}`.
Byte/item credits precede index mutation and follow actual I/O; admission failure
means not accepted, not a completed deletion. Serialized record size is capped
before read allocation, and the adapter must permit only `Compression::None`.
HybridCache's automatic writer/eviction paths use legacy void APIs and are **not**
compatible with guarded mode. Caller backing, bounded codecs, fixed pools and
index funding remain application responsibilities. No new journal or startup
dirty-marker inventory reset is used. Process-interruption fixtures preserve
unrelated completed records; this is not a power-loss durability guarantee.

The exact fork passes 130 affected native tests, three explicitly configured
doctests, default-feature package checks and all-target Clippy. Tests include
real-file collision/corruption, queue exhaustion, interrupted child processes,
tombstone wrap, lifecycle and 30 small geometry fixtures. The audit exception
permits only the exact public repository, revision and six workspace packages.

Resource qualification of the same bounded CLOCK representation recovered
8,388,608 tiny records: 1,837.48 MiB requested live memory and up to 1,984.96 MiB
observed RSS after touches. This supersedes the earlier unbounded-index estimate;
the order tree must be funded too. A sparse 1-TiB log-enabled open kept scratch
below 2 MiB but incurred about 3.75 GiB of file-cache residency. These are
index/geometry experiments, not a populated 1-TiB payload benchmark or a whole-RSS
bound. Retain raw source hashes, CPU/cgroup accounting and sampling limitations
outside tracked source. Integration must disclose the resulting L1 reduction and
test ownership/fallback under its actual configuration before accepting the feature.

For the independent CLI work, reuse maintained public
[regex 1.13.1](https://github.com/rust-lang/regex) (MIT OR Apache-2.0) with bounded pattern
and compiled-program sizes, and [bytesize 2.7.0](https://github.com/bytesize-rs/bytesize)
(Apache-2.0) for human-readable capacity parsing. Cargo.lock pins their selected
registry releases; do not introduce another regex engine or custom unit parser.

For backend namespace separation, reuse maintained public RustCrypto
[sha2 0.11.1](https://docs.rs/sha2/0.11.1/sha2/) (MIT OR Apache-2.0), already in the
SDK dependency graph. Its incremental SHA-256 API hashes length-delimited access
and encryption context without persisting raw credentials. A digest does not
prove an opaque credential provider is static: only explicitly known stable or
anonymous contexts can authorize cross-process reuse. Do not infer a principal
from profile names or a single non-expiring credential resolution.

## Native Range fixture and buffered device qualification — 2026-11-11

Reuse the project's existing [SeaweedFS 4.51](https://github.com/seaweedfs/seaweedfs/releases/tag/4.51)
`weed mini` fixture (Apache-2.0), with sealed read-only and separate disposable
read/write service instances, for measured S3 workloads. Do not implement a new
S3 server or patch request semantics. Public SDK contracts check exact Range/EOF,
If-Match, HEAD, listing pagination, metadata, COPY and multipart operations.
The explicit local executable is verified; tests do not silently download it.

Moto 5.1.17's full-object-before-Range path made 64 small reads of a 128-MiB object
read 8 GiB through process syscalls for 8 MiB of response payload. The qualified
SeaweedFS run read 16.54 MiB. Both had zero media `read_bytes` in this cached probe;
this is provider logical-I/O evidence, not cold-device throughput. Moto 5.2.5
retains that implementation shape, so an upgrade alone is not the correction.
Keep Moto for existing functional fault injection. SeaweedFS's larger absolute
provider RSS remains separately visible; lower transient Range cost is not a
whole-fixture memory win.

The maintained Foyer `FileDeviceBuilder::with_direct(true)` API was also measured
in an isolated variant. Eight ext4 pairs plus eight buffered A/A controls reduced
cgroup peak memory but increased hot-read CPU and latency substantially; the
tmpfs control did not establish direct-I/O semantics. Retain buffered runtime I/O
rather than adopting that variant globally. This is a negative result for the
tested backing/API, not proof that all kernel-cache optimization is impossible.
Keep raw aligned-I/O, filesystem and resource receipts outside tracked source.

## Metadata byte admission — 2026-11-10

The operator chose independent opt-in `--metadata-limit`; payload `--memory-limit`
retains its existing meaning. Reuse the existing Tokio owned-semaphore `Budget`
for lifetime charges, and retain Moka 0.12.16 for replaceable listing pages.
Foyer's weighted memory cache also cannot own canonical inode identity, kernel
lookup/open pins or an allocation moved out of a cache entry. Replacing L1 with
another weighted cache would not solve those ownership requirements.

Small application owners carry charges across inode state, returned directory
names/xattrs, listing snapshots, writer snapshots and namespace-operation plans.
Reserve variable-length encoding/copy work before construction or remote mutation;
settlement releases credit rather than assuming late admission cannot fail.
Namespace pressure examines at most 128 candidates per locked batch and yields
between batches, preserving canonical pins and surfacing final exhaustion as
`ENOMEM`. The existing entry count remains an additional bound. Disabled byte
admission preserves prior defaults and avoids accounting-only HEAD requests.

This is application metadata admission, not allocator accounting or a total-RSS
limit. SDK/parser allocations, allocator rounding/container slack, runtime and
kernel memory remain separate measured costs. No production limit is selected.

The subsequent compaction uses standard-library immutable sorted boxed pairs,
slice binary search and `Arc` ownership with the existing admission wrapper.
No global interner, custom hash table or new cache dependency is needed. Clean
attributes share one canonical allocation with their committed baseline; COW
writers retain distinct versions only when required. Shared keys similarly
preserve namespace/snapshot lifetime. Transport metadata and HTTP properties stay
at the backend boundary rather than in every cached inode. Preserve the existing
owned flush gate, conditional reads and private-receipt recovery contract.

## Selected mechanisms

| Concern | Selection and reason |
| --- | --- |
| FUSE | [fuser 0.18.0](https://docs.rs/fuser/0.18.0/fuser/) (MIT): protocol, callbacks and replies; no custom syscall/protocol stack. |
| S3 | [aws-sdk-s3 1.155.0](https://docs.rs/aws-sdk-s3/1.155.0/aws_sdk_s3/) and aws-config 1.13.0 (Apache-2.0): conditional ranges, streaming, retries, signing, profiles and multipart operations. Unneeded sigv4a is disabled. |
| Runtime/admission | [Tokio 1.54.1](https://docs.rs/tokio/1.54.1/tokio/) (MIT): owned semaphores, JoinSet, watch/mpsc, runtime and I/O; no custom executor. |
| Upload provenance | [getrandom 0.4.3](https://docs.rs/getrandom/0.4.3/getrandom/) (MIT OR Apache-2.0), already locked transitively: safe OS randomness for 128-bit per-upload receipts; no custom entropy or cryptography. |
| Metadata cache | [Moka 0.12.16](https://docs.rs/moka/0.12.16/moka/) (MIT OR Apache-2.0): bounded weighted directory/preload caches with namespace/TTL validation. |
| Payload cache | [lru 0.19.0](https://docs.rs/lru/0.19.0/lru/) (MIT): synchronous `pop_lru` releases backing before required admission. Moka maintenance/policy changes cannot provide that boundary. |
| Streaming/owners | bytes 1.12.1, futures-util 0.3.34, tokio-util 0.7.20 (MIT OR Apache-2.0): immutable owners and maintained stream/I/O adapters. Small charged Block views retain complete backing permits. |
| Parsing/configuration | quick-xml 0.42.0 (MIT), clap/serde/serde_json/percent-encoding/thiserror (MIT OR Apache-2.0): bounded schema and compatibility adapters, not new parsers. |
| OS/allocator | rustix 1.1.5 (Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT) safe process APIs; tikv jemalloc 0.7 (MIT OR Apache-2.0) default allocator/statistics. No local unsafe/FFI. |
| System comparison | stats_alloc 0.1.10 (MIT) delegates to System; six requested-allocation counters are not native heap/RSS. peak_alloc was rejected because its realloc/calloc implementation changes the measured behavior. |
| Fixtures | tiny_http 0.12 (MIT OR Apache-2.0), rcgen/tempfile (MIT OR Apache-2.0), Moto and SeaweedFS (Apache-2.0); real HTTP/TLS boundaries plus faithful gated storage fakes. |
| CPU accounting | psutil 7.2.2 (BSD-3-Clause) process counters and owned cgroup `cpu.stat`; wall time is not CPU consumed. |
| Diagnostic output | [tracing-appender 0.2.5](https://docs.rs/tracing-appender/0.2.5/tracing_appender/non_blocking/) (MIT): supported bounded lossy `NonBlockingBuilder`, off-thread stderr and drop counter; no custom queue, global tracing subscriber or synchronous writer in the detailed FUSE path. |
| Go heap accounting | Go's public `net/http/pprof` debug text exposes live `runtime.MemStats`. Python standard-library HTTP, response bounds and a narrow field adapter avoid implementing a heap-profile decoder or adding another runtime dependency. |
| Redistribution inventory | [cargo-about 0.9.3](https://github.com/EmbarkStudios/cargo-about/releases/tag/0.9.3) (MIT OR Apache-2.0) plus Cargo's source-vendor command: verified locked HTML/native-source notice bundle. Checksum-specific webpki clarification retains ISC/BSD/Apache texts; bundled C/assembly/file notices remain with full upstream sources. See [distribution](distribution.md). |

The pinned Go 1.25.9 [heap writer](https://github.com/golang/go/blob/go1.25.9/src/runtime/pprof/pprof.go)
prints `HeapAlloc` separately from sampled profile totals. Its
[HTTP handler](https://github.com/golang/go/blob/go1.25.9/src/net/http/pprof/pprof.go)
forces GC only when requested with a positive `gc` parameter; the benchmark never
sets it. Repeated bounded observations replace the former final `go tool pprof`
snapshot. Sampling overhead remains part of the observed server cost.

## Narrow application state

### Severe backend failure context — 2026-11-07

Reuse the locked aws-smithy-runtime-api 1.19.1
[finalizer context](https://docs.rs/aws-smithy-runtime-api/1.19.1/aws_smithy_runtime_api/client/interceptors/context/struct.FinalizerInterceptorContextRef.html)
and [typed orchestrator predicates](https://docs.rs/aws-smithy-runtime-api/1.19.1/aws_smithy_runtime_api/client/orchestrator/struct.OrchestratorError.html)
in the existing interceptor. Reuse reqwest 0.12.28 status and typed error
predicates for extension requests, and the existing backend/body validation
boundaries for streaming failures.
These supported boundaries expose status, failure class and completed attempt
count without formatting credentials, URLs, headers or remote error text.
Keep only the last failed execution per closed operation plus its count in each
request trace; serialize that bounded summary with severe records through the
existing serde_json/tracing-appender path. No new subscriber, transport, queue,
error-string parser, dependency or public Error variant is required.
Body failures occur after SDK finalization; their status and attempt count remain
explicitly unknown rather than borrowing another concurrent request's response.

### Debug trace output — 2026-11-07

Keep the already locked tracing-appender 0.2.5 `NonBlockingBuilder` and its
supported `buffered_lines_limit`, `lossy(true)` and enqueue-drop observations.
Use 256 queued records only for explicit debug; retain 16 normally. Request/phase
identities and closed-vocabulary fields are application state serialized through
existing serde_json. No custom queue, subscriber, OpenTelemetry runtime or new
dependency is needed to expose these existing observation boundaries.

### READDIRPLUS metadata pipeline — 2026-11-06

Reuse [futures-util 0.3.34 FuturesOrdered](https://docs.rs/futures-util/0.3.34/futures_util/stream/struct.FuturesOrdered.html)
for concurrent borrowed futures with ordered output, and the existing Tokio
metadata semaphore for a global bound. Create a network future only after its
permit; cached attributes return a ready future. Cancellation drops the queue
in the request, without detached tasks or a new executor/cache/dependency.

Reviewed the pinned [fuser 0.18.0 release](https://github.com/cberner/fuser/releases/tag/v0.18.0)
and [entry/attribute TTL issue 756](https://github.com/cberner/fuser/issues/756).
The latter concerns separate reply TTLs, not our sequential S3 attribute refresh;
keep the current supported reply/TTL API. fuser retains parsing, frame capacity
and exactly-once wire replies. The application changes only request scheduling.

### Namespace progress — 2026-11-06

Select released [papaya 0.2.5](https://docs.rs/papaya/0.2.5/papaya/)
(MIT, [maintained upstream](https://github.com/ibraheemdev/papaya)) for a
lock-free inode-number index. Its `pin().get()` returns a protected `Weak<Inode>`;
upgrade to an owned `Arc` and drop the pin before taking inode locks or awaiting.
Namespace bindings and their single canonical strong inode owner remain under
the existing transaction mutex. Unlike another strong-value cache, this index
does not pin eviction victims. `Arc::try_unwrap`, not a racy strong-count check,
decides eviction against concurrent weak upgrades. No custom atomic pointers,
unsafe code, reclamation scheme or process-wide shared collector is introduced.

Evaluated standard `RwLock` and DashMap: readers can still block behind a writer
or shard lock. Moka is already appropriate for replaceable pages, but its policy
maintenance and strong value ownership do not provide canonical inode lifetime.
Copy-on-write `arc-swap` of the complete map copies the namespace on insertion.
Papaya provides the required independent lookup without those tradeoffs; ordered
prefix transactions remain in the existing BTreeMaps rather than pretending
multi-name rename is a lock-free single-key operation.

Reviewed [0.2.5 release notes](https://github.com/ibraheemdev/papaya/releases/tag/v0.2.5)
(blocking-resize deadlock fix) and open
[shared-collector issue 102](https://github.com/ibraheemdev/papaya/issues/102).
Use one default, privately owned collector per filesystem index, never
`shared_collector`; map destruction cannot race a pin borrowing that same map.
Do not iterate the hash index or retain a pin across blocking/async operations.

Dirty generations use an application-owned ordered key/inode index updated under
the existing namespace/state lock order. This is filesystem generation state,
not a replacement concurrent container. Flush snapshots visit only dirty inodes;
descendant existence is a prefix query. Directory overlays seek past descendants
in the existing ordered names map and yield between bounded transactions.

### Prometheus observation — 2026-11-03

Select released [prometheus-client 0.25.2](https://docs.rs/prometheus-client/0.25.2/prometheus_client/)
(Apache-2.0 OR MIT) for counters, gauges and OpenMetrics text encoding, rather
than handwritten exposition/escaping. Serve the opt-in endpoint through the
already locked Hyper 1.12.0, hyper-util 0.1.21 Tokio I/O and http-body-util 0.1.5
(MIT), with two concurrent connections and five-second connection deadlines.
Tiny HTTP 0.12 is already a fixture dependency, but its per-connection background
threads do not provide the intended bounded async connection lifecycle. A full
web framework/exporter runtime is unnecessary for one GET route. Use the existing
jemalloc/stats_alloc APIs; the narrow Linux RSS adapter reads only self status.
Snapshot existing cache/namespace cardinalities under their current locks; never
scan inodes, reserve payload, refresh caches or contact storage during a scrape.

For phase diagnosis reuse 0.25.2's supported
[classic Histogram](https://docs.rs/prometheus-client/0.25.2/prometheus_client/metrics/histogram/struct.Histogram.html)
with explicit seconds buckets and prebound handles. Native-only histograms cannot
use this endpoint's OpenMetrics text encoding. Histograms have an internal mutex;
measure their observer cost rather than claiming lock-free instrumentation.
Use Tokio's task-local scope for request ownership across awaits, explicitly
propagating it into owned read jobs. Small lifetime guards describe application
reply/phase boundaries; no tracing/exporter framework or custom histogram is needed.

The maintained tracing-appender 0.2.5 release supports Rust 1.63 and adds the
nonblocking writer without requiring a global tracing subscriber. Its
[release notes](https://github.com/tokio-rs/tracing/blob/tracing-appender-0.2.5/tracing-appender/CHANGELOG.md)
and [implementation](https://github.com/tokio-rs/tracing/blob/tracing-appender-0.2.5/tracing-appender/src/non_blocking.rs)
confirm lossy `try_send`, fixed queue capacity and a finite shutdown flush.
Use 16 queued records, retain `WorkerGuard` with the filesystem observations and
export enqueue drops. Format one bounded record only after filtering/rate admission.
The default 128,000-line queue is unnecessary. Direct `eprintln!` can hold a runtime
worker behind stderr/journald; an application-owned queue or logging framework is
not needed. Process exit or stderr I/O failure still cannot guarantee delivery.

The pinned Smithy runtime API 1.19.1 exposes
[Intercept](https://docs.rs/aws-smithy-runtime-api/1.19.1/aws_smithy_runtime_api/client/interceptors/trait.Intercept.html)
hooks before execution/transmit and execution-local ConfigBag storage. Count
transport attempts there, including retries, without reading URLs, headers or
bodies. Presigning also runs the before-transmit hook: exclude query signing via
the public `aws-runtime` `SigV4OperationSigningConfig`/`HttpSignatureType`, not S3's
private `PresigningMarker`. Sent SDK operations use header signing. These counts
include transport failures before wire transmission. GET `.send()` ends at headers and is not a network-only timer;
observe streamed body consumption separately. Retain SDK signing, retry and
checksum semantics. The performance issues already reviewed below do not justify
replacing the transport merely to obtain these observations.

The [Linux 7.3 assessment](fuse-io-uring-assessment.md) compares versioned kernel
UAPI, installed fuser 0.18.0, the unmerged Skory transport stack, Apache-2.0
fractal-fuse/fuser-iouring and released versus newer libfuse. Keep the current
dependency: io_uring has credible small-core test configurations, but no candidate
has passed this filesystem's contracts or measured joint resource gate. Prefer
an existing supported transport over local protocol/unsafe code. A few workers
do not by themselves shrink possible-CPU queues or per-entry payload allocation.
The operator excludes forks of shared libraries and prefers released upstream
versions even at a modest performance cost (2026-11-01). The Skory fuser stack is
therefore rejected; unreleased upstream fuse-backend-rs `fusedev-uring` is watched,
not adopted. Use supported fuser 0.18.0 `n_threads`/`clone_fd` for transport work.

The [classic dispatch diagnosis](performance-classic-dispatch.md) first measures
queue/service intervals, off-CPU wakes and CPU stacks. Extra fuser receive loops
retain a separate `16 MiB + 4 KiB` buffer each and reduce four-loop random rates
in the measured pairs. Keep one loop. A cache-hit read experiment evaluated existing futures-util
0.3.34 [`FutureExt::now_or_never`](https://docs.rs/futures-util/0.3.34/futures_util/future/trait.FutureExt.html#method.now_or_never)
on a pinned mutable future and Tokio 1.54.1
[`Handle::enter`](https://docs.rs/tokio/1.54.1/tokio/runtime/struct.Handle.html#method.enter).
Polling a pinned borrow preserves a pending future for the same bounded Tokio
fallback, including admission, timeout, cancellation and reply ownership. It
does not add a cache implementation or duplicate generation/TTL logic. Despite
improving cached-read latency and CPU, the trial's joint result is inconclusive
under the [revised statistical protocol](performance-methodology.md). Its runtime/
test changes and receive-loop CLI remain absent; retain removal of redundant
post-rename notifications. No dependency
change, custom cache or executor is adopted.

The [metadata expiry diagnosis](metadata-expiry-diagnosis.md) uses standalone
[TShark 4.6.4](https://www.wireshark.org/docs/man-pages/tshark.html) for TCP/HTTP
reassembly instead of implementing a packet decoder. Public distro packages are
extracted locally with hashes; nothing is linked into or distributed with the
runtime. Reinspection of the locked SDK 1.155.0 confirms generated orchestration
and plugin assembly are crate-private, not a supported prepared-operation API.
SDK issue [#1349](https://github.com/awslabs/aws-sdk-rust/issues/1349) does not deliver
a metadata setup fix. Keep the selected SDK and integrity/metadata contracts;
sampled SDK work alone does not justify a private fork or replacement transport.

The suspended [cached-observation trial](performance-cached-observations.md) uses
Rust 1.99's supported [`Arc`/`Weak`](https://doc.rust-lang.org/1.99.0/std/sync/struct.Arc.html)
ownership APIs rather than a custom metadata cache or SDK replacement. Strong
references pin namespace eviction victims; weak identity plus a bounded original
observation avoids that contract change. Correctness passes, but the short paired
joint inference is unresolved; neither trial representation nor dependency change is retained.

Inode lifetime, namespace bindings, dirty generations, flush ownership, sparse
overlays and fsync barriers are filesystem state, not replacement infrastructure.
Existing Tokio permits/watch/JoinSet bound work, coalesce overlapping fills and
cancel/drain jobs. Disjoint windows remain concurrent. Required and optional
admission differ: optional refill skips pressure, required FIFO must progress.
Contiguous consumption hints protect foreign unread cache backing only during
optional eviction; they never pin required admission. Reset adaptive Pattern on
zero-to-one legacy open without discarding payload or active-reader history;
FUSE handles now own independent history.

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
- For an upload range already covered by one charged dirty extent, use bytes
  1.12.1's supported `Bytes::from_owner` and Smithy 1.9.0's known-bytes body rather
  than materializing another streaming frame. Keep the original block and upload
  activity alive through every byte/body clone; retain the 1 MiB frame bound.
  AWS SDK S3 1.155.0's checksum interceptor then signs a checksum header instead
  of selecting aws-chunked trailers. Sparse/composed/remote ranges still stream;
  checksum policy, retries, signing and post-commit HEAD remain library-owned.
  [SDK #1349](https://github.com/awslabs/aws-sdk-rust/issues/1349) is a broader
  performance report, not proof of a local transport bug; Bedrock/custom-client
  [#1287](https://github.com/awslabs/aws-sdk-rust/issues/1287) does not justify
  disabling S3 stalled-stream protection. No transport fork or new crate is needed.

## Investigation boundary

The writer trace contains two independent absence resolutions (kernel LOOKUP,
then create validation) and mandatory post-commit HEAD. Linux 7.0
[`fuse_atomic_open`](https://github.com/torvalds/linux/blob/v7.0/fs/fuse/dir.c)
performs LOOKUP before FUSE_CREATE; fuser 0.18.0 does not merge those callbacks.
Removing validation would change remote-create race behavior, and retaining a
previous miss would change the default zero negative-cache policy. Keep both.
Within each missing-object resolution, marker HEAD and implicit-directory LIST
were tested with overlap using the existing Tokio 1.54.1
[`select!`](https://docs.rs/tokio/1.54.1/tokio/macro.select.html) and a spare existing
metadata-resolution permit. The experiment preserves marker metadata/error
priority, cancels irrelevant LISTs and falls back to serial progress when slots
are occupied. Native SDK/HTTP wiring and cancellation tests pass. Four rotated
writer pairs nevertheless lose throughput once, p95/p99 three times and daemon
RSS in every repetition, without removing any writer requests. Reject the runtime
change; the serial lookup implementation remains. See the
[experiment results](performance-lookup-probes.md). No transport replacement,
new dependency or weaker negative-cache policy follows from this investigation.

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

The subsequent TTL/PLUS investigation checks fuser 0.18.0's supported
`KernelConfig::add_capabilities`, `ReplyDirectoryPlus::add` and reply TTL APIs.
[#756](https://github.com/cberner/fuser/issues/756) explains the single-TTL limit
for PLUS/create; conservative remaining freshness needs no fork. Public Linux
`fs/fuse/readdir.c` and native tests establish dot-entry/lookup ownership.
Mountpoint [#1801](https://github.com/awslabs/mountpoint-s3/issues/1801) and
[#953](https://github.com/awslabs/mountpoint-s3/issues/953) provide metadata/TTL
analogues, not an SDK correction. Per-open history follows the pinned public Go
`FileHandle` contract. Existing Tokio owned permits bound file handles; existing
rustix 1.1.5 `fs` APIs exercise native xattr/fallocate behavior without local unsafe.
jemalloc-ctl 0.7.0 exposes safe optional page gauges; no allocator tuning or
dependency replacement is justified by these observations.

The frozen candidate's native request trace confirms repeated standard LIST
prefix scans for bounded directory chunks (10–11 requests for 1,024 names).
AWS SDK 1.155.0 exposes `ListObjects::marker` and
[`ListObjectsV2::start_after`](https://docs.rs/aws-sdk-s3/1.155.0/aws_sdk_s3/operation/list_objects_v2/builders/struct.ListObjectsV2FluentBuilder.html#method.start_after).
A narrow exclusive-name adapter can skip consumed pages while retaining opaque
continuation tokens. Extended LIST retains its existing protocol until equivalent
start-after semantics are verified; no custom parser, request cache, larger
directory buffer or weakened type/metadata resolution is needed.

The warm directory profile also exposes repeated application page processing.
Reuse the existing Moka 0.12.16 page cache and Rust slice `partition_point` to skip
the consumed prefix; validate the remaining inode bindings, types and TTL exactly
as before. No new cache, dependency, kernel buffer or ownership pin is needed.
The alternative supported fuser 0.18.0 `FUSE_READDIRPLUS_AUTO` capability reduces
unused name-only lookup owners in a real mounted test, but four rotated metadata
pairs lose throughput in three cases and pooled p99 in all four. Retain the
existing forced-PLUS policy rather than shipping that unproven speed change.

Standard S3 LIST object fields supply size/ETag/modified time, not arbitrary user
metadata. Preserve the cached or HEAD observation time when merging selected
entries; only actual authoritative LIST metadata has the new LIST observation
time. Existing SDK responses, Moka and bounded futures suffice. Avoid another
TTL extension or cache-policy change to hide required metadata refresh requests.

For extended LIST error-body failures, retain the existing reqwest 0.12.28
streaming [`Response::chunk`](https://docs.rs/reqwest/0.12.28/reqwest/struct.Response.html#method.chunk)
and separate [`Response::status`](https://docs.rs/reqwest/0.12.28/reqwest/struct.Response.html#method.status)
APIs. Automatic gzip decoding can fail after response headers have established a
transient HTTP status. Classify that status in the existing bounded retry loop
instead of replacing transport, disabling gzip or adding a second retry layer.
Keep [`Error::without_url`](https://docs.rs/reqwest/0.12.28/reqwest/struct.Error.html#method.without_url)
for terminal body failures so presigned URLs remain excluded from error text.

For stderr journal severity, reuse the existing rustix 1.1.5 safe
[`fstat`](https://docs.rs/rustix/1.1.5/rustix/fs/fn.fstat.html) and standard-library
environment/formatting APIs with systemd's documented `JOURNAL_STREAM` device/inode
identity and `SyslogLevelPrefix` line framing. The sender only needs to preserve
the severity of its existing JSON records on a verified journal stream. Evaluated
[tracing-journald 0.3.2](https://docs.rs/tracing-journald/0.3.2/tracing_journald/)
(MIT, maintained Tokio tracing project) supplies native structured logging through
a subscriber layer, rather than this bounded JSON stderr worker.
[sd-notify 0.5.0](https://docs.rs/sd-notify/0.5.0/sd_notify/) (MIT/Apache-2.0)
provides service notifications, not journal logging. Neither is needed for a
small framing adapter; retain tracing-appender's bounded non-blocking ownership.
The public systemd 255 rate limiter has separate severity pools; no rate-limit
change or native journal protocol implementation is required.
For application-side saturation, reuse a second tracing-appender 0.2.5
`NonBlockingBuilder` worker with a fixed 16-record reservation for debug-mode
error/warning output. Its supported lossy queue and `ErrorCounter` avoid a custom
priority channel, async-runtime backpressure or an unbounded log buffer. Keep the
existing ordinary-mode worker and independent drop visibility.
