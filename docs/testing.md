# Testing and verification boundary

**Experimental read/write filesystem; compatibility and performance unaccepted.**
Build and test in isolated Kubernetes fixtures with disposable data. Never use
served mounts or production credentials. Use finite subprocess/operation deadlines.

## Library and contract checks

Use Rust 1.96.0+ with rustfmt/clippy, Python 3.12+ and the public dependencies from
`requirements-fixture.txt`. The latest verified runtime is `2bf8184`: 207 Rust
cases, formatting, warning-denying Clippy and public-source audit passed.
Runner corrections through `1ecc1b4` pass 57 Python cases and the source audit.
These checks are not full reference parity or production acceptance.

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
`read_refill.rs`, `read_stream_history.rs`, `receive_ownership.rs` and
`backend_contract.rs`. Keep FIFO progress, full backing charges, If-Match,
generation, checksum, cancellation and caller-owned task lifetimes intact.

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
