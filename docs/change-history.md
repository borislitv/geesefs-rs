# Change history and reasons

This is the concise engineering context for the experimental Linux/S3 rewrite.
Detailed results and operational logs stay outside tracked source files.
Overall compatibility and comparative performance remain **UNACCEPTED**.

## Correctness-equivalent gates and small-core io_uring assessment, 2026-11-01

- Record the operator's clarification: Rust need not beat Go where Go is
  demonstrably logically weaker. Preserve raw reference rates, identify the
  unequal contract and retain unrelated performance/non-regression obligations.
- Verify Linux 7.3/7.3.1 release dates and versioned FUSE code: the transport
  predates 7.3; buffer pools, registered pools and per-open page-backed zero-copy
  are the new opportunities. Installed fuser has constants, not a ring transport.
- Inspect public reuse candidates and small-core queue/worker sizing. Fewer
  possible vCPUs plus shallow queues are viable experiment parameters; CPU quota
  or worker count alone does not reduce possible-CPU queue coverage. New pools
  decouple parked entries from payload owners. Keep transport, pool and end-to-end
  zero-copy claims separate; no kernel/runtime change or measured gain is claimed.
  [Evidence, benefit assessment and comparison design](fuse-io-uring-assessment.md).

## Metadata expiry and reference freshness attribution, 2026-11-01

- Reassemble bidirectional HTTP with maintained TShark and join calibrated 999 Hz
  CPU profiles to 42 checked metadata phases. Distinguish flat-prefix LIST from
  payload GET and CPU from overlapping in-flight intervals; no new timing win is
  claimed. Rust's expiry burst resolves accurate metadata with 1,024 HEADs.
- Six equal-TTL S3/FUSE probes show both pinned Go references miss unchanged-ETag
  permission updates, including a successful fresh HEAD in the stat-only route.
  Rust sees the update; all three see the changed-ETag positive control. Exact
  reference sources apply metadata in the ETag/size-change branch and can renew
  cached attributes. Lower RPC count does not establish equivalent freshness.
- Keep `1b13033` and all 85 frozen inputs unchanged. No supported SDK prepared-
  operation API or measured minimal runtime correction is established. Preserve
  failed capture/verifier assumptions and scope in ignored evidence; continue
  setup/owner attribution without removing required validation or metadata.
  [Method, results and limitations](metadata-expiry-diagnosis.md).

## Rejected cached directory observation reuse, 2026-11-01

- Public ownership regression exposes copying/decoding an already published inode
  observation back into itself during directory publication. A strong-Arc trial
  additionally fails namespace-pressure eviction; its corrected weak-reference
  form preserves original observation age and the bounded fallback metadata.
- Experimental 262-case allocator suites, matching native clients and three
  selected mutations pass. Four short metadata pairs verify 8,000 phases and
  eight final generations, but throughput/p99 each lose twice and late allocation
  increases every time. These short cached traversals do not measure sustained
  expiry. [All rates, tails, CPU, memory and limitations](performance-cached-observations.md).
- Discard the experimental runtime/tests and verify exact restoration of all 85
  v8 inputs. Retain `1b13033` and its 260-case source scope; overall acceptance
  remains open. Attribute the actual expiry/request burst before another variant.

## Rejected lookup-probe overlap, 2026-11-01

- Trace/source attribution separates kernel LOOKUP, create validation and mandatory
  post-commit HEAD. Retaining a miss or removing create validation would change
  negative-cache/race behavior; neither is applied.
- Optional marker HEAD/implicit LIST overlap passes meaningful public and SDK/HTTP
  failure-sensitive tests, both 268-case experimental allocator suites and matching
  native clients. Four rotated writer pairs nevertheless lose throughput once,
  p95/p99 three times and peak/late daemon RSS in all four repetitions. The checked
  diagnostic removes no RPC. [Complete results](performance-lookup-probes.md).
- Reject the experiment and restore every runtime/test input to observation-age
  source `1b13033`. Experimental 268-case results do not replace the retained
  source's 260-case verification. Whole-file CRAP, broad mutation, compatibility
  and joint performance acceptance remain open.

## Preserving metadata observation age through listing, 2026-11-01

