# Dependency decisions

## Public provenance and selection

The reference checkout was verified clean at
`5c0884c9e8799bbab93e2d4102fdc15e9c079e0d`, public upstream
<https://github.com/yandex-cloud/geesefs> (Apache-2.0). Its contracts inform
independent Rust implementation; no Go SDK implementation is vendored.
The packaged `80c214ef14f620d93407454fc76ca01dd71fa25f` baseline was also built
from its verified public source object and hashed. Both incomplete benchmark
attempts stopped before reaching its mounted comparisons; see [performance](performance.md).

Before implementation, `cargo info` verified these releases and licenses from
the public crates.io registry (2026-10-26). APIs are linked below. All Cargo
dependencies must resolve from crates.io; no internal source, registry, generated
SDK, Arcadia code, or private dependency is permitted.

| Component | Selected version / license | API and decision |
| --- | --- | --- |
| FUSE | fuser 0.18.0 / MIT | <https://docs.rs/fuser/0.18.0/fuser/>; owns protocol, callbacks, owned replies; integrated for actual foreground read/write mounts. Mountpoint's namespace/write limitations and oxfs's whole-file buffering/object layout do not meet this plan. |
| S3 | aws-sdk-s3 1.155.0 / Apache-2.0 | <https://docs.rs/aws-sdk-s3/1.155.0/aws_sdk_s3/>; one client, conditional Range, streaming body, multipart upload/copy/completion/abort. Disable unneeded sigv4a; retain SDK TLS/signing/retry. |
| Credentials | aws-config 1.13.0 / Apache-2.0 | <https://docs.rs/aws-config/1.13.0/aws_config/>; supported provider chain/profile, not custom credential parsing. Metadata-token IAM uses the separate public-trait adapter described below, not AWS IMDS equivalence. |
| Runtime / admission | Tokio 1.54.1 / MIT | <https://docs.rs/tokio/1.54.1/tokio/sync/struct.Semaphore.html>; runtime, owned permits, async synchronization. No custom executor. |
| XML | quick-xml 0.42.0 / MIT | <https://docs.rs/quick-xml/0.42.0/quick_xml/>; bounded ext-v1 schema adapter. Standard SDK object-list models drop per-object Metadata. XML syntax belongs to this parser. |
| Retained cache | Moka 0.12.16 / MIT OR Apache-2.0, plus Apache-2.0 | <https://docs.rs/moka/0.12.16/moka/sync/struct.Cache.html>; weighted eviction, not hard admission. Allocation permits must survive eviction while pinned. |
| CLI | clap 4 / MIT OR Apache-2.0 | <https://docs.rs/clap/4/clap/>; typed parsing and errors, unknown flags rejected. Small fstab argument normalization follows reference. |
| Metadata escaping | percent-encoding 2 / MIT OR Apache-2.0 | <https://docs.rs/percent-encoding/2/percent_encoding/>; decode bytes; malformed escapes need explicit validation because the crate deliberately tolerates them. |
| Errors | thiserror 2 / MIT OR Apache-2.0 | <https://docs.rs/thiserror/2/thiserror/>; typed actionable failures. |
| Fixtures | tiny_http 0.12.0 / MIT OR Apache-2.0 | <https://docs.rs/tiny_http/0.12.0/tiny_http/>; cargo info verified public origin. Reuse HTTP server at SDK transport boundary; no live credentials or uncontrolled network. |
| Timestamps | aws-smithy-types 1.9.0 / Apache-2.0 | <https://docs.rs/aws-smithy-types/1.9.0/aws_smithy_types/date_time/>; reuse SDK timestamp parser for ext-v1 LastModified; already an SDK dependency. |

## Narrow custom state

