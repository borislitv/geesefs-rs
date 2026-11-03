# Testing and verification boundary

**Experimental read/write filesystem; compatibility and performance unaccepted.**
Build and test in isolated fixtures with disposable data. The operator also
authorizes the OpenCode VM for the 2026-10-31 native experiments. Never use
served mounts or production credentials. Use finite subprocess/operation deadlines.

## Independent review regressions — 2026-11-03

Four targeted suites cover the corrected review failures:

```bash
cargo test --locked --test write_review_regressions --test namespace_review_regressions --test startup_review_regressions --test fuse_deadline_review
```

The 24 cases include applied PUT/completion errors through the real SDK and
loopback HTTP, cancellation/newer writes/truncate boundaries, stale metadata
responses at the same content ETag, both pending-rename parents, implicit marker
discovery, absence of startup HTTP for invalid budgets, and actual fuser wire
replies with deadline/early-publication ownership. Baseline failures were observed
before correction; an early-publication control was additionally fault-injected.
These fixtures do not require a kernel mount or production credentials.

Both allocator suites and explicit native-mount tests remain separate checks.
Scoped mutation/complexity results do not imply repository-wide coverage or a
complete CRAP threshold pass.

## Prometheus observations — 2026-11-03

The configuration regression initially rejects `--metrics-listen`; after adding
the listener, tests verify numeric/IPv6 parsing and the disabled default. Seven
metrics regressions cover cold/warm/EOF accounting, backend failure, unchanged
admission and backend-call counts during scrapes, retained versus pinned bytes,
entry/byte pressure, namespace eviction/failure, real HTTP routes and contention,
bind errors, partial-header deadline and shutdown. Both allocator suites pass
262 Rust cases with thirteen native cases ignored by default. The benchmark
Python gate requires the project virtual environment; an initial system-Python
run failed on missing `psutil`, not a runtime regression.

Formatting, all-target/all-feature warning-denying Clippy, default release build
and the 342-package public-source audit pass. Fresh default/System LLVM coverage
and native Python coverage keep new metrics modules/fixture below CRAP 10 (worst
8.4); the broader filesystem CRAP gate remains open, including existing high-CC
read/lookup/rename methods. Duplication screening at 0.8 finds no candidates in
the changed runtime/fixture scope. Eleven selected mutations in HTTP/allocator
observations and cache/namespace counters are killed; this is not repository-wide
mutation coverage. Initial report-path/unsupported-report-option failures were
tool setup errors, not a quality pass.

The explicit real-binary fixture uses disposable local S3 and private mount/network
namespaces. It verifies independent bytes, a cold miss then a direct-I/O warm hit,
OpenMetrics output, existing stats-file JSON and normal unmount/listener exit:

```bash
timeout --kill-after=5s 130s unshare --mount --net --propagation private \
  python3 scripts/metrics_smoke.py --binary "$GEESEFS_RUST_BINARY" \
  --output "$GEESEFS_METRICS_RESULTS"
```

Run it in an owned resource-limited unit as described below. Results are local
fixture checks, not a new serving deployment, comparative performance result or
cache-size recommendation. See the [metric semantics and decision criteria](metrics.md).

## Cross-UID mount access — 2026-11-02

The public `scripts/mount_access.py` fixture requires explicit root, private mount
and network namespaces, a disposable local backend and a finite deadline. For
example, inside an owned resource-limited test unit:

```bash
timeout --kill-after=5s 130s unshare --mount --net --propagation private \
  python3 scripts/mount_access.py --binary "$GEESEFS_RUST_BINARY" \
  --output "$GEESEFS_ACCESS_RESULTS"
```

The output directory must not already exist. A child with UID/GID 65534 and no
supplementary groups reads via an inherited mount-root descriptor, so private
fixture ancestors cannot explain `EACCES`. All three real-FUSE cases pass:

- Default owner-only session: readable mode-0644 file is denied to the other UID.
- Explicit `ro,allow_other`: the other UID reads the independently specified bytes.
- Explicit `allow_other` plus mode 0600: the other UID is still denied by kernel
  permissions. The mount also retains `nodev,nosuid,noexec,default_permissions`.