- A source-bound expiry diagnostic identifies 1,002 HEADs and one LIST inside a
  slow metadata phase. Inspection finds cached, LIST and HEAD metadata collapsed
  into one page-publication timestamp. Standard LIST cannot revalidate cached
  permission metadata, but publication incorrectly restarts its full TTL.
- The age regression first returns almost 60 seconds instead of at most 15.
  A real mounted permission test changes only backend mode 0640 to 0600 and
  rejects the old publication-time assignment after the original TTL expires.
  The initial size-only probe is retained with its limitation: LIST includes size.
- Carry each entry's actual observation time through bounded selection and type
  resolution. Preserve exact metadata, newer HEAD/commit protections, limits,
  remaining kernel TTL and cancellation/error behavior; no cache-policy tuning.
- Both allocator suites pass 260 Rust cases, including ten native directory
  contracts, and matching release/instrumented clients pass. This is a correctness
  correction, not a speed claim. Earlier performance tables retain their source
  and now-known freshness gap; current-source joint performance remains unaccepted.
- [Two current-source mixed repetitions](performance-observation-age.md) verify
  1,440 phases and eight final full-byte/generation checks, with clean owners,
  counters and no lateness. Metadata/writer rates still lose to both Go references;
  process RSS and prior-Rust-relative metrics retain losses. The predecessor's
  lower refresh cost cannot excuse its permission-freshness defect.

## Resuming only the remaining directory page, 2026-11-01

- A backend page spans several kernel frames. The adapter repeatedly validated
  and copied its consumed prefix; namespace pressure on an already-emitted inode
  could unnecessarily repeat LIST. The public remainder regression first fails
  on two LIST calls instead of one after real inode eviction.
- Binary-search past consumed names, retaining the original page/epoch/TTL bound
  and exact validation of every remaining entry. Preserve backend continuation
  separately from EOF so new prefixes cannot hide later names. No cache size,
  kernel capability, inode pin, lookup ownership or metadata contract changes.
- Stale-type and inserted-prefix regressions reject deliberate freshness and
  continuation faults. Both allocator suites pass 258 Rust cases; matching
  release and instrumented native client variants pass. The selected partition
  mutation is killed; whole-file CRAP and broader mutation gaps remain open.
- Four rotated 1,000-traversal metadata-only pairs improve throughput by
  7.0–13.3%, but some tails and memory metrics lose. This short source attribution
  is not repeated sustained mixed acceptance. A separate AUTO experiment is
  rejected: three throughput losses and four pooled-p99 losses in four pairs.
- The [two-repetition mixed comparison](performance-directory-remainders.md)
  verifies 1,440 phases and eight dataset generations with clean counters and
  cleanup. Metadata improves against prior Rust in one repetition and loses in
  the other; random throughput and cgroup CPU lose in both. Go-relative metadata,
  writer, tails and process-memory gates remain unaccepted.

## Reusing owned contiguous upload bytes, 2026-11-01

- A real filesystem create/write/fsync/retry regression first sends 302 HTTP body
  bytes for nine dirty bytes. The generic streaming adapter forces checksum
  trailers and aws-chunked encoding even when one charged extent covers the body.
- Reuse supported bytes/Smithy known-body APIs for covered ranges up to 1 MiB.
  Retain the original dirty allocation and upload activity through the last retry
  or transport byte owner. Sparse/composed/remote ranges remain bounded streams;
  signing, checksum policy, limits and post-commit HEAD are unchanged.
- Wire tests check 1/9/1,048,576-byte bodies, independent CRC32/SHA256, signed
  checksum headers, retry after 503 and verified completion. Cancellation/truncate
  ownership tests reject deliberately lost activity and replaced byte charges;
  multipart tests check a nonzero offset into a split dirty backing.
- Both allocator suites pass 255 Rust cases; available matching release and
  instrumented native contracts pass. A separate short checked mounted trace
  observes exact 1,048,576-byte Content-Length for both writer PUTs. This proves
  use of the new path, not sustained speed. The subsequent
  [four-binary comparison](performance-owned-uploads.md) verifies 720 phases but
  still loses metadata/writer/random throughput and some tails. Whole-file CRAP,
  surviving mutation sites and repeated joint acceptance remain open.

## Backend-aligned directory batches, 2026-11-01

