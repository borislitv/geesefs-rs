---
title: 'GeeseFS Linux/S3 Rust rewrite with measured memory and throughput gates'
type: 'feature'
ticket: ''
created: '2026-10-26'
status: 'in-progress'
baseline_revision: 'c4cf807aa8316e6aa5762270da54d9848cfb0c4e'
route: 'full'
route_source: 'auto'
risk: 'high'
review: 'thorough'
review_source: 'auto'
lenses_ran: ['blind-hunter', 'edge-case-hunter', 'verification-gap', 'intent-alignment']
review_loop_iteration: 0
followup_review_recommended: true
context: ['{project-root}/AGENTS.md', '{project-root}/docs/project-brief.md']
warnings: ['oversized', 'multiple-goals']
deferred: []
---

<intent-contract>

## Intent

**Problem:** The Go optimization experiment did not establish sustained lower
whole-reader memory. A Rust replacement must support the required Linux/S3
reader and writer contracts, consume less memory and operate faster under
equivalent workloads; a different language alone is not success.

**Approach:** Reuse existing FUSE, AWS, XML and runtime crates; implement the
filesystem state/ownership semantics and narrow Yandex extensions. Establish
independent mounted compatibility tests and paired Go/Rust performance gates
before claiming a replacement or improvement.

## Boundaries & Constraints

**Always:** Follow AGENTS.md and reuse-first. Search and document crates before
custom infrastructure. Preserve Linux/S3 reader and writer behavior, configurable
metadata, failure propagation, bounded ownership and fsync durability. Inventory
every Linux/S3 CLI option rather than silently accepting ignored flags. Required
Linux/S3 behavior includes optional configuration paths in the reference: IAM,
encryption/signing modes, PATCH, permissions/specials and link-emulation options
must either be implemented/tested or explicitly leave overall compatibility
unaccepted. Correctness precedes unchanged-budget memory/throughput comparison.
The repository is public; use only disposable test credentials/data. Retain
source/license attribution and raw benchmark/error evidence.

**Never:** Handwrite FUSE, S3 signing, HTTP/TLS, XML parsing or an executor where
maintained crates provide them. Do not substitute a read-only prototype or Go
wrapper, collect whole large files/directories, lower budgets or durability,
force Go GC, conceal unsupported flags or mark unmeasured performance passed.
Do not change production or use its credentials. macOS/Windows, Azure/GCS and
distributed cluster ownership are explicitly inventoried outside this initial
Linux/S3 target, not claimed compatible.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
| --- | --- | --- | --- |
| Namespace | Explicit/implicit directories, empty markers, paged lexical keys | Complete confined lookup/readdir with stable live inode identities | Malformed/incomplete pages are not installed; only specific unsupported-extension errors allow fallback |
| Attributes | ext-v1 metadata, percent-escaped values, symlinks and configured keys | Compatible mtime/mode/xattrs/readlink; known-empty differs from unknown | Invalid encodings follow documented reference behavior; access/backend errors stay errors |
| Reads | Concurrent unaligned Range, EOF, short body, dirty overlap | Exact stored bytes, adaptive bounded reads, no dirty overwrite | Retry only recoverable failures; close bodies; do not decode stored gzip |
| Writes | Sparse extension, random overwrites, truncate, concurrent fsync | Read-after-write and zero holes; immutable flush generations | Failed upload/copy/completion never acknowledges durability or clears newer dirty data |
| Namespace mutation | Rename before close, delete during upload, stale LIST | Correct visible namespace and copy/delete ordering; no stale resurrection | Preserve retryable failed mutation; do not claim atomic S3 rename or writable-unlinked POSIX behavior |
| Pressure/shutdown | Full dirty budget, pinned reads, cancelled requests | Bounded admission and flushing progress; replies complete once | Surface cancellation/resource/backend failures, release ownership and clean only owned test resources |

</intent-contract>

## Code Map

- New repository currently contains bootstrap/config only. `AGENTS.md` and
  `docs/project-brief.md` are authoritative. This is a full route because the
  implementation spans substantially more than 100 lines and five files.
- Go reference is the clean checkout at
  `/tmp/opencode/geesefs-runtime-research-20261025t1916`, revision
  `5c0884c9e8799bbab93e2d4102fdc15e9c079e0d`. Use a configurable reference path
  in tooling, not this absolute path in implementation. Packaged upstream
  revision `80c214ef14f620d93407454fc76ca01dd71fa25f` is the second comparison;
  verify availability/build provenance instead of substituting an unknown binary.