The normalization test first rejects `allow_other` on the original parser. After
accepting it, an entrypoint fault that omits `SessionACL::All` makes the native
readable-file assertion fail with `EACCES`; restoring the wiring passes. A prior
interpreter-path setup failure is retained separately, not counted as fault
sensitivity. All positive mounts unmount normally and daemons/scopes exit.

For this source, both allocator suites pass 253 cases with 13 native tests ignored
by default; the explicit access fixture supplies the three cases above. Formatting,
all-target/all-feature Clippy, release build and all 99 benchmark Python tests pass.
Fresh LLVM/Python profiles give affected CRAP scores at most 8.3; affected
duplication at 0.8 is absent. The broader-file CRAP gate still fails. The automated
mutation scan found zero eligible sites and supplies no mutation score; the native
omitted-ACL fault is the failure-sensitivity evidence. These local checks do not
establish a new performance result or production deployment.

## Listing configuration and reader-access diagnosis — 2026-11-02

The [latest diagnostic checkpoint](performance-listing-diagnosis.md) uses the
unchanged frozen release in 40 independently mounted runs: default Rust,
explicit negative64, its identical-config A/A, Go 0.43.9 and its A/A. All byte,
listing, rsync and final generation checks pass; resource events, task/cgroup
exit and mount cleanup are verified. Separate traced mounts identify repeated
miss requests, then their absence with explicit caching. Traced timing is not
used for performance screening. New Go RSS controls are biased, not a memory win.

A real-FUSE external-create probe demonstrates the option's natural TTL delay.
Omitting the cache option makes its visibility assertion fail; restoring the
correct configuration passes again. That frozen executable's option probes reject `allow_other`,
`dev`, `suid` and Go's `--pprof`; a separate mount returns correct bytes to its
owner but `EACCES` to UID 65534 via an inherited mount-root descriptor. These are
observed integration gaps, not passing full serving compatibility. Read-only
production memory observations preserve reader/mount identity but cannot assign
live-heap owners without a working existing profiling listener.

The change is documentation only. Full Rust/allocator and Python suites retain
their earlier recorded scope; they are not rerun or relabelled as new coverage.
Private probes do not close the public generic matrix or broad quality gaps.

## Statistical gate and adapter quality — 2026-11-02

`bench/paired.py` and `bench/compare.py` now require eight mount pairs, a real
same-executable control arm and predeclared practical margins; 17 paired-assessment
cases plus three-arm scheduling, margin, zero/overflow and writer-file cases pass
with the whole 99-case Python suite. Mutation of `bench/paired.py` kills 40 of 42
sites. A public `assess` boundary case distinguishes an interval touching the loss
limit from a demonstrated regression, killing the previously surviving `> -> >=`
mutation. The two bootstrap percentile-rank mutations remain undetected; they are
an open test gap, not proven equivalent or impossible to distinguish.

Both allocator LLVM runs pass **265 Rust cases**, including thirteen native
directory contracts (new receive-thread liveness and xattr error contracts) and
the public `errno` mapping. CLI child profiles are now merged into both reports.
`crapper --threshold 10 src/fuse.rs` **passes** (worst `write` 9.5): `errno` and
`fallocate` delegate to `backend_errno`/`allocate`, `getxattr` is covered natively.
`compare.py` orchestration functions that need real mounts (`_measure_run`,
`_run_case`, `main` and helpers) still exceed CRAP 10 at 0-50% unit coverage;
the updated three-arm native matrix is still unverified end to end. Earlier
integration runs do not establish coverage of the changed orchestration. Clippy,
formatting and `dryer --threshold 0.8` pass in the recorded review scope.

## Actual LXC read-only preflight — 2026-11-02