- The bounded FUSE cursor consumed 128 names from each standard S3 page of up to
  1,000, then requested overlapping pages. An 8,192-name native traversal first
  fails with 65 LIST calls instead of 10, including EOF.
- Consume up to the supported backend-page limit while retaining the namespace
  capacity bound. The cursor still resumes across smaller kernel frames; accurate
  attributes, lookup ownership, small-capacity errors and cancellation remain
  covered by all nine native directory cases.
- Matching default/System suites and release/instrumented native clients pass.
  A separate 30x10-second mixed control verifies 720 phases and four complete
  full-byte/generation checks. Metadata throughput is 31,315 ops/s, versus 42,015/42,256
  for experimental/packaged Go; pooled p99 is 0.03241 versus 0.02656/0.02799 ms.
  The earlier retirement-only control measured 19,777 ops/s, but separate runs
  on a shared host do not isolate the batch change's performance effect.
- Rust late cgroup/file means remain 216.25/19.54 MiB versus Go 357.97/173.84
  and 371.53/156.78 MiB; these are temporal means, not memory plateaus. Writer and
  random-read losses remain. Experimental Go misses three offered-rate deadlines;
  the other binaries miss none. Whole-file CRAP and broader compatibility remain
  open; this single control does not pass the repeated joint acceptance gate.

## Closed-file kernel cache retirement, 2026-11-01

- Owned-superblock page events and PFN-to-cgroup attribution show 256 MiB of
  payload pages remaining after the last clean close. With `FOPEN_KEEP_CACHE`
  unset, the next open discards these pages. Faster Rust completion increases
  idle time retaining the full kernel working set, inflating temporal file-cache
  means without a larger final page count than Go.
- A checked native trace first fails on 268,435,456 retained bytes. Use supported
  fuser `Notifier::inval_inode` after releasing the last clean regular-file handle.
  A separate single-slot blocking owner avoids consuming FUSE request admission;
  bounded shutdown joins it. Eligibility is rechecked without holding userspace
  locks across kernel invalidation. Reclamation is best effort under reopen,
  remaining read owners or concurrent retirement, not an immediate memory bound.
- The same page assertion passes with zero final payload pages. Matching release
  and jemalloc/System native clients pass, including mmap after closing its
  original descriptor, rsync/nginx and durability/failure checks. Budgets, TTL,
  allocator policy and Go GC policy are unchanged.
- A separate 30-cycle, 10-second paced comparison gives late-half cgroup means
  217.75 MiB for corrected Rust versus 355.43/371.88 MiB for experimental/packaged
  Go; file-cache means are 19.23 versus 171.68/159.30 MiB. These are temporal means,
  not plateaus. Rust metadata throughput remains 19,777 versus 36,314/40,259 ops/s;
  writer throughput and some random-read metrics also lose. This single controlled
  design does not close repeated joint performance, quality or compatibility gates.

## Integrated freshness, handle and accounting correction, 2026-10-31

- A frozen-baseline FUSE/HTTP join attributes late metadata tails to name pages
  retaining older child attributes. Refresh authoritative extended LIST metadata
  without replacing dirty state or a newer HEAD/commit; return remaining kernel
  TTL rather than restarting it. Supported fuser READDIRPLUS provides attributes
  with bounded page cursors and lookup ownership transferred only on success.
  Unknown common prefixes still resolve marker permissions and ownership.
- Adaptive history now belongs to each open handle; cache, fills and budgets stay
  shared. Handles retain admitted readers and reserve capacity before create, so
  EMFILE cannot leave a new dirty name. Validate the supported semaphore limit.
  Clean fsync no longer reclaims cache loans when no generation needs uploading.
- Native failures and restored semantic mutants verify TTL, listing generations,
  cursor/lookup ownership, handle pressure, borrowed-read history and durability.
  Linux hardlink emulation now fails EOPNOTSUPP before mutation; this is a safe
  unsupported contract, not hardlink compatibility acceptance.
- Memory accounting streams the complete temporal window and preserves missing
  observations. Shared nearest-rank/raw-latency policy feeds both pooled gates and
  summaries; daemon/client, allocator and file/kernel scopes remain separate.
  Safe optional jemalloc page gauges add attribution without allocator tuning.
  Local nginx/rsync and failure checks pass for the implemented Rust subset;
  whole-file CRAP and actual-service gaps remain open.