- `core/cfg/{flags.go,conf_s3.go}`, `main.go`: CLI/fstab normalization, credentials,
  region/endpoint/addressing, metadata keys and optional S3 modes. Derive the
  compatibility matrix directly from NewApp/PopulateFlags, not flag-name guesses.
- `core/backend_s3.go` ListBlobs/GetBlob/UploadPartCopy/TryIAM and
  `core/ycs3ext/{client,list_decode,convert,types}.go`: ext-v1 metadata and tokens,
  V1 marker handling, exact unsupported fallback, stored-body identity, source
  conditions, IAM. Reuse their contract fixtures, not their SDK implementation.
- `core/dir.go`: implicit dirs, slurp/flat pagination and lexical cuts,
  Seek/LookUpInodeMaybeDir, renameRecursive/SendDelete and tombstones.
  `core/goofys.go` in-flight listings prevent stale namespace resurrection.
- `core/handles.go`: metadata escaping/casing, nil-versus-empty, symlink target,
  POSIX mode conversion, cloud/user xattrs. Default special key is
  `--symlink-target`; mtime is integer Unix seconds, not a new Rust schema.
- `core/{file,buffer_list,buffer_pool}.go`: bounded byte ownership, dirty extents,
  versioned flush, sparse resize, multipart-copy, SyncFile and completion errors.
  Small Arc/Bytes slices must not repeat Go backing-allocation retention.
- Regression sources: `core/{goofys_test,goofys_fs_test,goofys_unix_test,
  file_sync_test,slurp_refresh_test,negative_lookup_test,
  read_buffer_ownership_test,backend_s3_compression_test,backend_s3_copy_test,
  backend_s3_iam_test}.go`, `core/ycs3ext/list_decode_test.go`.
  `test/run-xfstests.sh` covers selected generic cases only; do not run its
  privileged installer unchanged or claim an all-xfstests pass.
- Dependency research verified `fuser 0.18.0` owned replies/Send+Sync/thread
  configuration, `aws-sdk-s3 1.155.0` standard streaming/conditional operations
  and multipart-copy, `aws-config 1.13.0`, `quick-xml 0.42.0`, Tokio 1.54.1,
  bytes 1.12.1 and Moka 0.12.16. Rust 1.99.1 satisfies declared MSRVs; complete
  dependency build remains to be verified. Primary APIs: docs.rs/crate/fuser/0.18.0,
  docs.rs/crate/aws-sdk-s3/1.155.0, docs.rs/quick-xml/0.42.0.
- Existing Mountpoint lacks required ordinary-S3 rename/symlink/xattr and write
  semantics. oxfs flat mode buffers/clones whole files; its database-backed mode
  changes object layout. Neither is a compatible implementation base.

## Tasks & Acceptance

**Execution:**

- [ ] `docs/dependency-decisions.md`, `docs/compatibility.md` -- record researched
  crates/versions/licenses/API links and every Linux/S3 reference capability;
  select existing FUSE/SDK/runtime/cache/XML infrastructure before source code.
  Use public upstream sources/crates only: no copied, translated, vendored or
  linked internal Arcadia libraries, generated code or internal dependencies.
  Audit manifests/lockfiles/generated artifacts/CI and record public provenance.
- [ ] `Cargo.toml`, `Cargo.lock`, `src/{lib,main,config,error}.rs` -- create the
  Rust binary/library and typed CLI/fstab/options/errors; add existing crates
  only for demonstrated roles, with supported authentication/configuration.
- [ ] `tests/config.rs`, `tests/metadata.rs` -- exercise actual CLI, metadata
  encoding, defaults and rejected/unsupported options against reference fixtures.
- [ ] `src/backend/{mod,s3,extended_list,metadata}.rs` -- integrate one AWS SDK
  standard client, streaming/conditional Range and multipart-copy/lifecycle;
  implement only the ext-v1 schema/operation and metadata mapping adapters.
  Prefer SDK customization; where its response model drops extended fields,
  reuse aws-sigv4 plus existing HTTP/XML crates for that extension alone. Keep
  credential source, error mapping, retry/cancellation and page/parse bounds explicit.
- [ ] `tests/backend_contract.rs`, `tests/fixtures/` -- validate actual HTTP
  requests, XML/percent decoding, gzip/EOF integrity, tokens, unsupported fallback,
  short reads, error bodies, failed multipart completion and source conditions.
- [ ] `src/{budget,cache,inode,namespace,read,write,filesystem}.rs` -- implement
  compact stable inode identities/forget ownership, weighted retained caches and
  separately charged in-flight/pinned allocations, bounded dispatch and reserved
  flush progress; implement versioned dirty extents, sparse/truncate/part-copy,
  read-after-write, async mutation and fsync generation barriers without global
  network-held locks or whole-object buffering.