The exact current jemalloc release passes available rw/ro nginx, rsync and failure
contracts on the development VM. It also passes an isolated read-only fixture on
the authorized LXC kernel `5.15.179-15.2mds`, with the installed nginx 1.24.0 and
rsync 3.2.7. Buffered and direct-I/O mounts pass byte checks, four-client unaligned
Range/EOF, implicit listings, `EROFS`, nginx full/Range/416 and rsync full/delta.
Buffered shared mmap passes; direct-I/O shared mmap returns `ENODEV` on this kernel.
A changed backend byte causes the expected SHA256 assertion failure; restoring
the fixture passes. Both success and failure clean up normally.

The fixture uses an owned private mount namespace, temporary paths, a 1 GiB unit
with a two-CPU quota, and a disposable backend reached through a temporary SSH forward.
It does not test the served configuration, golist, real S3 or sustained load.
See [target evidence and remaining measurements](performance-target-preflight.md).

## Loaded read controls and mount cleanup — 2026-11-02

The [read-shape checkpoint](performance-read-shape.md) completes a private
40-mount nginx/golist/rsync design with actual Go A/A controls, independent bytes
and final backend generation checks. A separate real-S3 LXC attempt stops on a
dependency timeout with five complete blocks; it does not meet the eight-pair
gate. The guard's twelve local-wire cases and ten relevant faults verify request,
byte, concurrency, rate, path, credential and redirect boundaries. Full/partial
body corruption and removed-oracle controls verify the native client byte checks.
These private runners do not verify the public generic matrix end to end.

A host audit finds three stale disposable mounts. Two retain kernel-blocked
threads under zombie leaders from earlier failed rename tests; `df` waits in
FUSE getattr. The idle mount unmounts normally, but the other two return busy.
After ownership is proved, only those two disposable connections are aborted
and ordinarily unmounted; the tasks exit and bounded `df` passes again.
Unrelated FUSE mounts are unchanged. This corrects the earlier cleanup conclusion:
inactive/failed units and `MainPID=0` do not establish absence of live threads.

Run native tests in a private mount namespace or a dedicated unit with
`PrivateMounts=yes`, with finite run/stop deadlines. Verify actual remaining
processes/cgroups and mount identities after failures, including zombie leaders
with live threads. Never infer ownership from a connection number or abort all
FUSE connections; preserve unrelated/served mounts. A private namespace prevents
host mount propagation, but does not replace verifying that blocked tasks exit.

## Retained rename correction — 2026-11-02

This block records the earlier rename-only source and verification scope.

The reduced source retains only removal of redundant post-rename entry
notifications; receive-loop CLI and first-poll read trials are discarded.
Both jemalloc and all-feature System-allocator LLVM runs pass **261 Rust cases**,
including eleven native directory contracts, and 75 benchmark Python cases.
The native suite also passes separately with two and four receive loops through
the test-only `GEESEFS_TEST_FUSE_THREADS` environment setting. The new four-client
rename/list/read/write regression hangs on the old notification path and passes
after correction, including full dirty-close bytes after unmount.

Matching default/System instrumented executables and the exact frozen default
release pass available rw/ro, literal nginx, rsync, Range/EOF/mmap and injected
fsync failure/recovery contracts. These are disposable Linux fixtures, not the
actual LXC/nginx/rsync/golist deployment or positive LINK/service acceptance.
Formatting, warning-denying all-target/all-feature Clippy and public-source audit
pass. All 85 frozen inputs match the committed source; only `src/fuse.rs` and
`tests/fuse_directory.rs` differ from the observation-age baseline.

Whole affected-file `crapper --use-existing-coverage --threshold 10 src/fuse.rs`
still **fails**: `errno` 23.8, `getxattr` 14.2 and `fallocate` 11.0. Adapter `rename`
is CC6/79.2% coverage/CRAP6.3 in both matching native reports. `dryer --threshold
0.8` finds no duplicates. Broad mutation remains open; historical restored-fault
hangs are failure-sensitivity evidence, not a completed new mutation campaign.
Two default-allocator CLI child profiles are preserved separately but were not
merged after switching allocators; do not claim complete child-entrypoint coverage.
Original setup/report-option failures and corrected completion receipts remain
private. Successful test/build commands do not turn these quality gaps green.

## Library and contract checks