The operator reaffirmed maximum reuse of maintained library mechanisms during
the read-buffer continuation. The miss-reply optimization reuses the existing
immutable `Block` view and Tokio-owned reservation lifetime; it adds no pool,
allocator, stream protocol or cache implementation. AWS Smithy ByteStream 1.9.0
already exposes `try_next() -> Result<Option<Bytes>, Error>` and
`into_async_read() -> impl AsyncBufRead`; the current body adapter uses the latter.
Direct transport `Bytes` adoption must account for a slice's complete retained
allocation, not just its visible length, and preserve whole-body checksum/EOF
barriers. That is a separate verified adapter change, not a reason to replace
the maintained SDK or introduce a second transport.

For the subsequent full-overwrite fragment path, Tokio 1.54.1 already supplies
[`copy_buf`](https://docs.rs/tokio/1.54.1/tokio/io/fn.copy_buf.html),
[`AsyncReadExt::take`](https://docs.rs/tokio/1.54.1/tokio/io/trait.AsyncReadExt.html#method.take)
and `AsyncWrite for Vec<u8>`. The SDK's `into_async_read` implements AsyncBufRead.
Bound `copy_buf` with the admitted fragment length and preallocate a Vec of that
capacity: initialized bytes extend directly from the SDK reader's existing
buffer, without a second intermediate buffer or pre-zeroing the destination.
Check the exact copied count before transferring the Vec into the existing
charged Block. The standard/library implementations own buffer initialization;
no local unsafe, manual polling, pool, BufMut implementation or new dependency
is needed. The existing final EOF and whole-body checksum barriers remain.

The package forbids unsafe code for every target. rustix 1.1.5 (Apache-2.0 WITH
LLVM-exception OR Apache-2.0 OR MIT,
<https://docs.rs/rustix/1.1.5/rustix/process/>) provides safe uid/gid operations;
it replaces direct libc calls. Public `cargo info` verified origin/license before
integration. tikv-jemallocator 0.7.0 and tikv-jemalloc-ctl 0.7.0 (MIT OR
Apache-2.0, <https://github.com/tikv/jemallocator>) provide the allocator and safe
statistics APIs; these replace direct unsafe mallinfo2 inspection. Use this same
allocator in ordinary and benchmark Rust builds, report its identity, and measure
RSS separately: jemalloc allocated bytes do not include allocations made by other
native libraries or kernel/transport buffers. No custom allocator/FFI is written.

Continuation implementation uses fuser 0.18.0 directly (default pure-Rust mount
support), bytes 1.12.1 (MIT, `Bytes::from_owner` retains byte permits without a
copy), libc (MIT/Apache-2.0, Linux errno/mode constants), and reqwest 0.12
(MIT/Apache-2.0, rustls/gzip) for the extension alone. The extension URL is
presigned by the *same AWS SDK client* after supported request customization;
no signing implementation or second credential chain is introduced. Standard
object requests remain SDK-owned. Tokio owns scheduling. Short per-inode locks
protect application generations; per-inode flush gates serialize remote commits.
async-trait 0.1 (MIT/Apache-2.0, <https://docs.rs/async-trait/>) provides the
object-store async testing boundary; it does not replace runtime or SDK behavior.
unicode-general-category 1.1.0 (Apache-2.0, public cargo info verified,
<https://docs.rs/unicode-general-category/1.1.0/>) supplies Unicode character
categories for the reference's IsPrint metadata escaping rather than a new table.

No existing crate establishes GeeseFS's inode lifetime, dirty generation and fsync
contract. Those are application state, not replacement infrastructure. Charge
allocation owners rather than slices; never use an Arc/Bytes subslice to conceal
the retained parent allocation. Reserve flush capacity independently from dirty
admission. Remote completion, not individual part acknowledgments, establishes
durability. Standard S3 operations remain SDK-owned. An ext-v1 network adapter
must preserve the same credential source, endpoint, retry and cancellation
policy; parser-only support is not backend compatibility.

## Audit status

Directory-page reuse uses the already selected Moka 0.12.16 synchronous weighted
cache, not a new cache implementation. Its capacity counts returned names against
the configured entry limit; pages also check the existing namespace mutation epoch,
inode retention and stat TTL before reuse. Only bounded returned pages are retained,
not a full remote tree. This addresses repeated warm READDIR network calls without
changing freshness, reducing the workload or adding a dependency.

Extended-metadata lookup preloading reuses the existing bounded `Store::list`,
Moka cache for per-directory attempt timestamps and Tokio's per-inode mutex.
One first page can install authoritative immediate file metadata into the existing
entry-limited namespace. No listing graph, negative-range index or second inode
cache is introduced. Unknown/standard metadata and paths outside the first page
retain the ordinary HEAD/directory probes; speculative failure is not a negative
lookup. This follows the public Go reference's batch-lookup approach with a smaller
scope and preserves the package's public-source/unsafe boundaries.

Benchmark CPU counters use public `psutil` 7.2.2 (BSD-3-Clause,
https://github.com/giampaolo/psutil) for process identity and user/system time,
rather than a new `/proc/PID/stat` parser. The existing Linux cgroup adapter reads
`cpu.stat` for complete owned-workload usage and throttling, which process CPU
counters cannot provide. Standard monotonic clocks define the measurement window;
no sampled percent-CPU estimate or sum of parent/child cumulative CPU is substituted
for counter deltas. This Python dependency is test/benchmark-only.

The immutable Kubernetes diagnostic reuses the native, maintained SeaweedFS
4.51 S3 server (Apache-2.0, public
https://github.com/seaweedfs/seaweedfs/tree/153ab668c99731a6d29492bf6c61d0b4fa701101).
Its upstream `weed mini` command owns S3, conditional/range reads, storage and
identity permissions. The Linux amd64 release archive is pinned to SHA-256
`c03be37416a8ee672c77766e20b22a4e1430744c3af2668599a4b6eeaec14cee`.
No new S3 protocol implementation or production storage deployment is introduced.
Existing Moto 5.1.17 remains the disposable fault-injection fixture; a separately
accounted native backend avoids conflating its Python service with filesystem
daemon CPU in the multi-gigabyte diagnostic. The controller uses public boto3
for seeding, full-byte checks and read-only permission probes. It removes the
seed identity and restarts the persistent disposable server before measurement;
both filesystems use the same read-only identity and standard S3 listing. This
fixture does not establish actual-service extended-metadata compatibility or
performance. Its backend CPU/cache scope is reported separately from clients.

Cargo.lock is reviewed for public registry origins and CI for public actions.
`cargo metadata --locked --format-version 1` reported 336 packages (including
this root package), with no non-public source: every dependency resolves from
the public crates.io index. All packages declare a license or license file; the declarations
include permissive alternatives and Unicode data licenses. This is a source/
declaration audit, not a completed binary-distribution notice bundle.
There is no vendored/generated/internal code. License for this repository remains
in LICENSE. Each dependency's license is retained in its public crate package;
redistribution of binary distributions must include the corresponding notices.
The complete transitive license/binary distribution audit remains outstanding.

## Backend continuation: public adapters (2026-10-26)

The public Go contracts in `core/backend_s3.go`, `core/cfg/conf_s3.go` and
`core/ycs3ext` require metadata-service IAM tokens, SSE request headers and the
PATCH operation absent from the standard S3 model. AWS SDK 1.155.0 exposes
`push_auth_scheme`/`auth_scheme_resolver`, asynchronous identity resolution and
header-only token authentication: use that supported boundary, not a second S3
client or handwritten cryptographic signing. `aws-smithy-runtime-api` 1.19.1
(Apache-2.0) supplies those public traits. Reqwest 0.12 owns metadata HTTP/TLS,
bounded timeouts and redirect policy; serde_json 1 (MIT/Apache-2.0) owns JSON.
The narrow token schema/cache is application state; there is no AWS credential
provider for the reference's GCP/custom IAM-header protocol.

Existing public SDK dependencies base64 0.22 (MIT/Apache-2.0) and md-5 0.10
(MIT/Apache-2.0, RustCrypto) supply SSE-C encoding and its protocol-mandated MD5
digest; do not implement either primitive. Humantime 2 (MIT/Apache-2.0) supplies
duration parsing; mime_guess 2 (MIT) was initially selected for filename guesses,
then rejected for the specific policy mismatch recorded below. Primary APIs: <https://docs.rs/aws-sdk-s3/1.155.0/aws_sdk_s3/config/struct.Builder.html>,
<https://docs.rs/aws-smithy-runtime-api/1.19.1/>, <https://docs.rs/base64/0.22/>,
<https://docs.rs/md-5/0.10/>, <https://docs.rs/humantime/2/>,
<https://docs.rs/mime_guess/2/>. AWS runtime's public payload override preserves
SDK-owned SigV4 when optional payload checksums are disabled.

Registry research found maintained `s3s-sigv2` 0.17.0 (Apache-2.0,
<https://docs.rs/s3s-sigv2/0.17.0/>, <https://github.com/Nugine/s3s>). Reuse its
canonicalization and HMAC signing through the SDK auth-scheme override; do not
translate Go's cryptographic signer. Both available 0.16/0.17 releases require
Rust 1.96, so this continuation raises the declared MSRV to 1.96 rather than
vendoring or reimplementing signing. SDK no-auth support supplies anonymous
requests. The compatibility adapter preserves the public Go subresource set and
encoded query values while SDK endpoint resolution owns path/virtual-host style.
`tempfile` 3.27.0 (MIT/Apache-2.0, <https://docs.rs/tempfile/3.27.0/>) owns isolated
profile fixtures; Tokio's process support provides finite, kill-on-drop test
children. AWS `credentials-process` is deliberately disabled: its current SDK
implementation spawns a shell without a process deadline or kill-on-drop, which
cannot meet the updated noninteractive/finite-subprocess requirement.

For `--no-verify-ssl`, the stable Smithy HTTP TLS builder exposes trust stores,
not a verification-disable switch. Evaluated public adapters:
`aws-smithy-http-client-reqwest` 0.1.0 (Apache-2.0) collects every response with
`resp.bytes()` and violates streaming ownership; `smithy-transport-reqwest` 0.1.0
(MIT/Apache-2.0) streams correctly but has no supported custom-client/TLS builder.
Use a narrow public-trait adapter around the already selected reqwest 0.12 only
for this explicit option: HTTP/TLS, bodies and verification policy remain crate
owned; no HTTP or TLS protocol is implemented. Preserve streaming response bodies,
disable redirects/decompression and retain finite request/body deadlines.
`http` 1 (MIT/Apache-2.0) supplies typed DTO conversions. TLS tests reuse tiny_http's
existing rustls feature and public `rcgen` 0.14 (MIT/Apache-2.0,
<https://docs.rs/rcgen/0.14.8/>) for disposable localhost certificates. Default
verification remains enabled; IAM metadata TLS is not weakened by the S3 option.

MIME policy correction: `mime_guess` was evaluated but its larger built-in table
and lack of Unix database precedence do not implement public Go's
`mime.TypeByExtension` contract (notably JavaScript/XML and local overrides).
Use `mime` 0.3.17's maintained syntax parser (MIT/Apache-2.0,
<https://docs.rs/mime/0.3.17/>, <https://github.com/hyperium/mime>) with a narrow Go compatibility/defaults
and Unix database adapter instead. The 16 default MIME associations derive from
the public Go standard library, copyright The Go Authors, BSD-3-Clause:
<https://go.dev/src/mime/type.go>, <https://go.dev/src/mime/type_unix.go>.
No Go parser implementation is copied; Rust adapters ingest bounded metadata
using standard readers and the MIME parser. Binary notice aggregation remains
the parent's final-distribution responsibility.

## Streaming configured upload parts (2026-10-26)

Re-evaluated public Smithy `SdkBody::retryable`/`from_body_1_x` and
`http-body` 1 (MIT, <https://docs.rs/http-body/1/>) before lifting the fixed
allocation-sized part limit. Reuse these APIs for retries and HTTP body framing.
`http-body-util::Full` buffers the whole part; `StreamBody` lacks an exact byte
hint and its async materialization future need not be Sync. Use a narrow safe
Body adapter with an exact remaining length and a mutex-protected pending future;
it materializes charged chunks, not whole configured parts. This does not
implement HTTP, checksums, transport or retry machinery. The SDK owns those.
Public Go part-size configuration is application scheduling data, not a protocol
implementation; its bounded parser and range arithmetic are independently written.

Streaming wire fixtures independently check SDK CRC32 trailers with maintained
`crc32fast` 1 (MIT/Apache-2.0, <https://docs.rs/crc32fast/1/>), already used by
the public compression dependency, rather than implementing checksum arithmetic.

## Concurrent directory metadata resolution

Reuse public `futures-util` 0.3.34 (MIT/Apache-2.0,
<https://docs.rs/futures-util/0.3.34/>, <https://github.com/rust-lang/futures-rs>),
already transitive in Cargo.lock, for FuturesUnordered/StreamExt. Unlike spawning
borrowed filesystem operations, these futures can borrow the existing lookup
path and cancel synchronously on drop. Acquire an existing Tokio semaphore's
global owned job permit before adding each future, not inside per-page spawned
tasks. This bounds admitted work across readdir calls and preserves the existing
lookup metadata/type/generation checks without custom scheduling infrastructure.

## Overlapping read-window coordination

Reuse Tokio's maintained watch channel
(<https://docs.rs/tokio/1.54.1/tokio/sync/watch/>) to notify read followers when
an owned fill completes or is cancelled. Moka's identical-key get-with operation
does not coalesce overlapping read-ahead ranges with different caller offsets.
The small per-inode, generation-scoped interval registry is filesystem state,
not a new generic single-flight framework. Its RAII owner removes the interval
and closes the channel on every exit; followers retry the normal cache check
under the existing request deadline. Disjoint windows still run concurrently.
Publish completed blocks into the current cache policy, not a clone detached
by a concurrent pressure eviction. Do not change payload/cache budgets.

## Charged cached-read views

Evaluated the already selected public `bytes::Bytes::slice` API before changing
cache-hit reply ownership. `Block` also provides exclusive mutable fill access
and carries allocation and optional outer-owner reservations; its live charge
must describe the complete backing allocation, even for a small immutable view.
Replacing that mutable owner with Bytes would introduce a second mutable/shared
representation or a separate owner adapter. Use a narrow `Arc<Allocation>` plus
standard-library `Range<usize>` view inside the existing charged Block. A public
read regression first observed the redundant 4 KiB reply allocation; the view
retains the complete backing charge through eviction and last-owner drop.
This reuses the
existing allocation, semaphore ownership, Moka policy and fuser borrowed reply;
it is not a new allocator, cache or FUSE implementation. Measurement is required
before claiming that removing this copy resolves the observed CPU/tail deficit.

## Requested-range publication with owned read-ahead

Reuse the existing Tokio JoinSet, watch notification and request semaphore for
read-window scheduling, cancellation and admission. A one-shot Rust callback can
consume the existing fuser reply when its requested bytes are complete; the
already admitted FUSE task continues owning the remaining window and reports its
error. No extra spawned job or per-read channel is needed. Tests alone use
Tokio oneshot channels to observe the callback while independently gating backend
ranges. The ordinary awaitable read retains its full-window completion/error
contract. This is application-level reply timing, not a new runtime, queue or
FUSE protocol implementation.

## Incremental conditional-GET publication

The public AWS SDK's ByteStream and Tokio AsyncRead adapter already own body
framing, errors and cancellation. Reuse them for bounded 128 KiB completed body
buffers, with a bounded Tokio mpsc channel to the existing window owner. The
configured conditional GET ranges and request concurrency remain unchanged;
do not add a small duplicate GET or implement an HTTP/S3 protocol. A shared Arc
reservation must retain the complete admitted GET charge through producer,
queued fragments, cache and last borrower. The existing charged Block owns each
immutable completed buffer. Retain complete-body SDK validation before exposing
any prefix when the response advertises a whole-body checksum, rather than
publishing bytes before that integrity check can finish. Existing whole-range
reads and writer materialization keep their full validation contract.