- A subsequent frozen-candidate trace shows repeated standard LIST prefix scans.
  Supported V1 marker/V2 start-after avoids rescanning consumed bounded chunks;
  opaque continuation and extended protocol semantics stay intact. Independent
  lexical-boundary, malformed-response, pagination-limit and native cursor tests
  protect accurate types and marker attributes. Separate default/System builds
  pass 251 Rust cases and real instrumented clients; the locked notice bundle
  retains complete dependency sources, including native/embedded attribution.
- The joint [2026-11-01 measurements](performance-integrated.md) independently
  verify 240 isolated healthy records and 2,880 mixed phases. Paced metadata p99
  improves against prior Rust, but throughput and equal-rate/concurrent memory
  retain losses. Missing accounting, original isolated-controller failures and
  full required compatibility remain explicit acceptance gaps.

## Persistent admission and metadata correction, 2026-10-31

- Native VM CPU profiles, scoped FUSE lifecycle events and temporary ownership
  diagnostics reproduce the rare 30-second read. Admission waits for 42,074,112
  bytes while an in-flight fill publishes 170 cache fragments over about 25 ms;
  no second reclamation sweep runs before timeout. A deterministic gated public
  read regression fails for the same mechanism before correction. Admission now
  reclaims newly published owners through Tokio Notify while retaining FIFO place,
  unchanged budgets, pinned-owner accounting and cancellation.
- Persistent metadata repeatedly makes 15 LISTs per cycle because an unrelated
  writer changes the global namespace epoch. A fresh-page regression first fails
  on the extra backend call. Directory-local epochs invalidate affected parents
  and moved/deleted subtrees instead. Existing TTL and dead-inode checks remain;
  in-flight responses also validate their binding. A separate observed failure
  during authoritative directory removal catches the missing binding check.
- Tests cover unaffected in-flight listings, both rename parents, descendant
  listing rejection during a parent move, waiter cancellation and byte/budget
  correctness. Temporary runtime diagnostics are removed. These corrections do
  not establish all-workload comparative or production acceptance.

## Initial implementation and correctness

- Reused fuser and AWS SDK rather than implementing FUSE/S3 protocols. Added
  bounded allocation ownership, generation-aware namespace state and sparse,
  truncate, multipart, rename and fsync paths. Optional backend adapters preserve
  reference metadata, authentication, signing and PATCH contracts through public
  dependency APIs; actual-service coverage remains incomplete.
- Reserved a flush lane and retained permits through pinned/queued buffers and
  cleanup. This fixes progress and ownership, not whole-process RSS.
- Bounded concurrent type resolution and directory-page/preload reuse avoid
  serial HEAD/repeated LIST work. Binding-aware metadata validation prevents an
  unrelated remote deletion from invalidating successful operations or late
  responses from removing dirty/newly durable descendants.

## Read-path investigation, 2026-10-27–28

- Adaptive history originally missed cache-hit progress. Corrected contiguous
  tracking, but deterministic request reduction alone did not fix mounted speed.
- Actual FUSE tracing then found neighboring reads evicting active-fill policy
  and publishing into an orphaned cache clone. `572195a` coalesces overlapping
  generation-scoped fills and publishes into current cache; disjoint work remains
  concurrent. This resolves the severe selected sequential throughput collapse.
- Charged immutable views remove redundant cache-hit and callback reply copies.
  Requested bytes can reach FUSE before unrelated read-ahead finishes, while the
  same caller-owned future retains cancellation, errors and admission.
- Incremental SDK body publication avoids waiting for an entire unchecksummed
  range. Whole-body checksums retain their validation barrier. Maintained Tokio
  copy APIs avoid pre-zeroing fully overwritten fragments. These changes had
  mixed CPU/RSS/tail tradeoffs, not an unconditional performance win.
- A consumed transport `Bytes` owner survived until the next body poll and
  impeded receive-buffer reuse. Maintained futures/Tokio adapters now drop it
  promptly. Permit splitting transfers charges to actual fragment backings.
- Required pressure eviction used whole-cache replacement because Moka lacks
  synchronous pop-LRU. Payload retention now uses maintained `lru`; metadata stays
  in Moka. Range-index coverage filtering avoids useless cache lookups.