Use Rust 1.96.0+ with rustfmt/clippy, Python 3.12+ and the public dependencies from
`requirements-fixture.txt`. The idle flush-cache policy passes 211 Rust cases,
formatting, all-target/all-feature warning-denying Clippy and the public-source
audit in Kubernetes. The unchanged runner also passes 57 Python cases.
Default and System-allocator ThinLTO builds pass; the default release passes
30 backend-contract, cache-loan and read-view cases.
These checks are not full reference parity or production acceptance.

The 2026-10-31 admission/directory correction passes 218 Rust cases with all
targets/features, formatting, warning-denying all-target/all-feature Clippy, the
public-source audit and a default release build on the VM using Rust 1.99.1.
The gated admission and unrelated-directory regressions each fail before their
fix, then pass. A further directory-removal race first exposes the missing final
binding check. Directory tests also retain TTL, inode eviction, parent/descendant
rename and in-flight response coverage. The Python fixture dependencies must be
on the selected interpreter's environment; setup failures are not behavioral RED.

Installed `mutator` kills all three selected epoch/binding-guard faults using the
directory public-interface suite. This is a three-site check, not whole-file or
repository mutation coverage. `dryer --threshold 0.8` finds no duplicate candidates
in the three changed runtime files. `crapper --no-coverage` reports complexity
only: coverage and CRAP remain N/A, and large existing methods remain open work.

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
python3 scripts/audit_public_sources.py
```

Public-interface tests cover SDK loopback HTTP/TLS, conditional ranges, checksum
and EOF barriers, metadata/configuration, generation races, sparse/truncate writes,
multipart/PATCH recovery, bounded admission and cancellation. Gated faithful
storage fakes and channels control concurrency without sleep-based interleavings.
Expected content, ranges and observable side effects are independent of production
algorithms. Each bug fix needs an observed behavior failure before the correction;
selected faults/mutations verify sensitivity and must be restored before rerunning.
Broader mutation coverage and large-method quality findings remain open.

The integrated freshness/handle correction also runs nine explicitly requested
native tests in `tests/fuse_directory.rs`; they fail if mounting is unavailable.
They cover PLUS paging/refcounts/error rollback, real kernel TTL, handle pressure,
marker attrs, side-effect-free unsupported hardlinks, writer metadata/xattrs,
fallocate/truncate/rename/removal and durable close. Run separately on an authorized
Linux fixture:

```bash
timeout --kill-after=10s 180s unshare --mount --propagation private \
  cargo test --locked --test fuse_directory -- --ignored --test-threads=1
python3 -m unittest discover -s bench -p 'test_*.py'
```

Both default-jemalloc and all-feature System-allocator LLVM runs pass 251 Rust
cases, including those nine native mounts. Separate instrumented executables also
pass read/write and read-only rsync/aiohttp/nginx/failure contracts, validate the
reported allocator and effective budgets, and exit normally. Jemalloc page gauges
are checked through the real statistics file; this is not performance timing.
The exact frozen default release separately passes the same available rw/ro
contracts, including literal nginx, rsync, injected fsync recovery and EROFS;
213+3 real allocator/page/budget observations and both daemon exits are checked.
Child-process profiles are preserved and merged only with matching source/build.
Python branch coverage covers 75 bench tests; the separate fixture suite passes
14 tests. CLI receipts, malformed schema/budgets, actual worker protocol and
checksum/failure-sensitive workload tests supplement pure gate-policy tests.
Automated mutation kills 24/24 Go-heap and 30/30 shared-metrics sites, including
exactly two samples and zero values. This is not repository-wide mutation coverage.
Affected runtime/accounting duplication >= 0.8 is absent. With matched native
entrypoint coverage, mount options and serving have CRAP 9 and 8; the extracted
directory pipeline, standard LIST decoding and benchmark gate have CRAP 6, 8
and 7 respectively. Whole affected-file CRAP still exceeds 10: existing rename
is 62.9, and uncovered matrix orchestration reaches 56. These remain failures,
not excluded files or a relaxed threshold. A successful release build is not
coverage or full compatibility acceptance.

SDK loopback tests retain malformed/confinement/order checks, V1 prefix-only
continuation, exact 1,000-entry bounds and opaque V2/extended tokens. Controlled
faults in each boundary trigger the intended assertions and are restored before
the passing suite. Native 8,192-name traversal requires 65 bounded LIST calls,
including EOF, without larger directory buffers or inaccurate file types.

Key read regressions live in `tests/read_windows.rs`, `read_reply.rs`,
`read_refill.rs`, `read_stream_history.rs`, `cache_loans.rs`, `receive_ownership.rs` and
`backend_contract.rs`. Keep FIFO progress, full backing charges, If-Match,
generation, checksum, cancellation and caller-owned task lifetimes intact.
The loan tests verify cache reuse without a higher total, owned reply isolation,
writer/multipart progress, concurrent uploads, cancellation and failed-fsync retry.
Five deliberate semantic faults trigger the intended assertions, including the
history-reset regression after explicit independent cache pressure. These checks
are not a whole-repository mutation or coverage/CRAP result.

## Disposable mounted verification

The fixture needs Linux `/dev/fuse`, `fusermount3`, rsync, Go, Python dependencies
from `requirements-test.txt`, a built Rust executable and the pinned public Go
checkout:

```bash
cargo build --locked
timeout --kill-after=30s 1200s python3 scripts/integration.py \
  --go-reference "$GEESEFS_GO_REFERENCE"
