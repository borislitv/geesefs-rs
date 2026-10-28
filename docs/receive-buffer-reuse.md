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

## Exact-source native comparison

Source `f2e0f04e2c79b3f7ec06531268ae620b384dfd6c` completes forty uninstrumented
phases, repetitions 80–84, against index prior `a5862ed` and both frozen Go
references. All 120,000 complete file/metadata checks, unchanged native hashes
before/after, 97 exact listings, 280 CPU windows and 460 artifact hashes pass.
Elapsed time is 762.907 s. Only Cargo.toml/Cargo.lock/S3 differ among 25 inputs.

The sealed dataset remains 3,000 files / 8,298,727,528 bytes; filesystem/client
2 CPU / 4 GiB, separate backend 4 CPU / 6 GiB, payload 512 MiB, 100,000 entries,
four readers at 128 KiB, default jemalloc. Fresh means a new process/mount, not
controlled cold host/backend caches. Warm revisits the complete larger-than-cache
dataset. Complete xattr/forced-stat work is equal and included in whole CPU; the
earlier failing plain-list/stat Go workload remains separate.

Five-phase means, decimal MB. RSS peak / mean final fifth is process memory, not
a stationary plateau. [All numeric scopes and dispersion](receive-buffer-reuse-results-20261028.json)
include daemon/client user/system CPU, p50/p95, allocator and separate
cgroup/file/kernel/backend memory; missing Go heap/backend request series remain.

| Implementation/state | Read MB/s | Daemon RSS peak / final-fifth MB | Whole cgroup CPU s | Read p99 ms | Listing s |
| --- | ---: | ---: | ---: | ---: | ---: |
| Go experimental fresh | 520.128 | 1299.735 / 1225.468 | 34.812 | 13.300 | 0.512 |
| Go experimental warm-revisit | 530.952 | 1291.396 / 1223.302 | 32.959 | 12.753 | 0.034 |
| Go packaged fresh | 517.489 | 1313.254 / 1200.156 | 34.828 | 13.121 | 0.544 |
| Go packaged warm-revisit | 512.863 | 1287.946 / 1195.442 | 33.956 | 12.981 | 0.036 |
| Rust candidate fresh | 536.137 | 552.002 / 453.327 | 33.078 | 13.067 | 1.521 |
| Rust candidate warm-revisit | 521.917 | 562.125 / 490.855 | 33.166 | 13.540 | 0.024 |
| Rust index prior fresh | 510.780 | 566.107 / 444.542 | 34.880 | 13.727 | 1.667 |
| Rust index prior warm-revisit | 486.770 | 568.287 / 468.958 | 35.391 | 15.256 | 0.022 |

Against exact prior, mean throughput improves 4.96% fresh / 7.22% warm, whole CPU
falls 5.17% / 6.29%, p99 falls 4.81% / 11.25%, peak RSS falls 2.49% / 1.08%.
Throughput favors four fresh / three warm pairs, CPU and p99 four pairs each,
peak RSS all five each. But final-fifth RSS rises **1.98% / 4.67%**, improving
only one fresh pair and no warm pair. Candidate throughput population stdev is
18.881/43.167 MB/s and p99 stdev 0.322/1.654 ms; no significance claim is made.

Candidate fresh mean throughput/CPU narrowly beat both Go references in this
repeat. Warm throughput/CPU remain worse than experimental Go, warm p99 worse
than both Go references, and cold listing remains slower. Lower process RSS
does not establish a total-cgroup memory win under file-cache pressure. Overall
acceptance remains **UNACCEPTED**; keep the adverse metrics and continue diagnosis.

## Separate absolute page-cost confirmation

Nine rotated fresh owned-PID perf profiles of candidate, index prior and
experimental Go pass another 27,000 full-file checks and native hashes/listings.
Zero lost samples, 27 raw profile/report/stack hashes, absolute periods and
independent process/stage counters reconcile. The
[profile receipt](receive-buffer-reuse-profiles-20261028.json) records source and
binary identity. These are instrumented diagnostics, not acceptance timings.

Prior → candidate daemon user/system CPU is 4.567/13.473 → 4.843/11.313 s;
minor faults fall 1,168,644 → 740,848. RSS peak/final fifth is
563.884/452.330 → 552.025/476.072 MB. Go is 5.923/11.373 CPU s, 315,759 minor
faults and 1294.789/1195.327 MB RSS. Selected fault leaves fall 1.436 → 0.805 s.

Retained caller stacks partition those selected faults: TCP receive falls
1.119 → 0.181 s; admitted-fragment copy instead rises 0.309 → 0.604 s. Candidate
other/unresolved callers account for 0.018 s and FUSE 0.002 s. The next concrete
actor is fragment allocation/page reuse, not another speculative transport change.
The [standard system allocator screen](system-allocator-screen.md) subsequently
improves throughput/CPU/p99 but increases RSS; it requires truthful heap accounting
and a full Go comparison before policy selection. Earlier jemalloc tuning/RSS
tradeoffs remain retained.

Candidate capture is 96.9–97.2%, prior 97.0–97.6%, Go 93.1–94.0%. Unrecorded CPU
and unknown frames are not assigned, and overlapping callchains are not added to
leaf categories. The source report SHA256 is
`60ee6974dac4272500f4bacdf8f9c56ba3f8369b7127ab010e514371d942289e`.
