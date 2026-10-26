# Progress tracker for plan-geesefs-rust-rewrite.md

Status format: [ ] - incomplete, [X] - completed. Updated: 2026-10-26.
The parent owns BMAD orchestration, final review and all VCS delivery.

## Execution Stages

[X] Stage 1: Dependency decisions, public provenance and reference option inventory.
Selected public crates and narrow adapter rationale recorded. Cargo metadata
reports 290 packages, zero private sources, all license declarations present.
Binary redistribution notices remain a final-delivery gap.

[ ] Stage 2: Cargo library/binary and complete typed CLI/configuration/errors.
Build and foreground read/write mounts work. Supported flags are validated;
unimplemented flags reject. Complete CLI/IAM/encryption/signing semantics remain.

[ ] Stage 3: Complete reference CLI/metadata fixtures and contracts.
Metadata tests now include Unicode categories, invalid UTF-8 RuneError and byte
escaping. Supported configuration and attributes have tests; not all CLI paths do.

[ ] Stage 4: Full backend integration.
One SDK client supplies standard S3 operations and extension request signing.
V1/V2/ext-v1 listing, bounded pages, exact fallback and compression validation
are wired. IAM, encryption/signing modes, PATCH and aged-upload cleanup remain.

[ ] Stage 5: Complete backend contract suite.
Real loopback HTTP tests cover conditional Range, stored gzip identity, short
body/access errors, metadata, copy conditions, embedded completion errors and
signed extension/fallback/gzip validation. Optional-mode and complete ambiguous
commit/cancellation/retry coverage remain gaps.

[ ] Stage 6: Filesystem state/ownership, reads and writes.
Inode, namespace, read/write/filesystem modules implemented with owner charges,
bounded admission, reserved flush progress and immutable dirty snapshots.
Rename refreshes copied ETags and gates descendant commits; partial directory
rename remains retryable. Cancelled multipart abort retains bounded admission
until cleanup, and shutdown drains it. Complete matrix/invalidation review remains.

[ ] Stage 7: Observable filesystem state-machine and concurrency tests.
Eighteen state/concurrency tests cover sparse/truncate/fresh readers, newer writes
during fsync, upload/copy/completion failure, partial directory rename, stale LIST,
replaced inode eviction, read-ahead coherence, full-budget progress and cancelled
multipart cleanup. Cancellation test failed on the bug and passed after the fix.
Full required failure matrix and broad sensitivity proof are not complete.

[ ] Stage 8: Fuser integration.
Owned replies, bounded task admission, write ownership, attrs/xattrs, directory
callbacks, mmap/RO handling, link/special helpers and shutdown are wired.
Actual mounts work. Complete callback/invalidations/failure verification remains.

[ ] Stage 9: Actual disposable mounted FUSE/S3 integration and client suite.
Both Rust and experimental Go completed full/Range/mmap, rsync full/delta,
symlink/mode/mtime/xattr/FIFO/fsyncdir, sparse/truncate, fresh backend durability,
rename-before-close and concurrent writer checks. Rust PUT-failure recovery passes;
Go remains EAGAIN through the 75-second recovery deadline. Both optional hardlink
probes return EIO. Runner exits 1; full compatibility remains unaccepted.

[ ] Stage 10: Paired performance workloads, runner and fail-closed evidence gate.
Runner, fresh client processes, raw accounting and eight evidence-gate cases
implemented. Full command attempted: zero runs, missing owned delegated cpu+memory
cgroup subtree. Go peak/steady heap time-series instrumentation also incomplete;
single end snapshots no longer masquerade as peaks. No measured improvement.

[ ] Stage 11: Final CI/documentation, source/license review and delivery.
CI and documentation describe the actual implementation and failed acceptance.
Transitive binary notices, full review and parent commit/push/draft PR remain.

## Verification and boundaries

- Formatting, warning-denying clippy, all 43 Rust tests (including a wrapper for
  eight Python gate tests), and cargo build passed after the cancellation fix.
- Mounted runner exited 1 with partial Rust/Go receipts and traceback preserved
  under ignored private results. Full benchmark command failed closed, zero runs.
