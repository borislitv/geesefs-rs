# Release consumed SDK chunks through maintained reader adapters

## Reproduced cause

The previous absolute profiles isolate excess TCP receive/page cost rather than
an additional FUSE copy. Inspecting the pinned library path identifies one
concrete ownership problem: Smithy ByteStream's `into_async_read()` uses Tokio
`StreamReader<Bytes>`. On consumption it advances the Bytes but retains the empty
handle while polling the next body frame. bytes 1.12.1 `advance` retains the
backing allocation even when no bytes remain. Hyper splits its mutable receive
buffer into shared Bytes; keeping a consumed slice can prevent buffer reuse.

Public upstream [bytes #840](https://github.com/tokio-rs/bytes/issues/840) and
[#856](https://github.com/tokio-rs/bytes/pull/856) confirm that empty-handle behavior.
The latest released bytes version checked is already our 1.12.1; the advance fix
is unmerged. The merged truncate/clear fix does not repair this path. Hyper
[#4060](https://github.com/hyperium/hyper/issues/4060) discusses related backing
pinning, but its header/HTTP2 proxy case is not this reproduction.

## Standard-library integration

Reuse futures-util 0.3.34 `try_unfold`/`TryStreamExt::into_async_read`, whose
AsyncBufRead consumer immediately drops a fully consumed chunk. Adapt it back to
Tokio with tokio-util 0.7.20 `FuturesAsyncReadCompatExt`. The existing admitted
Tokio `copy_buf`/`take` path still initializes one charged fragment. Stack pinning
keeps the adapter allocation-free. Only the published futures-io 0.3.34 crate
is added to the lockfile; existing dependency versions are unchanged.

SDK signing/transport/retries/checksum errors and public ByteStream `try_next`
remain in use. No custom reader, pool, protocol, unsafe code or dependency fork.
Do not adopt retained transport Bytes as payloads: their full backing size is
not visible. Full GET admission, fragment sizes, bounded delivery, final EOF,
whole-body checksum materialization and writer/cancellation contracts remain.

## Verification and next gate

The public S3 regression supplies a faithful HTTP body using BytesMut shared
receive chunks. It reads complete correct bytes through the SDK and verifies
that the producer can reclaim the consumed buffer before the next frame poll.
On unchanged source, all 33 attempted reuses fail the final ownership assertion;
payload/offset/budget checks already pass. With the maintained adapter the test
passes for 8 KiB and unaligned 49,157-byte chunks, interspersed empty frames,
multiple 128 KiB output fragments and final partial output. Existing affected
HTTP, checksum, EOF, late-error, cancellation, writer and FIFO tests also pass.

The [quality receipt](receive-buffer-reuse-quality-20261028.json) reconciles 185
Rust cases, 14 source/oracle and 23 benchmark Python cases, fmt, all-target/
all-feature warning-denying Clippy and audit of 337 public packages after package
clean. Both selected final-EOF tool mutations are killed; 205 S3 sites remain
unselected, coverage/CRAP and three existing duplicates remain open. The native
release binds all 25 compiled inputs to SHA256
`340f8e613b3267713fdb28e2b2c8519a74d0aabff3fc7660d051296a772f7392`.

An exact-source matched native comparison remains required. A fixed ownership
regression does not itself prove lower faults, better throughput/tails or
whole-memory acceptance.
