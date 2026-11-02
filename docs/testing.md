# Testing and verification boundary

**Experimental read/write filesystem; compatibility and performance unaccepted.**
Build and test in isolated fixtures with disposable data. The operator also
authorizes the OpenCode VM for the 2026-10-31 native experiments. Never use
served mounts or production credentials. Use finite subprocess/operation deadlines.

## Statistical gate and adapter quality — 2026-11-02

`bench/paired.py` and `bench/compare.py` now require eight mount pairs, a real
same-executable control arm and predeclared practical margins; 15 paired-assessment
cases plus three-arm scheduling, margin, zero/overflow and writer-file cases pass
with the whole Python suite. Mutation of `bench/paired.py` kills 39 of 42 sites;
the survivors are a bootstrap rank off-by-one of 4,096 resamples and an exact
lower-bound equality that no honest finite fixture can distinguish.

Both allocator LLVM runs pass **265 Rust cases**, including thirteen native
directory contracts (new receive-thread liveness and xattr error contracts) and
the public `errno` mapping. CLI child profiles are now merged into both reports.
`crapper --threshold 10 src/fuse.rs` **passes** (worst `write` 9.5): `errno` and
`fallocate` delegate to `backend_errno`/`allocate`, `getxattr` is covered natively.
`compare.py` orchestration functions that need real mounts (`_measure_run`,
`_run_case`, `main` and helpers) still exceed CRAP 10 at 0-50% unit coverage;
they are exercised by integration runs, not honest unit tests. Clippy, formatting
and `dryer --threshold 0.8` pass.

## Retained rename correction — 2026-11-02

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
timeout 180s cargo test --locked --test fuse_directory -- --ignored --test-threads=1
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