- Earlier GET-condition and fallback-classification mutations failed as expected
  and were restored. New cancellation regression first failed with one orphaned
  upload, then passed after bounded abort/drain was implemented.
- No nested BMAD invocation, commit, push, PR, production credential access or
  replacement of an existing mount. Only owned disposable resources were changed.
- Compatibility and performance remain **UNACCEPTED**. This is not a complete port.

## 2026-10-26 delegated backend/config/main continuation (append-only)

- Added bounded IAM token refresh/authentication, profile-file selection and
  explicit-profile precedence, signature/region/anonymous discovery and public
  crate-backed SigV2, with no-detect and explicit-auth boundaries.
- Added signed requester/SSE-S3/KMS/SSE-C/ACL/storage/MIME/no-checksum options;
  finite HTTP/retry controls; explicit insecure-TLS streaming adapter with default
  certificate verification preserved. Disabled unbounded credential-process shell
  execution; explicit dependent profiles fail instead.
- Added prefix-confined paginated multipart expiry and conditional multipart
  clean-copy ownership/abort/drain. Main initializes detection/cleanup and drains
  known backend-copy cleanup before runtime teardown.
- Added charged, conditional one-attempt PATCH transport and full bounded XML
  validation. No enable/prefer/conflict flags are accepted without writer policy.
  Parent-owned filesystem/FUSE/read/write modules were not edited in this tranche.
- Corrected Go-compatible Unix MIME precedence/defaults using public `mime`
  parsing; rejected `mime_guess` policy mismatch with documented rationale and
  retained Go BSD association-data notices. MSRV is now 1.96.0 for public SigV2.
- Verification: cargo fmt check, warning-denying all-target/all-feature clippy,
  and all 65 Rust tests pass (including eight Python gate cases). Public Cargo
  metadata audit: 332 packages, zero non-public sources/missing license entries.
  MIME mutation failed with the wrong JavaScript type, then passed restored;
  wrong-root PATCH regression failed before complete-document validation, then
  wrong roots/trailing documents/DTD cases passed after the fix.
- Remaining: writer PATCH activation/generation/conflict recovery; multipart final
  size for exact cold-min-size policy; complete CLI/providers/actual-service and
  mounted matrix; binary-notice aggregation; delegated cgroup/performance evidence.
  No new mount/benchmark acceptance or performance improvement is claimed. No
  commit, push, PR, production operation or ambient-credential test was performed.

- Final backend follow-up: added live-listener IAM HTTP-error/redirect/oversize
  rejection coverage. Raising the cap admitted an oversized token and failed the
  forbidden-S3-request assertion; restoring 64 KiB passes. Final fmt/clippy/full
  test rerun passes **66 Rust tests**, including **19 backend-option tests**.
  Cargo metadata also confirms zero non-workspace path dependencies.

## 2026-10-26 writer/core follow-up (append-only)

- Preserved the parent's unsafe-code forbid, safe rustix identity and safe
  jemalloc changes. No unsafe code, main/tooling/CI changes, BMAD or VCS actions.
- Fixed Budget total capacity versus individual u32 acquisition counts. A 32 GiB
  configured filesystem admits small writes without preallocation; over-u32
  individual requests reject before conversion. The old limit test failed first;
  removing the new per-request guard also fails the wraparound assertion.
- Activated enable/drop-conflict/prefer-upload PATCH policies with fixed-size,
  part-boundary-split requests, intermediate remote progress, versioned retirement,
  metadata replacement, explicit conflict baseline/metadata merge, and safe retry
  after lost responses/cancellation. Newer writes survive conflict discard and
  successful older snapshots. Preferred new uploads use a two-part prefix without
  truncating an uncached existing object; unsupported PATCH falls back to upload.
- begin_upload carries final snapshot size for exact cold_min_size behavior.
  PUT/complete/COPY return known committed ETags. A same-sized foreign HEAD no
  longer silently acknowledges our dirty generation (reproduced before fix).
- Added core use-enomem, no-implicit-dir, no-dir-object and stat-cache-ttl behavior.
  Missing reference flags still reject instead of being accepted as no-ops.
