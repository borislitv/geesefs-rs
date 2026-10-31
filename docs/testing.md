# Testing and verification boundary

**Experimental read/write filesystem; compatibility and performance unaccepted.**
Build and test in isolated fixtures with disposable data. The operator also
authorizes the OpenCode VM for the 2026-10-31 native experiments. Never use
served mounts or production credentials. Use finite subprocess/operation deadlines.

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
Broad mutation coverage, coverage/CRAP and three duplicate candidates remain open.

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
returns EIO on the tested kernel, so the complete runner remains failed.

Actual-service IAM/SSE/signing/PATCH, imported multipart layouts, full CLI parity
and broader durability cases remain open. Aiohttp is not literal nginx; no
all-xfstests pass is claimed. CI covers deterministic checks, not mounted or
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
