# Change history and reasons

Concise engineering context for the experimental Linux/S3 rewrite. Compatibility
and comparative performance remain **UNACCEPTED**. Detailed journals, failures,
source/binary manifests and pre-cleanup documents stay in ignored evidence;
measurement pages retain their original tables and source scopes.

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