- Reproduced and fixed source-child resurrection after a failed directory move,
  repeated drain-page copying and unreadable/unretryable file rename after an
  applied DELETE with a lost response. Pending prefixes are bounded/indexed;
  copied backing keys survive failure while live inode identities/dirty bytes stay
  intact. No complete remote object graph is accumulated.
- Verification target: 88 Rust tests, including 21 backend-option and 13 writer
  tests, under all-target/all-feature warning-denying clippy and formatting check.
  Public Cargo metadata: 336 packages, zero non-public/path sources or missing
  license declarations. Final actual results are reported with the handoff.
- Remaining acceptance: real-service PATCH/IAM/encryption, imported MPU layouts,
  complete configurable single/part-size and scheduling/retry CLI parity, FUSE
  reply-cache policy, full mounted compatibility, binary notices and reproducible
  comparative performance. Current fixed-part writer explicitly rejects objects
  needing larger parts for the 10,000-part limit; no full-port acceptance claimed.

- Final actual rerun: formatting check, warning-denying all-target/all-feature
  clippy, and **all 88 Rust tests passed**, with no ignored tests. The 336-package
  source/license metadata audit also passed. Backing-key support required small
  inode/read/namespace changes within core integration; safe rustix calls remain
  intact. No new mounted/performance/real-service acceptance run was performed.

- Conditional PATCH service errors are normalized to the reference retryable
  EBUSY at the writer boundary after baseline refresh. The modeled
  PreconditionFailed regression failed before normalization, then the final
   formatting/clippy/all-88-tests rerun passed again.

## 2026-10-26: configurable scheduling and effective-window continuation

- Implemented reference single-part clamping and tiered part-size parsing, up to
  10,000 parts, early configured-object EFBIG checks and schedule-aware PATCH
  boundaries. Virtual 60 GiB layout regression no longer hits the old fixed-part
  ceiling. Exact-size retryable Smithy bodies materialize at most 1 MiB chunks,
  so configured large PUTs/parts do not allocate an entire part or enlarge the
  reserved flush lane. Added public http-body adapter and crc32fast test reuse
  rationale; no custom HTTP/checksum/retry implementation.
- Added bounded per-file/global upload/copy scheduling and ordered multipart
  completion, with admission deadlines and owned abort/drain cleanup retaining
  its flusher permit. Real loopback SDK copy requires two concurrent parts;
  real 12 MiB PUT fixtures independently validate full bytes and CRC32 trailers
  as well as the no-checksum request.
- Removed hidden 1 MiB read-window and half-data-lane cache caps. Read-ahead chunks
  run concurrently after aggregate output/scratch/reply owner admission; cache
  lookup covers far offsets. A 100 MiB requested window, cache use over half the
  shared lane and cancellation-owner release are deterministic regressions.
  Filesystem::effective_budgets exposes serializable requested/effective window,
  cache/lane/usage/scheduler/object bounds for parent-owned receipt integration.
- Added bounded per-directory negative lookups, disabled by default. Reproduced
  and fixed stale negative publication over concurrent creation, clean deleted
  inode blocking recreation and markerless parents hiding dirty local children.
- Failure sensitivity observed for reinstated old window/cache/object limits,
  serial backend copy and omitted streamed dirty overlay; all faults restored.
- Final verification: cargo fmt --all -- --check; warning-denying all-target/
  all-feature clippy; all 102 Rust tests passed with none ignored, including the
  gate's eight Python evidence cases. Cargo metadata audit still has 336 packages,
  zero non-public/path sources and zero missing license declarations.
- Kept main.rs, benchmark/tooling/CI and safe rustix/jemalloc code untouched; no
  VCS actions or production operations. Remaining: parent receipt/FUSE cache
  integration, full CLI parity, actual-service/imported-layout acceptance, binary
  notices, full mounted compatibility and paired performance with an owned
  delegated cpu+memory cgroup. No whole-process memory/performance win claimed;
   overall compatibility/performance remains UNACCEPTED.

## 2026-10-26: core/backend inspection follow-up

