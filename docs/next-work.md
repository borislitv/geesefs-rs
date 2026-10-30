# Next work: observed problems and causes, 2026-10-30

Overall status remains **UNACCEPTED**. This is the handoff for the next iteration;
it separates confirmed mechanisms from hypotheses. See the latest
[measurement tables](../README.md#latest-completed-measurements--2026-10-30),
[architecture](architecture.md) and [comparison contract](performance.md).

## 1. Warm random reads: cache working-set eviction

**Observed:** the complete five-repeat warm matrix gives Rust 1.01 MiB/s versus
7.90/8.33 MiB/s for experimental/packaged Go. Mean run p99 is 7.428 ms versus
1.026/0.897 ms; independently pooled p99 is 7.680 versus 0.764/0.886 ms.
Concurrent-reader throughput and RSS also do not beat both references.

**Confirmed contributing mechanism:** `Filesystem::new` reserves 10 MiB for flush
and 4 MiB for ingress at a 64 MiB total, leaving 50 MiB for clean/dirty/pinned data.
The deterministic random trace's first plain pass fetches about 60 MB; repeating
the same offsets still makes 396 backend reads / 51,961,856 bytes. The reply-path
repeat makes 417 / 54,718,464. Reverse-order revisit reduces misses to 56. Static
capacity plus access order demonstrably causes repeated fetches; it is not proof
that every millisecond of mounted latency comes from the cache.

**Where:** `src/filesystem.rs::{new,admit,try_refill_admission,load_window}`,
`src/cache.rs::{insert,evict_lru}`, `src/read.rs::Pattern`.

**Next:** correlate retained ranges, backing charges and real mounted GET counts;
test a minimal policy change against the same trace and both Go references.
A proposed idle-flush borrowing policy has an observed diagnostic RED: a 12 MiB
clean working set at a 16 MiB total makes 192 rather than 96 backend calls across
two passes. This is a desired new policy, not a broken existing guarantee; its
test/source is retained privately, and no implementation is delivered.
Before borrowing, prove writer progress, cancellation/loan release, full pinned
backing charge and existing FIFO invariants. Do not simply enlarge the data lane
or remove the reserve. Reuse the maintained permit/cache APIs first.

## 2. Persistent mixed mount misses its target rate

**Observed:** all three implementations complete 60 cycles / 360 checked passes.
Rust takes 885.09 s and misses the 10-second cycle target 58 times; experimental
Go takes 600.01 s with no late cycles, packaged Go 600.00 s with one. Rust uses
less process RSS/CPU, but late-half cgroup memory is 314.7 MiB versus experimental
Go's 298.0 MiB. Different durations do not establish an equal-rate memory win.

**Cause boundary:** static cache pressure is independently reproduced, but its
share of mixed-cycle delay has not been isolated. A sampled late-half mean,
linear slope or one persistent run does not prove a memory plateau or leak.

**Where:** `src/filesystem.rs` read/refill/admission, `src/read.rs`, `src/cache.rs`,
`src/main.rs` allocator observation; `bench/accounting.py` measurement scopes.

**Next:** retain per-operation mixed-cycle latency and backend traffic; repeat
with sufficient backend headroom and controlled observer overhead. Compare the
same fixed work and the same offered rate separately. Keep whole-series daemon,
client, allocator, cgroup, file and kernel memory; extend duration and repeats
before claiming a plateau. Preserve baseline-owned Go GC.

## 3. Resource and observer effects still qualify performance claims

**Confirmed:** the earlier parent ceiling was only 8 CPU while daemon/client and
backend each had 8 CPU. That attempt accumulated 251.118 s parent throttling.
After raising the parent to 24 CPU, the completed matrix still records backend
throttling in sequential/eviction runs. Subsequent persistent/observer experiments
use 32 CPU parent, 16 CPU backend and unchanged 8 CPU daemon/client limits.
Resource enlargement is not a source optimization.

**Where:** `bench/compare.py::{cgroup,Sampler}`, `bench/go_heap.py`,
`src/main.rs` optional statistics writer; fixture parent/backend limits.

**Next:** inspect before/after counter deltas for every compared interval, not
cumulative old throttles. Complete repeated observer-on/off attribution before
treating observation as free: Go requests pprof data, Rust writes allocator
statistics. Short metadata phases are particularly noisy and often contain fewer
than two valid Go heap samples; those allocator values must remain missing.
Partial observer experiments cannot replace the complete workload matrix.
The final on/off attempt retains 133/180 planned records, all correct, and stops
before a new mount at its bounded deadline. Metadata, sequential, random and
concurrent groups have five pairs per implementation; eviction is partial and
writer/fsync is unmeasured in this attempt. Its final generation check is not
reached. Preserve these omissions and pair-level variance; do not call it a
completed observer-overhead acceptance test.

## 4. Remaining initial-read/tail cost has no proven SDK defect

**Observed:** the earlier sealed read comparison improved throughput/RSS but
Rust mean p99 still lost to both Go references in every rotated pair. Socket/FUSE
joins show extra work before request transmission and after required socket bytes.
Removing external observers and moving to client-wide hooks does not remove it.

**Confirmed boundary:** endpoint resolution is only part of setup; two calls
match auth selection and endpoint orchestration. SDK 1.155.0 already includes the
investigated interceptor/endpoint improvements; SHA acceleration and TCP_NODELAY
are enabled. A bucket-only endpoint cache is invalid for key/prefix-sensitive
Params. No single library defect has been demonstrated.

**Where:** `src/backend/s3.rs` SDK setup/range path,
`src/filesystem.rs::load_window`, `src/fuse.rs` request/reply lifetime.

**Next:** separate SDK, scheduler, admission and reply work using controlled
profiles and a behavioral reproducer before another workaround. Preserve
If-Match, checksum validation and ownership. FUSE passthrough and disk cache
remain deferred.

## 5. Open-file fsync recovery differs from the pinned Go references

**Observed:** all 20 Go fault-recovery runs in the complete matrix end with EAGAIN
through the unchanged 75-second recovery deadline; all ten Rust runs pass.

**Confirmed reference mechanism:** public Go `Inode.SyncFile` clears `forceFlush`
before returning a retained `flushError`; small-file `sendUpload` needs a closed
handle, `forceFlush` or memory pressure. Waiting past the retry interval alone
does not arrange another upload. This is not evidence of Rust data loss.

**Where:** pinned Go `core/file.go::{SyncFile,sendUpload}`;
`scripts/integration.py::recover_fsync`, writer-failure workload.

**Next:** add a separate close/recovery boundary probe to characterize reference
semantics, preserving the original failing open-handle workload. Do not lengthen
timeouts or close the handle to relabel those failures as success. Clarify the
required recovery contract independently of baseline weaknesses.

## 6. Linux hardlink-as-symlink returns EIO after creating the alias

**Observed and explained:** Rust and both pinned Go binaries fail the mounted
probe on Linux 6.8. The symlink is already visible after EIO. Linux
`fuse_link/create_new_entry` requires reply/source file types to match; returning
a symlink for a regular source violates that requirement.

**Where:** `src/fuse.rs` link callback, `src/filesystem.rs` link emulation;
public kernel reference in [compatibility](compatibility.md).

**Next:** choose an explicit supported Linux contract and add a mounted regression
for both return status and post-error side effects. Do not silently accept the
shared failure as compatibility or pretend it is a Rust-only regression.

## Remaining acceptance and quality gates

- `bench/compare.py::Sampler.metrics` computes its `steady` memory fields from
  the latter half of a `deque(maxlen=1000)`, not the whole measurement's late
  half once that buffer wraps. The README operation table calls this a late
  window; the persistent table uses independent whole-series analysis instead.
  Before using long-run `steady` fields in acceptance, replace this bounded-tail
  aggregation with an explicit whole-window/time-window contract and a regression
  that exceeds 1,000 samples. Otherwise an earlier sustained high-memory interval
  can disappear from the reported mean. Raw JSONL and sampled peaks are retained.
- Actual-service IAM, signing/encryption/PATCH, complete Linux CLI parity and
  imported multipart layouts remain incomplete; loopback tests do not replace
  those checks. See [backend options](backend-options.md).
- Concurrent handles share inode adaptive history. Zero-to-one reopen reset is
  implemented; per-handle policy isolation is not. Require an interleaved-reader
  reproducer before introducing that change.
- Heap-observer automatic mutation scope killed 24/25 mutants; one boundary
  survivor remains. This is not whole-repository mutation success. Broader Rust
  quality findings and binary redistribution notices remain open.
- A merge can preserve this experimental checkpoint, but does not establish the
  agreed all-workload speed/memory goal or authorize production replacement.
