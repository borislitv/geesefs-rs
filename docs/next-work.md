# Next work: observed problems and causes, 2026-10-30

Overall status remains **UNACCEPTED**. This is the handoff for the next iteration;
it separates confirmed mechanisms from hypotheses. See the latest
[measurement tables](../README.md#latest-completed-measurements--2026-10-30),
[architecture](architecture.md) and [comparison contract](performance.md).

## 1. Warm random reads: cache working-set eviction

**Observed before the correction:** the earlier five-repeat warm matrix gives
Rust 1.01 MiB/s versus 7.90/8.33 MiB/s for experimental/packaged Go. Mean run p99
is 7.428 ms versus 1.026/0.897 ms; independently pooled p99 is 7.680 versus
0.764/0.886 ms. These measurements describe the unchanged predecessor.

**Confirmed contributing mechanism:** `Filesystem::new` reserves 10 MiB for flush
and 4 MiB for ingress at a 64 MiB total, leaving 50 MiB for clean/dirty/pinned data.
The deterministic random trace's first plain pass fetches about 60 MB; repeating
the same offsets still makes 396 backend reads / 51,961,856 bytes. The reply-path
repeat makes 417 / 54,718,464. Reverse-order revisit reduces misses to 56. Static
capacity plus access order demonstrably causes repeated fetches; it is not proof
that every millisecond of mounted latency comes from the cache.

**Where:** `src/filesystem.rs::{new,admit,try_refill_admission,load_window}`,
`src/cache.rs::{insert,evict_lru}`, `src/read.rs::Pattern`.

**Subsequent attribution:** three rotated fixed-binary native controls make
3,381–3,389 Rust GETs per 4,096 warm random reads; both Go references make zero.
These controls and CPU/off-CPU captures pass Go `--gc-interval 0`; they are
configured diagnostics, not evidence of unchanged-default Go performance.
PID-scoped CPU profiles and complete GET/FUSE lifecycle joins show no reported
sample/event loss, quota throttling, memory reclaim or OOM. Instrumented times are
kept separate; scheduler wait and instrumentation overhead remain measurable.
Transport timing is not proof of a single SDK defect or every internal await.

**Correction:** the 12 MiB clean / 16 MiB total diagnostic first fails with 192
rather than 96 backend calls. Required admission now transfers exclusive clean
owners into idle flush capacity, using existing Tokio/lru ownership APIs. Total
and lane limits remain unchanged; writers revoke loans, and owned replies cannot
export a revocable backing. Cancellation, independent flush concurrency, failure
recovery and FIFO/pinned ownership are tested. Optional refill does not initiate
loans. The reopened-history test now creates real independent cache pressure
instead of depending on an incidental old cache capacity.

**Measured outcome:** the default-GC four-binary comparison completes all 120
records and verifies all 30 generations. Candidate warm-random GETs are zero in
all five repeats, versus 3,380–3,389 for the predecessor. Total-work/time throughput
is 9.07 MiB/s versus Go 8.80/8.88, but mean run p99 still loses: 0.926 ms versus
0.714/0.650. Concurrent throughput is 4.09 versus 4.41/4.35 MiB/s and uses more
RSS/cgroup memory. Writer pooled p99 is 348.573 versus Go 288.321/347.198 ms;
the favorable mean-run-p99 column cannot override this loss.

**Next:** attribute concurrent/admission ownership, writer-tail regressions and
the rare serial stalls below before another change. Keep the additional retained
cache's memory tradeoff visible; reducing GETs alone does not close acceptance.

## 2. Persistent mixed mount misses its target rate

**Latest observed:** all four binaries complete 60 cycles / 360 checked passes.
Candidate Rust takes 709.11 s versus predecessor 873.78 s and 600.00 s for both
Go references. Late cycles are 58/54/0/0 respectively. Candidate peak RSS is lower
than Go, but its whole-series late-half RSS is 232.1 MiB versus experimental Go's
226.9 MiB; cgroup late means are 296.0 versus Go 300.1/300.2 MiB. Different
durations and one fixed-order observation do not establish an equal-rate memory
win or repeatability. The earlier 885.09-second Rust run remains retained under
its different resource configuration.

**Cause boundary:** candidate/predecessor have 11/16 approximately 30-second
successful serial reads; both Go have zero. Candidate sequential/eviction active
times total 164.23/252.35 s; experimental Go totals 118.57/113.41 s. Persistent
metadata also totals 35.65 s versus Go 2.90/2.88 s despite the faster short warm
metadata checkpoint. Memory/quota counters remain clean, while filesystem/client
I/O PSI-some reaches 57.9%/63.3% for candidate/predecessor. This identifies waits,
not their responsible SDK, admission, FUSE or backend stage. P99 and mean rates
hide rare stalls; retain raw calls, maxima and total-work/time rates. A matching
30-second admission timeout is a hypothesis, not a proven cause. A sampled mean,
linear slope or one persistent run does not prove a memory plateau or leak.

**Where:** `src/filesystem.rs` read/refill/admission, `src/read.rs`, `src/cache.rs`,
`src/main.rs` allocator observation; `bench/accounting.py` measurement scopes.

**Next:** correlate the retained slow offsets/calls with scoped FUSE events,
SDK/body progress and permit owners; observe a behavioral RED before fixing the
responsible wait. Keep observer overhead controlled. Compare the
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

The first default-GC candidate attempt stops after 47 completed records on the
strict memory-pressure gate. Backend working files persist for the whole shared
bucket lifetime. Their footprint falls after scoped bucket deletion, motivating
one bucket per paired workload group, not higher application budgets. All 120
records in the versioned isolated comparison pass byte/generation and quota/memory
checks. CPU and I/O waits remain measured; the resource change is not a source
performance improvement. The partial/rejected attempt remains retained.

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
  half once that buffer wraps. The older operation checkpoint labels this a late
  window. Latest README tables use independent full-raw-series analysis instead.
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