- System allocator screening exposed a speed/RSS tradeoff. It remains an explicit
  comparison feature with scoped counters; jemalloc stays the default. Counter
  ablation did not identify telemetry as the remaining bottleneck.

## Read-ahead corrections, 2026-10-29

- Equal higher CPU/memory resources removed quota throttling and reclaim, but
  Rust tail latency still lost to Go. Resource effects were not a source fix.
- `6e16525`: cache hits skipped next-window planning, exposing the next GET on
  demand. Socket/FUSE joins and selective-delay injection confirmed this mechanism.
  Refill now runs after reply publication in the existing owned request lifetime,
  skips unavailable optional admission and preserves integrity/cancellation.
- `d0d7b99`: optional refill could evict another reader's unread backing. Track
  contiguous consumption and reclaim foreign backing speculatively only when
  fully consumed. Required LRU/FIFO admission stays unchanged. Traffic and tails
  improve, but the experimental Go p99 gap remains.
- `2bf8184`: inode history survived the final close, so reopening inherited a
  promoted initial window. Reset `Pattern` on zero-to-one open, preserving payload
  and active-reader history. Warm duplicate payload disappears in the selected
  comparison; p99 improves versus prior Rust but still loses to both Go references.
  P95/CPU regressions versus that prior remain part of the conclusion. Concurrent
  handles still share inode policy; full per-handle isolation is not implemented.

## Benchmark contract correction, 2026-10-30

The acceptance runner still required cold-start superiority after that requirement
was removed. Cold runs now retain all ratios and correctness, provenance and
accounting checks, while comparative performance gates apply to warm work.
The regression fails before the correction; adverse warm tails still fail.
This aligns the runner with the contract and establishes no runtime improvement.

Go allocator accounting no longer substitutes a final sampled pprof total for
missing temporal data. A separate bounded observer records live `HeapAlloc`
without requesting GC, retains failures and excludes boundary-crossing requests.
Loopback HTTP/timeout tests and four injected faults verify the adapter; actual
workload overhead and long-lived heap behavior still need measurement.
Reference inspection also finds baseline-owned `debug.FreeOSMemory` calls. Their
counter deltas are reported without rejecting the reference or changing its GC
policy; the observer itself never requests collection.

The worker also labelled metadata/random/writer runs warm after scanning only
payload. Warmup now executes the matching checked operation; the failure case
primes a healthy writer before injection. The client is closed even if warmup
fails, and warmup receives the bounded workload deadline rather than the short
cold-initialization timeout. Earlier screening evidence remains diagnostic.

Open-file fsync recovery failures in both pinned Go references are consistent
with their shared flush state machine: `SyncFile` clears `forceFlush` before
returning the retained error, and small-file background upload needs a closed
handle, `forceFlush` or memory pressure. Waiting past the retry interval does not
by itself restore progress. Preserve those failures and the original workload
rather than increasing retries until a different state happens to pass.

## Idle flush-capacity cache policy, 2026-10-30

Fixed-binary native controls confirm repeated warm random GETs in Rust while both
Go references issue none. These controls/profiles disable Go's allocation-triggered
GC threshold with `--gc-interval 0`, retaining pressure GC; their scope is configured
diagnosis rather than default-policy comparative acceptance. CPU profiles and
complete FUSE/transport joins locate the corresponding waits without quota
throttling or memory reclaim. Observers
add measurable CPU/time, and the profiles do not identify a single SDK defect.
Exact-version upstream discussions and supported Tokio APIs were checked first.

The new public-behavior regression fails with 192 rather than 96 reads for a
12 MiB clean working set at an unchanged 16 MiB payload limit. Required admission
now transfers exclusive cache backings into idle flush permits instead of losing
those bytes. Upload activity revokes loans and follows cloned budgets/bodies;
owned replies copy loaned bytes into data admission. No total/dirty limit or
durability barrier changes. Optional speculation keeps its previous policy.