- Reviewed all 13 supplied candidates against current callers, namespace/flush
  guards and public Go source. Fixed reproduced stale HEAD, ghost dirent,
  rmdir/create, second unlink, cancelled DELETE, known commit verification,
  timestamp, root mutation, requester-pays and metadata-property failures.
- The creation probe spanning parent rename/delete was already epoch-guarded;
  its test passed before changes. Added final locked parent ID/name/key/deleted/
  rename validation without claiming a reproduced narrower preemption race.
- Explicit Go distinctions: Go also emits file hints for non-directory dirents
  and omits requester-pays on write/delete methods. Standard LIST now resolves
  unknown concrete types through bounded selected-page lookups (safe fuser has
  no DT_UNKNOWN); extended metadata avoids extra HEADs. Requester-pays is signed
  on all request phases. These fixes are not claims of identical Go wire output.
- Added refresh_inode_if_expired for parent-owned getattr/fstat wiring and
  create_with_identity for request UID/GID only with enable-perms. Public Go does
  not derive userspace setgid-parent ownership. Root refresh is a no-op; implicit
  directories use LIST; dirty state and open/lookup refs survive refresh.
- Ambiguous/cancelled DELETEs retain bounded old-name ownership until unlink
  retry or reconcile_pending_deletes, called by flush-all/shutdown. Known rejected
  authorization/validation requests restore ordinary lookup. Dirty bytes are not
  retired on ambiguity and shutdown cannot resurrect the cancelled delete.
- Added 14 review regressions and three backend wire cases. Observed failing
  before fixes for confirmed races and omissions; timestamp and caller-identity
  faults were also deliberately reinstated and failed, then restored. The dirent
  test failed with RegularFile instead of NamedPipe/Symlink before resolution.
- Final check: formatting, all-target/all-feature warning-denying Clippy and all
  119 Rust tests passed, none ignored. No dependencies added; unsafe forbids and
  public-only provenance retained. No parent FUSE/main/tooling/CI edits, VCS, SSH,
  production or mounted/benchmark runs. Detailed evidence and API handoff are in
   docs/review-follow-up.md; overall acceptance remains UNACCEPTED.

## 2026-10-27: bounded metadata concurrency and cached-read FIFO pressure

- Read the parent's actual paired-smoke-20261026-v1 cold-metadata receipts:
  correctness true for both, Rust 678.7162 ops/s versus Go 5697.5527 ops/s in one
  diagnostic pair. This is pre-change evidence, not acceptance or a speedup claim.
- Replaced sequential selected-page type lookups with maintained FuturesUnordered
  and permits acquired before job creation from a filesystem-wide max-requests
  semaphore. Metadata/type/generation/epoch checks and sorted publication remain;
  no per-entry spawned semaphore waiters. Cancellation/error drops borrowed jobs
  synchronously. Effective budgets expose capacity and admitted job count.
- Observed the concurrency regression fail on serial code. A per-page-semaphore
  mutation failed the cross-call bound with four starts instead of two; restored
  the global permit pool. Gated tests verify overlap, exact file/symlink/FIFO types,
  error/cancellation owner release and retry, without measuring a performance win.
- Reproduced the cached-read FIFO cycle through public Filesystem reads, explicit
  polling and controlled reply ownership. Moved charged output admission before
  borrowing cache entries and release the temporary reservation before miss-window
  admission. The original path and a deliberate restoration both failed the
  progress deadline; corrected code passes with full byte assertions. No budgets,
  cache policy or windows were lowered, and pinned ownership remains charged.
- Final verification: format check, warning-denying all-target/all-feature Clippy
  and all 123 Rust tests passed, none ignored (includes the parent's lifecycle
  case). Dependency metadata remains 336 public packages with no path/non-public
  sources or missing license declarations; futures-util was already transitive.
- Only this bounded core/test/dependency/docs scope changed. No parent-owned
  FUSE/main/tooling/CI edits, VCS, SSH, production or mounted/benchmark runs.
  Fresh paired measurements are still required; detailed handoff is in
  docs/bounded-metadata-pressure.md. Overall acceptance remains UNACCEPTED.