```

By default the runner owns Moto, a unique bucket, credentials and mounts. An
external backend requires explicit disposable-test opt-in and dedicated test
credentials. Ambient AWS credentials are not reused. Logs and partial failures
belong in ignored `benchmark-results/private/`; cleanup is confined to ownership.

Actual mounts exercise full/unaligned Range/EOF, mmap, implicit directories,
aiohttp full/Range, rsync, symlinks, mode/mtime, xattrs, FIFO, fsyncdir,
sparse/truncate, fresh-reader durability, rename and concurrent writer/fsync.
Rust injected upload/fsync failure propagation and recovery passed; the reference
Go open-file recovery remained EAGAIN through its deadline. Hardlink-as-symlink
formerly returned EIO after mutation. Corrected Rust returns EOPNOTSUPP without
mutation, so the required hardlink compatibility probe remains failed.

Actual-service IAM/SSE/signing/PATCH, imported multipart layouts, full CLI parity
and broader durability cases remain open. A separate native fixture checks literal
nginx 1.28.3 HEAD/full checksum, unaligned/suffix/short-EOF 206 and unsatisfiable 416
responses through all three mounted filesystems. No all-xfstests pass is claimed.
CI covers deterministic checks, not mounted or
performance acceptance. See [compatibility](compatibility.md) for the inventory.

The cache-policy release separately completes 120 healthy-warm four-binary records
with 30 frozen generations verified, then 60 persistent mixed cycles per binary
(1,440 checked passes). Independent raw-series analysis verifies fixed work,
unchanged budgets and quota/memory counters. Rare successful 30-second Rust reads,
other adverse metrics and unisolated observer cost keep performance unaccepted.
These healthy runs do not close the known mounted compatibility/failure gaps.

The final admission/directory binary separately completes 1,440 fixed-work and
720 equal-offered-rate passes on the VM, with all eight final full-byte/generation
checks passing. Owned mounts, clients, daemons and backends stop successfully.
The paced control has no missed cycles but retains a late-run metadata-tail
regression and cgroup-memory losses. Both raw-series summaries verify equivalent
work and unchanged budgets; all 25 compiled inputs and four executable hashes are
checked against retained provenance. These comparisons neither repeat the same
design independently nor pass compatibility/performance acceptance.

The later integrated checkpoint independently verifies 240 isolated healthy
cold/warm records, 60 generation/full-byte groups and 2,880 mixed phases with
16 further checks. The original isolated controllers retain backend-startup and
final-bookkeeping errors; independent complete-grid validation does not relabel
them. The mixed controls complete, preserve recorded interval counters and stop
their owned processes/mounts. Short isolated allocator/late windows remain missing,
and throughput/memory nonwins remain acceptance failures; see the
[latest measurements](performance-integrated.md).