Kubernetes verification passes 211 Rust cases, 57 Python cases, fmt, all-target/
all-feature Clippy and the public-source audit. Five deliberate faults fail the
intended assertions: disabled lending, exported loan owner, omitted revocation,
dropped upload activity and omitted reopen reset. Sources are restored and tests
rerun. The history test now forces cache eviction independently of its old cap.
Default and System-allocator ThinLTO releases build successfully. The default
release also passes 30 backend, cache-loan and read-view contract cases.
Quality tools report complexity without Rust coverage; CRAP remains unavailable,
and a mutation-site scan is not a whole-repository mutation score. Mounted
comparative performance, sustained memory and compatibility remain separate gates.

## Cache-policy measurement and remaining waits, 2026-10-30

The versioned default-GC comparison completes 120 healthy-warm records on four
binaries, with all byte checks and 30 frozen generations verified. Candidate warm
random GETs are zero versus 3,380–3,389 for its predecessor. Random throughput
improves, but tails, concurrency and memory still lose agreed gates. Mean writer
p99 also hides an unfavorable independently pooled p99. The latest README uses
total checked work / summed measured time for throughput, keeping mean rates,
variance and every outlier in retained evidence.

The first default-GC attempt had stopped after 47 records on memory pressure;
backend working files outlived the paired workload. Scoped per-group bucket
lifetime removes that imbalance in the new configuration without raising payload
or weakening the resource gate. It is not a source optimization. The earlier
GC-threshold-disabled diagnostic and incomplete/rejected attempts are retained.

All four binaries also complete 60 mixed cycles / 360 passes each. Candidate Rust
takes 709.11 seconds versus predecessor 873.78 and both Go at 600.00; its late RSS
still exceeds experimental Go. Raw serial latencies expose 11/16 approximately
30-second successful reads in candidate/predecessor and none in either Go. Quota,
memory-pressure and reclaim counters are clean, but I/O waits are substantial.
Their causal stage remains unproven. Independent whole-series accounting replaces
bounded-tail means for this checkpoint; no plateau or overall acceptance follows.

## Earlier measurements and open gates

The completed 2026-10-30 native operation matrix retains all 210 records: 190
correct and 20 Go open-file fsync-recovery failures. It exposes a severe warm
random loss despite selected sequential/writer gains. Deterministic traces
confirm repeated backend reads under the static data-lane working-set pressure;
they do not isolate all mounted latency. The idle-flush policy above now passes
that diagnostic and the ownership regressions; these older operation/mixed
measurements describe its unchanged predecessor, not the new release.

A persistent mixed mount completes 60 cycles / 360 checked passes on each binary.
Rust takes 885 seconds versus about 600 for both Go references, despite lower
process RSS and CPU. Whole-series cgroup/file/kernel accounting prevents calling
that an all-metrics memory win or plateau. Parent/backend quota interference was
measured and versioned separately; increased headroom is not a code improvement.
The observer-on/off experiment stops at its bounded pre-mount deadline after 133
correct records, rather than completing 180; it remains partial attribution.
The [README](../README.md) contains the requested aggregate checkpoint; detailed
evidence stays ignored. [Architecture](architecture.md) describes delivered
ownership and [next work](next-work.md) separates confirmed causes from hypotheses.

Release builds now use ThinLTO and one codegen unit to optimize across the SDK
and application crate boundaries. With unchanged runtime sources, four rotated
mount pairs improve throughput, p50/p95/p99 and CPU versus the former Rust build
in the selected System-allocator fixture. Small RSS/cgroup increases are retained;
both Go p99 values still win. Default jemalloc builds and release-mode backend
contracts also pass, but this is not representative-workload acceptance.

Initial-read joins find additional Rust client work before GET write and after
required socket bytes. Internal SDK hooks retain the setup/signing/response work
with external observers removed and with client-wide rather than per-request
instrumentation. Default endpoint resolution is only part of that work; its two
calls correspond to auth selection and request endpoint orchestration. These
boundaries do not establish an SDK defect or a causal contribution to global p99.
Keep instrumentation outside the runtime and validate a targeted change before
claiming an improvement.

Hardlink EIO, optional actual-service/CLI/imported-layout compatibility,
representative performance acceptance and long-lived memory behavior remain open.
Go live heap series now exist for the persistent run; short phases can still
lack two valid samples, and observer overhead is not eliminated. Lower daemon
RSS and higher selected throughput do not close these gates.
FUSE passthrough/disk caching remain deferred. Dependency rationale and applicable
upstream links are in [dependency decisions](dependency-decisions.md).