- [ ] `tests/state_machine.rs`, `tests/concurrency.rs` -- test observable read/
  write/truncate/rename/delete/fsync behavior using faithful storage boundaries,
  deterministic concurrency and failure sensitivity; prove progress at full budget.
- [ ] `src/fuse.rs` -- integrate fuser callbacks, owned exactly-once replies,
  bounded admission before detached work, borrowed-write ownership, forget,
  invalidations, xattrs, fallocate, fsync/fsyncdir, mmap/RO and orderly shutdown.
  Do not write another FUSE protocol layer.
- [ ] `tests/mounted.rs`, `scripts/integration.py` -- run actual disposable FUSE/S3
  integration: full/delta rsync, nginx-style full/Range reads, symlink/metadata,
  writer/fsync failure and concurrency cases; explicitly fail requested mounted
  verification when unavailable rather than silently skipping it.
- [ ] `bench/{workloads.json,compare.py}`, `tests/performance_gate.rs` -- implement
  paired baseline/Rust runs with identical data generations/cache budgets/CPU,
  cold/warm states, at least five interleaved repetitions, independent checksums,
  metadata traversal and sequential/random/concurrent read plus writer/fsync
  workloads. Record peak/steady RSS, allocator and cgroup/file-cache accounting,
  throughput, p50/p95/p99 and variance; a gate cannot pass on missing results,
  changed budgets, reduced compatibility or an unverified baseline.
- [ ] `.github/workflows/ci.yml`, `README.md`, `docs/{testing,performance}.md` --
  build/lint/test, document mounted/benchmark requirements and real results/gaps;
  preserve licenses. Parent owns final source review, commit/push and draft PR.

**Acceptance Criteria:**

- Given the identified Go Linux/S3 contract and a disposable equivalent backend,
  when clients mount and exercise both implementations, then independent complete
  bytes, metadata, directory traversal and intended errors match across the matrix;
  unsupported target features leave the overall compatibility gate unaccepted.
- Given a concurrent writer and failed upload/copy/completion, when fsync returns,
  then success implies the intended generation is remotely durable, failures are
  visible and newer writes remain pending; a fresh reader verifies persisted bytes.
- Given a working set exceeding unchanged caches and full outstanding capacity,
  when concurrent client reads/writes complete, then bytes remain correct,
  bounded ownership and flushing progress hold without deadlock or hidden overage.
- Given the same verified Go/Rust workloads, data, limits and cold/warm states,
  when repeated paired comparisons complete, then Rust demonstrates lower memory
  and faster operation with reported variation and no concealed tail-latency,
  correctness or durability regression; otherwise performance remains unaccepted.
- Given a consumer of this public repository, when it follows documented commands,
  then it can build and repeat tests/benchmarks without production credentials,
  private paths or unreported dependencies and see exact remaining acceptance gaps.

## Implementation Notes

- User requires no unsafe code: Cargo forbids it for all package targets, with
  explicit library/binary root forbids as well. Safe public rustix uid/gid APIs
  replace direct libc calls. Jemalloc and its safe control crate replace unsafe
  mallinfo2 sampling; benchmark records identify the allocator and retain separate
  RSS/cgroup accounting. Dependencies' implementations remain their public crates.

- Updated user instruction makes BMAD optional: retain this plan as the execution
  and verification record, but do not block useful independent implementation on
  framework ceremony. All commands/subprocesses/external operations require finite
  timeouts; SSH is noninteractive with deadlines and no user questions.

- Before implementation the user added a mandatory public-source boundary: no
  internal Arcadia libraries or code may be exported without explicit approval.
  AGENTS.md and the brief now enforce that boundary and GeeseFS-style direct
  operations, explicit ownership and minimal idiomatic Rust abstraction.
- The parent has already rendered and executed BMAD steps 1 and 2 and delegated
  step 3 implementation. The implementer must execute this plan, not invoke a
  nested build-auto workflow or wait for planning. The first delegated attempt
  made no changes because it tried a nested renderer with an incomplete PATH.
  Installed tools are available with
  `export PATH="$HOME/.local/share/geesefs-rs-tools/bin:$HOME/.cargo/bin:$PATH"`.
  Use the project directory for all Cargo commands. Source implementation and
  verification are the delegate's responsibility; workflow orchestration,
  final review, commits and PR delivery remain with the parent.

## Plan Change Log

## Review Triage Log

