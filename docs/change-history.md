# Change history and reasons

Concise engineering context for the experimental Linux/S3 rewrite. Compatibility
and comparative performance remain **UNACCEPTED**. Detailed journals, failures,
source/binary manifests and pre-cleanup documents stay in ignored evidence;
measurement pages retain their original tables and source scopes.

## 2026-11-13: preserve live xattrs and release avoidable metadata backing

A verified fsync replaced live xattrs with the remote wire projection, losing
uppercase names and non-UTF-8 bytes. Keep live metadata independently of the
normalized remote baseline. Six public filesystem regressions first fail on the
previous source and then preserve the same inode's values, durable bytes and
subsequent writes. The existing wire codec and reopen normalization are unchanged.

Move untouched inodes' writer-only fields into lazily admitted backing and share
one compact budget handle across metadata charges. Admit backing before mutation
or remote side effects. An independently reproduced rename/LIST race additionally
requires protecting moving identities from clean metadata replacement and funding
writer backing before publishing retry intent.

Use Moka's supported per-entry expiry from the page's original fetched time and
drive maintenance before listing admission. Already-expired pages are not cached;
an active response retains its own ownership. Removing only cache expiry makes
the positive-TTL regression retain two backings instead of one. See the
[verification scope](testing.md#metadata-backing-and-live-xattrs--2026-11-13).
These corrections do not establish a whole-reader memory bound or resolve the
strict native flat-directory progress gate.

## 2026-11-13: retry interrupted extended LIST transport within its existing budget

Target traversal retained failures where extended LIST request errors exhausted
only one attempt, alongside independent LIST/HEAD deadline failures. Real loopback
TCP regressions reproduce two missing retry cases: closure before headers and a
successful response ending before its declared Content-Length. Both are assertion-RED
on the previous source. Even complete XML from an incomplete HTTP message must be
discarded, not published as a successful page.

Classify supported reqwest/hyper error sources and retry the same signed page within
the existing attempt count, backoff and aggregate deadlines. Hyper represents a
truncated Content-Length body as an UnexpectedEof I/O source; gzip decoder EOF must
remain terminal when the HTTP message itself completed. Five public-backend tests
cover recovery, exact page/token preservation, configured exhaustion, deadline
cancellation and permanent decoding failures. Full default/all-feature suites each
pass 535 active tests. See [verification](testing.md#interrupted-extended-list-transport--2026-11-13).
This does not resolve every target timeout or the strict flat-directory capacity
gate, and does not establish completed physical L2 reuse or acceptance.

## 2026-11-13: correct native directory fixtures for flat metadata reuse

Three preserved native failures assumed the older listing path: the request count
omitted the initial flat probe, a child-LIST failure was no longer reached when a
complete flat page established the directory, and the marker check demanded a HEAD
despite authoritative flat metadata. Correct those fixtures without changing the
runtime. Force real pagination before the injected failure, check EIO and zero
escaped references, and verify marker metadata through both flat and common-prefix
paths. The name/size/ownership contracts remain intact. Relevant temporary faults
each fail the corresponding corrected oracle and are reverted.

The native suite now has 18 passes and the same strict flat ENOMEM failure. Its
8388608-byte requirement is unchanged; target traversal and physical recycling
remain open. See [verification](testing.md#directory-listing-fixture-contracts--2026-11-13).

## 2026-11-12: tie directory-open lifetime to the admitted owner

Native teardown intermittently retained directory opens after unmount. A real
fuser-protocol regression deterministically opens 32 directories, omits individual
RELEASEDIR requests, then observes 32 opens after DESTROY instead of zero. Each
directory handle now pins its inode and releases its open count when the actual
cursor owner drops. Shutdown drains the registry after admitted requests finish;
normal release no longer decrements before an in-flight cursor relinquishes it.
The protocol regression and native handle controls pass with one/three receivers.

Stable CI additionally reports `needless_bool` on the preload validation helper;
use the equivalent positive predicate rather than suppressing the lint. This
correction leaves the strict flat traversal and three baseline PLUS gaps open.

## 2026-11-12: yield optional preload and reuse finite reclamation progress

A full namespace exposed two independent progress defects: speculative preload
finished a complete synchronous scan under the namespace mutex, and consecutive
admissions repeatedly scanned the same long kernel-owned prefix. Public lookup
regressions first fail on the original source. Preload now uses bounded yielding
admission without retaining the namespace mutex, then revalidates the publication
epoch and binding. A mutation during that yield must preserve local dirty bytes
and obtain the unrelated target's current metadata rather than publish the old LIST.

Keep a shared last-successful-victim hint, not a shared in-progress scan. Each
admission retains its own initial high-water mark and wraps once, so a released
prefix remains reachable and interleaved all-pinned scans still terminate. Count
pressure alone retains cached listing backings; invalidate them only on actual
byte-charge pressure. Existing metadata-reuse, no-HEAD and writer controls remain.

This fixes the isolated stalls, not the strict flat-directory capacity gate.
Adaptive READDIRPLUS and a larger notification quantum both failed that oracle
and were reverted. The latest opt-in suite additionally exposes a directory-open
count retained after unmount; preserve its failure rather than retry it away.
See [verification](testing.md#yielding-preload-and-reclaim-continuation--2026-11-12).
Full target traversal, physical L2 reuse and acceptance remain outstanding.

## 2026-11-12: preserve active directory owners and bounded traversal semantics

A small native inode bound exposes successful but truncated `du`: an active
remainder retained inode numbers without owning the pending inodes. Pin only the
active remainder, including cache hits; keep TTL pages numeric. Kernel-owned
entries survive TTL expiry, so retire clean unused dentries through supported,
detached bounded notifications and keep lookup counts until actual `FORGET`.
Exhausted cached remainders advance the backend cursor without publishing EOF.
An independent open-file fixture catches the false-EOF fault as successful
523264-byte traversal instead of the required 1572864 bytes.

A separate native fixture reproduces EMFILE from capping idle directory handles
at four times request concurrency. Share the existing entry-bounded owned handle
semaphore with files, preserving byte charges and actual cursor lifetimes.
No runtime budget, TTL, metadata or durability requirement is weakened.

The nested walk and within-budget pressure cursor controls pass. The strict
over-capacity flat traversal remains a failing progress gate; speculative pending
owner/batch changes did not fix it and were reverted. Keep its exact-size assertion
and the three independently reproduced native baseline failures visible in
[verification](testing.md#directory-ownership-and-progress--2026-11-12).
This is a local correctness correction, not full target `du`, L2 recycling,
memory/performance acceptance or traffic readmission.

## 2026-11-12: expose data-region fullness and broaden mirror selection

Configured file capacity, index counts and cumulative I/O did not show how much
of the L2 data-region pool remained reusable. Connect the pinned engine's existing
four state gauges and actual partition geometry to the Prometheus endpoint through
its supported metrics/picker APIs. Report used/free data-region capacity separately
from file allocation, indexed records and useful payload bytes. Empty preallocated,
populated, recovered and actively recycling real-file fixtures verify the boundary;
scraping performs no cache I/O. No engine format, compression or budget changes.

The prior deb/rpm-only operating example excludes ISO images and other mirror
payloads. Add a checked-in, CLI-tested case-insensitive policy for images, archives
and packages, retaining metadata/sidecar exclusions. The generic regex remains
explicit and matches mount-relative paths. See [operation and selection](disk-cache-operations.md).
Apply an operating-policy change as a separately recorded restart phase; local
tests and these observations do not complete the ongoing full-capacity/du trial.

## 2026-11-12: backpressure at saturated FUSE request admission

Real buffered-read and sendfile regressions reproduce EIO from immediate
`admission_full` EAGAIN while a previous READ has replied but still owns gated
read-ahead. Wait on the existing semaphore in fixed fuser receiver threads before
spawning; keep pending WRITE bytes funded and actual jobs responsible for their
permits. One absolute deadline includes waiting, with an explicit expiry check
before starting a job. No runtime limit or durability contract is relaxed.

Native byte/ownership controls and protocol tests cover total-deadline expiry,
already-expired writes, fsync failure/recovery and draining shutdown. Deliberate
early permit release, deadline renewal and expiry-boundary faults fail their
assertions. The broader native directory suite retains three baseline-reproduced
READDIRPLUS failures on the development kernel. Full scopes and reproduction
commands are in [testing](testing.md#fuse-admission-backpressure--2026-11-12).
This correction neither rewrites the earlier HDD measurements nor establishes
sustained target acceptance or traffic readmission.

## 2026-11-11: record the withdrawn-reader HDD functional trial

The operator accepts single-user downloaded-copy semantics without an extra KMS
authorization GET and occasional disk-cache slowdowns. A frozen `91bdc20` binary
allocates a 2-TiB HDD file, completes 127.997 GiB of direct-filesystem population,
and recovers 87,981 records on the same inventory after an ordinary restart.
Independent peer rsync/metadata and HTTP controls pass before and after restart;
range probes demonstrate actual recovered L2 delivery.

Preserve failed setup attempts and correct their causes: rejected live nginx
zone-key changes, a banned single replay generator, inappropriate full downloads
of historically aborted responses, and an immediate-Ready assertion that ignored
asynchronous recovery. No runtime budget/deadline or cache inventory was changed
to obtain these results.

The completed hour still contains seven truncated responses and eleven socket
timeouts, including five truncations through the changed common mount. Immediate
FUSE admission rejection is a concrete investigation target, not an exemption
because its source predates this PR. Keep reader RSS, allocator, cgroup and the
large consumer file-cache footprint separate in the
[target checkpoint](target-disk-cache-functional.md). Production acceptance
remains open; temporary test exceptions are removed and admission stays withdrawn.

## 2026-11-11: correct two independently falsified admission/progress defects

Five independent WOOF roles and a separate falsifier reviewed the full PR at
`8d91571dffb22ff2560b7cba21817ce2caa050d9`. Of 21 deduplicated candidates,
two P2 defects survived: a namespace cursor inherited from an earlier admission
could skip a newly released prefix inode and return false ENOMEM; repeated disk
timeouts could defer the first source GET of one producer range.

Make reclamation continuation local to each admission, start at the beginning,
and cap each batch plus the initial inode high-water mark. Stop remaining disk
attempts for the current range after timeout/saturation, without expiring healthy
lookups after source waits or releasing actual-worker funding early. Public
filesystem regressions fail on the original code. Independent post-fix review
also caught lost synchronous install progress after EAGAIN: retain its finite
local scan through completion, including speculative preload, while keeping
async publication bounded. Public tail-victim/exhaustion/released-prefix and
preload tests distinguish this path. Ten isolated faults are rejected.
Direct-tree default/all-features suites pass 518 tests each, with
24 opt-in entries. This does not attribute historical matrix failures to either
defect or close qualification. Target/recovery acceptance and KMS cached-copy
revocation semantics were explicit questions at this review checkpoint; the later
single-user decision above resolves only the latter scoped contract. Unrelated
optional recommendations and pre-existing traversal costs remain separate.

## 2026-11-11: overlap bounded source gaps and transfer decoded payload ownership

Gap discovery previously waited for each source miss run before inspecting later
disk hits. Use one capacity-one scalar descriptor channel and one source consumer
per existing range task; publish verified hits before waiting on a full queue.
Deduct its additional ownership from the existing window budget. Move the
validated decoded payload Box into its exact existing reservation instead of
copying it, retaining identity/CRC/generation and actual-worker lifetime checks.

Source-matched local qualification at `8d91571` completes 456 attempts with
449 byte/state passes and seven timeout-related zero-source failures. A separate
96/96 Rust-relative comparison shows scoped eviction throughput improvement;
CPU/memory and absolute L2/RAM regressions remain in the
[frozen checkpoint](performance-disk-cache-followup.md#combined-pipeline-checkpoint).
Neither measurement describes the later WOOF repairs or production acceptance.

## 2026-11-11: retain validated disk coverage in L1

A fixed warm-random workload repeatedly demoted 212 fragments of already resident
full disk records. Preserve the validated immutable record's actual backing and
offset when publishing into L1, without changing reply coverage, allocation or
charges. Partial records and EOF retain their real length; reclaimed entries can
still be refilled. Public filesystem regressions and independent fault controls
check unnecessary writes, offsets, invented coverage and refill after index removal.

Eight paired mounts plus eight A/A controls remove all 212 redundant admissions
and show a scoped warm-random throughput/CPU improvement. RSS controls are biased;
other workloads and historical timeouts remain separately qualified in the
[follow-up report](performance-disk-cache-followup.md#validated-backing-correction).

## 2026-11-11: preserve funded warm-L2 reads under contention

Four recovered readers could bypass valid disk records when read/spill slots or
namespace/inode try-locks contended. Required windows now wait on caller-funded,
key-free FIFO admission; read and spill owners use their independently funded
limits. Bounded blocking eligibility acquires namespace and inode guards
sequentially and revalidates names after the inode wait. Actual work retains
funding after timeout/cancellation. Each block lookup keeps its own deadline;
slow source gaps do not prematurely expire later warm lookups.

Proven-new media defers engine/file creation until eligible capacity pressure,
while existing inventory still recovers eagerly. Funding/lock ownership transfers
without release/reacquire. A separate maximum-record regression corrects the
raw-versus-page-rounded engine cap without relaxing decoded identity/payload
bounds. Reuse identical full/fragment keys instead of allocating them twice.

Deterministic contention, real parent rename and slow-prefix regressions fail
under independently reintroduced faults. Native large warm reads and capacity
reuse pass representative gates; repeated-mount performance acceptance is separate.

## 2026-11-11: qualify Range fixtures and fixed-work comparisons

Moto materialized the entire object before Range slicing, distorting large-read
preparation. Reuse the existing supported SeaweedFS fixture for measured S3 work;
retain Moto for functional fault injection. Verify byte ranges, conditional
requests, metadata and writer operations, and account provider costs separately.

Independent harness review exposed success after a final source-receipt error,
per-arm timed repetition and I/O counters extending into an untimed eviction
probe. Fail closed while retaining results; enforce identical declared operation
plans; measure probe I/O separately. Fixed-capacity preparation checks every byte
and actual region reuse. Original failed runs remain preserved independently of
the corrected protocol and subsequent results.

## 2026-11-10: demote eligible byte-pressure victims before loans

A real-file regression reading 48 MiB with a 32-MiB payload budget retained
10 MiB of idle flush loans but admitted zero L2 records: loan-first pressure
handling made every later victim ineligible for demotion. Prefer an eligible
nonloaned capacity victim before borrowing, rechecking identity outside the
cache lock and the selected candidate again under it. The same fixture now
persists 266 records and reads the first block after restart without a source GET.
No budget, queue or required-request wait was added. Disabled/excluded behavior
and genuine Prepared-to-Ready loan revocation remain separately tested.

This changes selected-data residency policy, not the total admission limit:
Ready selected victims may no longer use the idle flush lane. Native/paired
qualification remains necessary; bounded optional drops are still permitted.

## 2026-11-10: compact canonical metadata and shared keys

Controlled allocation probes confirmed two retained attribute containers per
clean inode and independently allocated namespace/state keys. Replace these with
shared immutable sorted attributes, a compact committed remote record and one
charged key backing. Writers use pre-admitted COW versions; snapshots/listing
records share owners without releasing admission early. Keep full transport
metadata and HTTP properties only in backend operations.

Removing LIST decode/encode/decode also preserves binary xattr bytes that the
roundtrip could substitute with UTF-8 replacement characters. Review regressions
ensure canonical projection after a successful PUT and a longer PATCH ETag use
prepaid operation credit, not fresh postcommit admission. Native directory-lock
checks also exposed inherited pre-exec descriptors retaining a failed preparation
lock; an owned safe unlock guard now covers that validation error path.

Matched default-jemalloc allocation experiments show lower clean metadata costs;
final exact-source mounted/paired performance verification remains separate. Keep
all adverse CPU samples and distinguish allocated memory, admission and RSS.

## 2026-11-10: bounded metadata admission and persistent payload tier

Add opt-in metadata byte admission independent of the existing payload limit.
Charges follow returned names/xattrs, listing/inode state, writer snapshots and
rename plans. Independent regressions exposed premature charge release, encoding
growth and move intent published before a mutating request; preserve old names
on pre-remote rejection/cancellation and retain intent after attempted operations.

Qualify the maintained Foyer fork and integrate capacity-only L1 demotion,
validated L2 reads/refill, bounded funding and owned recovery/shutdown. Correct
the demonstrated stale-size admission panic by checking size and generation
before splitting the new window. Real-file and native ext4 ENOSPC/EACCES tests
preserve completed inventory without reset. Full mounted/performance acceptance
remains open. Separate allocation evidence confirms duplicate clean metadata and
key owners still need compaction; a byte limit alone does not remove their cost.

## 2026-11-09: release 0.1.11 preparation

Advance the package and lockfile together for bounded subtree prefetch. Package
only the successful exact-source default-jemalloc CI build, including checksums,
build provenance and complete locked dependency notices. Retain earlier immutable
releases. Dependencies, timeouts and directory block accounting are unchanged.
Publication does not close compatibility or comparative performance acceptance.

## 2026-11-09: prefetch one flat page at the start of directory reads

Delimiter-only directory reads repeated LIST for each small descendant. Follow
the public Go 0.43.9 `DirHandle.loadListing`/`slurpOnce` rule: with a nonzero
metadata TTL, try one flat page at a directory boundary, starting at the mount
root with an exclusive key bound. Cache that bounded response in RAM for the
complete directories it covers. If the requested directory is incomplete, use
ordinary delimiter pages; continuation and zero-TTL reads do not start another
flat prefetch. A point lookup retains its existing authoritative HEAD/fallback
path and bounded optional parent preload.

Reuse Moka 0.12.16 with TTL and entry-weighted capacity; cached directory keys
share the response rather than copying its metadata. Conservatively charge a
whole response per directory index entry and cap the index to 1000 directories.
Local namespace changes invalidate speculative ranges. Preserve marker metadata,
unknown-metadata HEAD resolution, generation checks, deadlines and errno.
Synthetic traversal/request-count regressions first fail on the old code;
full compatibility and comparative speed/memory acceptance remain open.

Reference: [Go 0.43.9 directory listing, Apache-2.0](https://github.com/yandex-cloud/geesefs/blob/80c214ef14f620d93407454fc76ca01dd71fa25f/core/dir.go).

## 2026-11-08: consolidate merged corrections in release 0.1.10

Publish the merged extended-list resume and Go-compatible symlink-size corrections
from the main branch as a new immutable release. Advance only the package version
and its lockfile entry; dependencies and filesystem behavior are unchanged from
0.1.9. Retain existing releases and package the successful exact-source CI build
with checksums, build provenance and complete locked dependency notices.
Compatibility and comparative performance acceptance remain open.

## 2026-11-08: Go-compatible symlink size and release 0.1.9

Report the stored object-body size for symlinks, matching Go 0.43.9, instead of
substituting the decoded target length. Newly created metadata-only symlinks
therefore have size zero before and after fsync/reload. Existing symlink objects
with nonempty bodies retain their actual object size. Target metadata and readlink
are unchanged. Public filesystem regressions first reproduced both size overrides;
they cover HEAD, extended listings and durable creation/reload.

Advance the package and lockfile together for this correction and the extended
listing resume fix below. Preserve all previously published releases and assets;
package the exact-source successful default-jemalloc CI executable with checksums,
build provenance and complete locked dependency notices. These fixes do not close
compatibility or comparative performance acceptance.

## 2026-11-08: preserve exclusive resume bounds in extended listings

The ext-v1 adapter discarded the exclusive key bound supplied by bounded directory
reads. A later chunk could restart from the beginning and repeatedly fetch already
visited pages. Pass the confined, percent-encoded `start-after` through the existing
SDK signing path. An opaque continuation token still takes precedence. Standard
fallback, metadata, cache freshness and backend error handling remain unchanged.

The loopback wire regression first failed for the missing extended-list bound;
it covers mounted-prefix confinement and special characters alongside v1/v2.
Existing token-precedence and directory-page contracts remain applicable. This
corrects redundant listing work; it does not establish comparative performance
or explain every slow traversal.

## 2026-11-08: keep tag checks separate from snapshot publication

A version-tag push rebuilt an already published source commit and attempted to
publish its existing build snapshot again. The independent build had different
binary bytes, so the immutable-asset comparison correctly refused it. Publish
build snapshots only from branch pushes; tag pushes still run checks and builds.
Keep the existing checksum, provenance and binary comparisons for branch reruns,
and preserve all published releases. CI builds are not a reproducible-build claim.

## 2026-11-08: bounded throttling retries

Extended LIST exhausted four attempts with only 210 ms of backoff on HTTP 429.
Reduce the default to two retries, with Go-like jittered throttling pauses of
0.5–1 s and 1–2 s. Numeric Retry-After is a lower bound on the next pause. Keep
the SDK's native jitter/retry policy without a second retry loop. Bound each
LIST (including fallback), HEAD and GET-header operation to five seconds, or
the shorter configured HTTP timeout. GET payload transfer and writer deadlines
remain separate. Cache freshness and generation rules are unchanged.

Loopback tests cover throttling pauses, Retry-After recovery and exhaustion,
silent ext-v1/v1/v2 LIST, HEAD and GET headers, and existing configured retry
and access-error contracts. The retry-spacing regression first failed against
the previous implementation.

## 2026-11-08: release 0.1.7 preparation

Advance the package and lockfile together for self-contained severe backend
context. Preserve published v0.1.6 and its assets. Package only the successful
exact-source default-jemalloc CI executable with source/build provenance,
checksums and complete locked dependency notices. No external dependencies
change. Publication is separate from mounted verification, historical EIO
attribution and compatibility/performance acceptance.

## 2026-11-07: retain backend context in severe records

Terminal EIO records retained only a coarse backend class, requiring debug
history that can be dropped independently. Real fuser/HTTP regressions first
reproduce missing status/class/attempt context and missing GET-body attribution.
Capture final SDK and extension failures in five fixed request-local operation
slots; serialize their latest sanitized observation, count and monotonic age
directly with error/warning records. Recovered retries are not recorded as failed
executions. GET stream/validation failures retain a body class with unknown HTTP
status and attempt count. Preserve existing retries, errno, queues and filtering;
do not expose paths, URLs, headers, credentials or arbitrary remote error text.
This improves future attribution, not diagnosis of historical EIO or acceptance.

## 2026-11-07: release 0.1.6 preparation

Advance the package and lockfile together for transient LIST error-body retries
and protected severe diagnostic output. Keep published v0.1.5 immutable. Package
only the successful exact-source default-jemalloc CI executable with checksums,
source provenance and complete locked dependency notices. Receiver priority and
application reservation improve attribution, not durability; publication does
not close compatibility, memory or comparative performance acceptance.

## 2026-11-07: preserve diagnostic severity on journal stderr streams

Debug saturation also consumed the only application output queue, dropping a lone
error behind otherwise successful traffic. A blocked-stderr fuser regression
first observes that missing severe reply while FUSE continues. In debug mode,
reserve a separate 16-record tracing-appender worker for error/warning records;
retain the 256-record ordinary lane. Aggregate drops across both queues and expose
reserved-severe drops separately. Each lane is FIFO, but cross-lane output can
reorder; correlate timestamps and IDs rather than output position. Both lanes
remain bounded/lossy, and error bursts can still exhaust the reservation.

JSON severity alone does not set a journal stream record's `PRIORITY`. Without a
line prefix, errors and verbose debug consume the receiver's default severity
pool together. A real fuser/socket regression first fails for absent severity
framing. Match `JOURNAL_STREAM` against stderr's actual device/inode once, using
the existing safe rustix API, then prefix whole records with systemd's supported
error/warning/info/debug priority. Ordinary stderr, absent/malformed identities
and redirected descriptors retain plain JSON. Bounded lossy workers and drop
metrics remain; severity framing does not guarantee delivery or disable the
receiver's rate limits. No service restart or additional log volume is implied.

## 2026-11-07: retain transient LIST status after error-body failure

The extended LIST adapter read the response body before checking its HTTP status.
A malformed gzip error response therefore returned a transport/protocol error
immediately, skipping the configured retry for HTTP 429 and 5xx. A real loopback
HTTP regression reproduces that failure before the correction. Preserve the
transient status when its body cannot be read, discard incomplete XML and use the
existing bounded retry loop. Successful and non-transient response-body failures
remain errors; neither they nor transient failures select standard LIST fallback.
Tests cover recovery, exhausted and disabled retries, access denial and malformed
successful responses. This independently reproduced defect does not establish
the cause of historical filesystem errors or close performance acceptance.

## 2026-11-07: release 0.1.5 preparation

Advance the package and lockfile together for the explicit diagnostic levels and
bounded debug events below. Preserve the published v0.1.4 tag and assets. Package
only the successful exact-source CI executable with checksums, build provenance
and complete locked dependency notices. Debug logging is opt-in, has observer
cost and may drop records; this release does not close compatibility or comparative
performance acceptance.

## 2026-11-07: explicit diagnostic levels and debug request traces

The previous maximum `warn` mixed ordinary slow-work diagnostics with errors and
could not reconstruct successful requests or parallel phase lifetimes. Make
`warn` the normal default, move slow/inflight detail to `info`, and provide
explicit `debug` request/executor/reply/completion, cache/fill, phase and S3-attempt
events. Preserve request/phase IDs, bounded error classes and cancellation versus
success, without paths, payloads or backend messages.

Reuse tracing-appender's bounded lossy worker. Debug mode bypasses rate sampling
for every severity with a 256-record queue; ordinary error/warn/info modes retain
separate one-record-per-second slots and the normal 16-record queue. Filtering and lazy detail construction keep
debug formatting out of ordinary operation. A real fuser-wire regression first
fails because successful requests produce no trace; loopback HTTP and blocked
stderr fixtures cover remote attribution, cancellation, bounded drops and progress.
Verbose logging has explicit observer cost and can lose records under pressure;
this is not an exhaustive audit, kernel trace or performance improvement.
Final review reproduces 299 suppressed severe replies in a 300-request debug
burst; mode-based rate bypass corrects that gap. The same test retains bounded
queue drops and independent FUSE progress. Session-joined counter assertions also
remove a fixture race after socket delivery without weakening the expected count.
The published v0.1.4 release is unchanged; its old level meanings remain historical.

## 2026-11-07: release 0.1.4 preparation

Advance the package and lockfile together for the bounded READDIRPLUS metadata
pipeline below. The operator requests a versioned release and a scoped Linux/S3
check. Package the successful exact-source CI executable with checksum/build
provenance and the complete locked dependency notices. Publication does not close
filesystem, production or Go-relative performance acceptance.

## 2026-11-06: overlap READDIRPLUS directory attributes

An extended delimiter LIST supplies common-prefix names, not authoritative
directory-marker attributes. READDIRPLUS subsequently refreshed these inodes
serially, so one fast parent LIST still led to a sum of per-directory HEAD/LIST
waits before publication. A real fuser wire regression first reproduces this
serialization with two independent marker requests gated before either response.

Pipeline owned refresh futures through the existing futures-util `FuturesOrdered`
and global metadata semaphore. Admission precedes network-job creation; fresh
attributes bypass that lane. Preserve names/cookies, marker metadata, observation
times, concurrent-writer generation checks and lookup ownership. Full buffers,
errors and deadlines drop pending futures and their permits. Basic READDIR keeps
its direct path and performs no new attribute work. No dependency, cache budget,
TTL, worker count or served mount changes.

The checked 110-directory protocol fixture falls from roughly 381 ms to 65 ms
with identical backend operation counts. This demonstrates removal of the serial
critical path in the fixture, not attribution of a past request or a measured
production/Go speedup. [Verification and limitations](testing.md#readdirplus-directory-attributes--2026-11-06).

## 2026-11-06: bounded latency and remote-work diagnostics

Cache/RSS counters and terminal errno alone cannot attribute read tails, namespace
retry conditions or excess remote work. Completed jobs also hide unfinished work,
and READ read-ahead can outlive its successful reply. Add opt-in, fixed-vocabulary
reply/phase histograms, inflight/oldest observations, source events, SDK attempts
and bytes, process CPU and rate-limited path-free JSON records. Use supported
prometheus-client/Smithy APIs without changing storage or retry semantics.
Exclude presigning from transport attempts and replace our SDK observer when
reapplying configuration. Keep reply publication distinct from tail cancellation.
Retain bounded error classes before errno conversion and per-stage failure/
cancellation flags, so a metadata EIO can identify a failed LIST without exposing
backend messages. This supplies evidence for subsequent failures, not a diagnosis
of historical EIOs.

A real fuser regression reproduces a blocked stderr retaining the sole async
worker even after its EIO reply. Use tracing-appender's bounded lossy output worker,
with a 16-record queue and enqueue-drop counter. Explicit `off`/`error`/`warn`
levels default to errors and keep metrics independent of log filtering. The
blocked-output and full-queue controls verify continued replies/admission, drops
and path-free complete records. Output delivery is best effort, not durability.

Busy namespace/cache snapshots now omit only the affected component, with explicit
availability; they no longer hide independent observations behind HTTP 503. No
stale snapshot or fabricated occupancy is retained. `--no-diagnostics` keeps basic
metrics for same-binary observer controls. Real fuser/HTTP regressions cover reply,
retry, cancellation, source attribution and log bounds. The [collection guide](metrics.md)
defines comparison limits and ignored overhead fixtures. This adds diagnosis;
it does not explain old requests retrospectively or close mounted/performance gates.

## 2026-11-06: release 0.1.3 preparation

Advance package and lockfile together for the bounded diagnostic build. Release
assets require successful merged-main CI and source/checksum provenance. The
authorized single-reader update remains a scoped trial, not general acceptance.

## 2026-11-06: release 0.1.2 preparation

Advance the package and lockfile version together for the namespace-progress and
lock-scope corrections below. Release assets must come from successful merged-main
CI with their source provenance and checksums. A versioned release and an explicitly
authorized reader update do not close filesystem or comparative-performance gates.

## 2026-11-06: namespace progress for readers and writers

Full clean-namespace scans in periodic flushing and dirty-descendant checks,
plus directory overlays visiting every descendant, monopolized the same mutex
used by synchronous inode resolution. Regressions reproduce blocked RO/RW
resolution, unrelated-state blocking in RW flush/rename and nested listing.

Use a maintained Papaya weak inode index with atomic ownership-aware eviction;
keep multi-name transactions under the existing mutex. Index dirty generations,
seek directory children with bounded critical sections and restrict rename scans
to its bound subtree. Preserve upload retry/cancellation, newer writes and dirty
rename/unlink semantics. An additional fault-sensitive regression prevents old
deleted-inode fsync from clearing a recreated name's dirty generation.

The follow-up lock audit removes the redundant mutex spanning cached response
copies, groups cache-hit metadata work and indexes revocable loan keys. Move
FUSE replies/scheduling and directory inode work outside unrelated admission,
registry and inode-state locks. Retain necessary generation/durability gates and
document remaining contention risks in the [locking audit](locking.md).

The in-process scaling fixture diagnoses the removed clean-cardinality costs;
its extra inode index increases metadata RSS. It does not establish mounted
FUSE, Go-relative or production acceptance. See [verification](testing.md#namespace-progress--2026-11-06).

## 2026-11-03: release 0.1.1 preparation

Advance the package and lockfile version together so the executable identifies
the release containing the independent-review corrections and Ubuntu 26.04 build.
Dependency versions and runtime behavior are unchanged by this version bump.
Release availability remains separate from filesystem and performance acceptance.

## 2026-11-03: independent review corrections

Regressions reproduce lost applied PUT/multipart replies stranding subsequent
flushes, stale mtime captured before admission, out-of-order metadata HEADs,
marker metadata hidden by implicit-directory shortcuts, and lost rename-parent
lifetime. Add bounded upload provenance/reconciliation and use the existing
generation, observation-time and pending-operation ownership checks consistently.

Complete payload-budget validation now precedes startup backend activity. FUSE
deadline cancellation returns ETIMEDOUT only for unsent replies and preserves an
already delivered READ. Deterministic public-API, loopback SDK and fuser transport
regressions cover the failures and retained side-effect/ownership boundaries.
The Linux release build now uses Ubuntu 26.04. These fixes do not establish
sustained performance or full filesystem compatibility acceptance.

## 2026-11-03: addressed metrics-build and cache-budget update

A new operator authorization updates the same three RO trial readers sequentially
to the metrics build, with 48 GiB total payload and 30 million metadata entries.
The third reader stays withdrawn until both peers pass their checks and its own
update completes. Full protocol bytes, mount propagation, listing-process retention,
loopback metrics and healthy recovery no-ops pass. Retain the controller startup
error and lock-only refusal alongside their distinct successful continuations.
This is deployment/context delivery without a runtime-code change. Historical
timings retain their original build; missing comparable CPU/memory baselines and
new cold-cache observations do not establish a performance or memory improvement.

## 2026-11-03: optional Prometheus cache and memory observations

RSS and diagnostic allocator JSON could not establish payload occupancy, reuse or
eviction pressure. Add an opt-in bounded HTTP/OpenMetrics listener using public
prometheus-client and Hyper APIs. Constant-time nonblocking snapshots distinguish
retained spans from charged/pinned storage, and report read reuse/fills, namespace
pressure and shared allocator/RSS observations. Scrapes never refresh metadata or
call S3. Tests cover accounting, failure, contention, HTTP lifetime and an explicit
real-binary disposable FUSE/S3 fixture. This changes observability, not cache limits
or production deployments; larger-cache usefulness remains to be measured.

## Third addressed reader and error-screen correction — 2026-11-03

A separate authorization extends the same-build, same-budget RO trial to a
third reader. Its first ordinary-unmount refusal preserves the old mount;
the continuation waits for empty consumer cgroups and passes protocol bytes,
namespace propagation and healthy recovery no-op. No runtime code changes.

The error screen must match vhost/method/URI rather than combine different
virtual hosts. The third reader's sampled 404 rate rises; all newly observed
canonical missing URIs have current native absence evidence, with the original
TLS timeout retained. Earlier EAGAIN/EIO, backend read failure and upstream
timeout remain separate from successful controls. Memory growth and broader
acceptance are unresolved. [Scope and limits](pr-context.md#scoped-serving-trial--2026-11-02).

## Second addressed reader trial — 2026-11-03

A separate operator authorization extends the read-only serving trial to one
additional reader with the same source-matching executable and cache/resource
budgets. Ordinary unmount, consumer namespace propagation, independent peer-host
HTTPS and rsync bytes and healthy recovery no-op pass. Preserve the prior live
error/anonymous-growth gaps and the second reader's transient EIO/HTTP 500;
later successful controls do not attribute that failure. A cold new cache is
not a fair memory comparison.
[Combined scope and limits](pr-context.md#scoped-serving-trial--2026-11-02).

## Scoped serving trial and recovery boundary — 2026-11-02

The operator authorized one read-only served-reader replacement, separately from
general acceptance. Deployment recovery now recognizes the addressed Rust mount,
quiesces filesystem consumers and preserves native-S3 listings; healthy delivery
does not restart the reader. Actual production crash/OOM recovery is unobserved.
Live errors and continued anonymous-memory growth still need attribution; defer
cache enlargement.
[Trial scope and remaining uncertainty](pr-context.md#scoped-serving-trial--2026-11-02).

## Cross-UID mount access — 2026-11-02

Owner-only fixtures missed a serving contract: the frozen Rust binary rejected
`allow_other` and denied another UID even on a readable file. Accept the explicit
option and select supported fuser `SessionACL::All`; keep the default owner-only
policy and kernel `default_permissions`. Preserve other supported mount options
and reject unknown ones.

The parser regression fails before the change. An omitted-ACL wiring fault then
fails the real-FUSE readable-byte assertion; restored wiring passes alongside
owner-only and mode-0600 denial controls. Both allocator suites, explicit native
access checks, formatting, Clippy and 99 Python tests pass. Scoped quality passes
do not close broader-file CRAP or production acceptance. [Verification](testing.md).

## Reference, cache policy and statistical contract — 2026-11-02

Go 0.43.9 is the primary acceptance reference. The unmerged Go experiment remains
historical evidence, including favorable and adverse measurements. Go-relative
superiority requires equivalent correctness: demonstrated stale permission
metadata or weaker durability is not behavior to reproduce for speed.

The operator accepts Go-equivalent bounded negative caching. Use explicit Rust
negative64 and one-minute stat TTL in read-only comparisons; 64 is a per-directory
capacity, not seconds. The cache default remains zero. Eight controlled blocks
attribute the default listing loss to five absent-name checks/15 S3 requests;
the existing option removes those requests. The separate five-second expiry
probe demonstrates accepted external-create delay. [Listing diagnosis](performance-listing-diagnosis.md).

Withdraw strict per-pair/all-metric inference from short trials. Require paired
confidence intervals, eight independent mounts and genuine same-executable A/A
controls; superiority and practical non-inferiority are separate decisions.
Lookup-overlap and cached-observation trials remain absent/inconclusive.
[Protocol](performance-methodology.md).

## Rename notification deadlock and dispatch controls — 2026-11-02

A native four-client rename regression hangs before correction. Synchronous
post-reply entry notifications can wait for a directory operation while holding
the notifier lock. Linux/VFS already maintains rename dentries; remove the
redundant notifications while retaining last-close inode invalidation and durable
unmount. No extra executor hides the cycle.

Supported receive-loop controls retain allocation/rate objections. Inline first
polling has inconclusive mixed performance after A/A calibration, but a native
contended read stalls the sole receive thread: reject it on liveness. Trial
runtime/tests/CLI remain absent. Backend errno/fallocate helpers preserve behavior.
[Measurements and public-source analysis](performance-classic-dispatch.md).

## Read-only target and bounded native verification — 2026-11-02

The clarified primary target is LXC serving nginx/rsync/golist, with read-only
performance first and writer correctness preserved. Isolated actual-kernel
nginx/rsync checks pass byte/Range/EOF/416 and read-only contracts. Buffered shared
mmap passes; direct I/O returns `ENODEV`. Corrupt-byte controls fail correctly.
[Target scope](performance-target-preflight.md).

The transformed read-shape comparison completes eight five-arm blocks. A separate
guarded real-S3 attempt stops after five complete blocks on an upstream HEAD
timeout, below acceptance requirements. Retain cumulative budgets, setup failures
and cleanup receipts. An audit also finds stale disposable rename mounts despite
failed units: verify actual task exit, ordinary unmount and unchanged unrelated
mounts rather than trusting unit status. [Read controls](performance-read-shape.md).

## Metadata observation age and bounded directory work — 2026-11-01

Standard LIST renewed cached user metadata's observation time, extending permission
TTL without fresh metadata. A source-age regression and native mode-only update
fail before correction. Carry actual observation timestamps through bounded
selection/type resolution and return remaining TTL. Protect newer HEAD/commit
observations. Equal-TTL probes independently show stale Go modes at unchanged
content ETag; lower RPC counts are not equivalent freshness.
[Diagnosis](metadata-expiry-diagnosis.md), [corrected-source measurements](performance-observation-age.md).

Consumed directory prefixes were repeatedly copied/validated and could trigger
extra LIST after inode eviction. Resume only the unconsumed suffix without
extending TTL or pinning inodes. Then align bounded FUSE consumption with the
backend page size: an 8192-name traversal drops from 65 LIST calls to the expected
10, including EOF. Kernel-buffer continuation, accurate types, lookup ownership
and error boundaries remain tested. The READDIRPLUS_AUTO trial stays absent.
[Directory measurements](performance-directory-remainders.md).

## Owned uploads and clean-close cache retirement — 2026-11-01

A retry regression sends 302 HTTP body bytes for nine dirty bytes because a known
contiguous body used the generic aws-chunked path. Reuse supported Bytes/Smithy
known-body APIs for covered extents, retaining charge/activity through the last
retry owner. Sparse/composed/remote streams, signing/checksum policy and
post-commit validation are unchanged. Byte/checksum/lifetime faults verify the
correction. [Owned-upload measurements](performance-owned-uploads.md).

Owned-superblock/page attribution identifies 256 MiB of clean payload pages left
after close although the next open discards them without `FOPEN_KEEP_CACHE`.
Use supported best-effort inode invalidation after last clean close; release
userspace locks before notification and preserve mmap/reopen owners. The retained-
page regression fails before and passes after correction. Lower temporal file-
cache means do not prove process-memory superiority or a plateau.
[Integrated measurements](performance-integrated.md).

## Freshness, handles and admission progress — 2026-10-31

FUSE/HTTP joins place late metadata bursts inside LOOKUP: cached name pages retain
older child attributes. Validate freshness/type/binding, refresh authoritative
extended LIST and return remaining kernel TTL. Supported bounded READDIRPLUS
transfers lookup ownership only on successful replies. V1 marker/V2 start-after
avoid repeated standard-LIST prefix scans while opaque continuation retains
priority. Preserve accurate common-prefix metadata and lexical boundaries.

Move adaptive history to each open handle; cache/fills/budgets remain shared.
Reserve handles before create so `EMFILE` cannot leave a dirty name. Clean fsync
preserves cache loans when no generation needs uploading. Linux hardlink-as-symlink
fails after mutation in the reference-style path; Rust now rejects unsupported
LINK before mutation. [Architecture](architecture.md), [compatibility](compatibility.md).

A rare 30-second read is reproduced by a gated public test: required admission
does not revisit cache owners published after its first reclamation sweep.
Notify-driven reclamation retains FIFO place and unchanged budgets. Separately,
directory-local epochs prevent unrelated writes invalidating all cached listings;
in-flight binding checks preserve rename/delete correctness.
[Native checkpoint](performance.md#native-vm-admissiondirectory-checkpoint--2026-10-31).

## Read path and benchmark accounting — 2026-10-27–30

Coalesce overlapping generation-scoped fills and publish into the current cache
instead of an orphan clone. Charged immutable views avoid redundant cache/reply
copies; incremental SDK body publication retains checksum barriers. Drop consumed
transport owners promptly and transfer charges to actual fragment backings.
Maintained `lru` supplies synchronous payload eviction; metadata stays in Moka.

Cache-hit progress now plans refill after reply publication within the existing
owned lifetime. Speculation cannot evict another reader's unread backing; required
admission retains its policy. Reopening resets promoted history without discarding
payload. Required admission can borrow idle flush capacity, revocable by uploads;
owned replies cannot export a revocable backing. Byte-budget/lifetime regressions
fail for the original mechanisms. [Historical cache-policy measurements](performance-cache-policy.md).

The runner now warms the actual operation, excludes cold superiority from warm
acceptance and reports complete temporal CPU/memory scopes. A bounded Go observer
reads live HeapAlloc without requesting GC; missing samples stay missing. Go's
open-handle fsync recovery failures remain failures, distinct from recovery after
close. ThinLTO/System-allocator comparisons and observer-on/off partial attempts
retain their exact configurations. None establishes all-workload acceptance.

## Dependency reuse and suspended transport work

Reuse public fuser, AWS SDK, Tokio and maintained ownership/cache APIs instead of
custom protocols or unsafe adapters. Actual-service coverage remains incomplete.
Linux 7.3 pool/zero-copy research and standalone C controls do not supply a fitting
released Rust transport. FUSE io_uring stays an [upstream watch](fuse-io-uring-assessment.md);
host boot/package recipes and execution checklists are archived outside the PR.