- Four independent inspection lenses completed. Raw dispositions: 17 high,
  10 medium, 1 false; deduplicated into 21 patch groups, all addressed. No human
  decision or deferred change-caused defect. The create-probe finding was rejected
  as stated because its deterministic case was already epoch-guarded; caller
  ownership was corrected without inventing setgid inheritance. Details and
  regression evidence are in `docs/review-summary.md` and
  `docs/review-follow-up.md`. This is not compatibility/performance acceptance.

## Design Notes

fuser owns the wire protocol and replies; a bounded scheduler owns asynchronous
work. One SDK client owns ordinary S3 operations. Ext-v1 is a narrow documented
gap: standard SDK listing models discard its per-object metadata. Reuse a
parser/signing/transport rather than writing those primitives. Moka is a
best-effort cache policy, not a hard memory limiter: allocation-owner permits
remain charged while pinned, queued or in flight, including flush replay.

Keep dirty-range versions until remote completion; part acknowledgments alone
do not establish durability. S3 rename is copy/delete, not atomic. Default
flush/close behavior is distinct from fsync. Preserve logical compatibility,
not Go's eager whole-subtree rename representation or avoidable retained graphs.

## Verification

**Commands:**

- `cargo fmt --all -- --check` -- no formatting drift.
- `cargo clippy --all-targets --all-features -- -D warnings` -- no warnings.
- `cargo test --all-targets --all-features` -- deterministic behavior and contract
  checks pass; baseline/mutant sensitivity demonstrates relevant failures.
- `python3 scripts/integration.py --go-reference "$GEESEFS_GO_REFERENCE"` -- actual
  owned FUSE/S3 client interoperability and failure checks pass, with ordinary cleanup.
- `python3 bench/compare.py --go-reference "$GEESEFS_GO_REFERENCE" --workloads bench/workloads.json --output benchmark-results/private` -- all unchanged-budget comparisons
  complete with independent correctness, raw statistics and an honest gate verdict.
- `git diff --check` -- no whitespace issues; separately inspect final source,
  dependency/lockfile/licenses and credential/public-artifact boundaries.

## Code Review

### 2026-10-26

Code review complete: 0 decision-needed, 21 patch, 0 defer, 1 rejected raw finding.

- [x] [Review][Patch] F1: synchronous shutdown timer and public lifecycle regression (`src/fuse.rs`).
- [x] [Review][Patch] F2: delayed HEAD generation publication (`src/filesystem.rs`).
- [x] [Review][Patch] F3: known commit before fallible verification (`src/write.rs`).
- [x] [Review][Patch] F4: cancelled pending-delete reconciliation (`src/filesystem.rs`).
- [x] [Review][Patch] F5: directory removal prefix exclusion (`src/filesystem.rs`).
- [x] [Review][Patch] F6: old unlink waiter identity (`src/filesystem.rs`).
- [x] [Review][Patch] F7: fresh clean getattr/fstat (`src/fuse.rs`).
- [x] [Review][Patch] F8: expired directory ghosts (`src/filesystem.rs`).
- [x] [Review][Patch] F9: caller creation ownership (`src/fuse.rs`).
- [x] [Review][Patch] F10: stable S3 LastModified (`src/inode.rs`).
- [x] [Review][Patch] F11: HTTP property preservation (`src/backend/s3.rs`).
- [x] [Review][Patch] F12: requester-pays write/copy/delete (`src/backend/options.rs`).
- [x] [Review][Patch] F13: position-distinguishing integrity oracle (`scripts/integration.py`).
- [x] [Review][Patch] F14: bounded resolved directory types (`src/filesystem.rs`).
- [x] [Review][Patch] F15: synthetic root mutation policy (`src/filesystem.rs`).
- [x] [Review][Patch] F16: applied/effective benchmark budgets (`bench/compare.py`).
- [x] [Review][Patch] F17: in-cgroup warmup/accounting (`scripts/bench_worker.py`).
- [x] [Review][Patch] F18: overall benchmark deadline (`bench/compare.py`).
- [x] [Review][Patch] F19: bounded owned daemon cleanup (`scripts/integration.py`).
- [x] [Review][Patch] F20: isolated correctness evidence gate (`bench/test_compare.py`).
- [x] [Review][Patch] F21: actual mounted O_SYNC failure (`scripts/integration.py`).

Rejected: false R13 as stated, because the existing epoch guard already rejects
the demonstrated parent-change-spanning probe. Narrower publication was hardened
without claiming a failing deterministic reproduction. See review summary for
the refuted setgid subclaim and corrected O_SYNC fixture boundary.
