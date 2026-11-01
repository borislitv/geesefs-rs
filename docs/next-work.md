# Next work: observed problems and causes, 2026-11-01

Overall status remains **UNACCEPTED**. This is the current working inventory;
it separates confirmed mechanisms from hypotheses. See the latest
[measurement tables](performance-integrated.md),
[architecture](architecture.md) and [comparison contract](performance.md).

The subsequent [native VM checkpoint](performance.md#native-vm-admissiondirectory-checkpoint--2026-10-31)
attributes and corrects the admission and global-directory-invalidation mechanisms
below at `67f7aec`. Earlier measurements retain their original scope; the new
fixed-work control improves total elapsed but not every tail or memory metric.
The reverse-order offered-rate control adds a late-run metadata-tail regression
and higher late cgroup memory than both Go references to the unresolved work.
The later integrated checkpoint below corrects the demonstrated metadata
mechanisms and measures them jointly, but retains throughput and memory losses.

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

**Earlier observed:** all four binaries complete 60 cycles / 360 checked passes.
Candidate Rust takes 709.11 s versus predecessor 873.78 s and 600.00 s for both
Go references. Late cycles are 58/54/0/0 respectively. Candidate peak RSS is lower
than Go, but its whole-series late-half RSS is 232.1 MiB versus experimental Go's
226.9 MiB; cgroup late means are 296.0 versus Go 300.1/300.2 MiB. Different
durations and one fixed-order observation do not establish an equal-rate memory
win or repeatability. The earlier 885.09-second Rust run remains retained under
its different resource configuration.

**Earlier cause boundary:** candidate/predecessor have 11/16 approximately 30-second
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

**Subsequent confirmed cause/correction:** a scoped FUSE/ownership join shows the
admission timeout while an in-flight fill has already published reclaimable cache
fragments. The admission waiter never revisits them. A gated public read first
fails for this mechanism; Notify-driven reclamation now preserves its FIFO
reservation. Separate diagnostics show 15 LISTs per mixed cycle because unrelated
writer mutations change the global directory-page epoch. Directory-local epochs
remove that invalidation, with TTL, binding, rename/delete and cancellation checks.

**New VM outcome:** the final 60-cycle fixed-work comparison takes 200.89 s for
corrected Rust versus prior Rust 251.35 s, with one-to-zero calls above 20 s.
The separate reverse-order 30-cycle offered-rate control completes in 300 s for
all four binaries without lateness. Neither Rust binary has the rare stall in
that paced control. It does not establish the same improvement under every rate.
Corrected metadata pooled p99 rises to 1.26849 ms versus prior Rust 0.07488 and
Go 0.02886/0.02510. Cycle 28 alone has 345 metadata calls above 1 ms: a sustained
operation concern, not only initial listing. Late cgroup memory is 389.8 MiB
versus Go 350.6/371.4 despite lower sampled daemon RSS. Full tables and scope
qualifiers are in [performance](performance.md#native-vm-admissiondirectory-checkpoint--2026-10-31).

**Subsequent confirmed cause/correction:** a frozen-baseline FUSE/HTTP join places
128/256/384 file HEADs inside serial LOOKUP in paced cycles 14/21/28. Cached name
pages reused older child attributes; returning a new full kernel TTL compounds
their differing ages. Child freshness/type/binding validation, authoritative clean
LIST refresh, remaining TTL and supported bounded READDIRPLUS correct those
mechanisms. Newer HEAD/commit observations and marker attributes remain protected.

**Further confirmed cause/correction:** frozen-candidate HTTP traces show repeated
standard LIST prefix scans for bounded chunks. Supported SDK V1 marker / V2
start-after skip consumed names while opaque tokens retain priority. The extended
protocol remains unchanged. Lexically adjacent directory/object names are not
skipped, and native 8,192-name traversal now uses 65 bounded LIST calls including
EOF. Standard LIST still resolves accurate types rather than assuming regular
files from metadata-free listings.

**Integrated outcome:** 240 healthy isolated cold/warm records and 2,880 mixed
phases are independently verified with all generation/full-byte checks. Paced
metadata p99 is 0.02417 ms versus prior Rust 0.06678, but metadata throughput is
23,110 ops/s versus Go 36,404/42,814; 1 MiB writer throughput also loses. Late
cgroup memory is 395.21 MiB versus Go 357.86/370.08, including file cache
198.35 MiB versus Go 171.33/155.68. In isolated concurrent work, late cgroup is
177.98 MiB versus Go 153.30/154.98. These owners/components, and the remaining
SDK/scheduler/admission work, need causal attribution before another correction.
Do not shrink budgets, remove exact metadata/durability or force reclamation.
The [checkpoint](performance-integrated.md) retains missing metrics, original
controller errors, observer controls and previous-Rust-relative losses. Extend
duration and repeated offered-rate designs before claiming a memory plateau.

**Subsequent ownership attribution:** owned-superblock events and PFN-to-memcg
tracking identify 256 MiB of final payload pages in Rust and both Go references.
Rust finishes faster and spends more idle time retaining the full kernel cache.
Because `FOPEN_KEEP_CACHE` is unset, these pages cannot survive the next open.
Best-effort last-clean-close invalidation through supported fuser notifications
passes the original retained-page failure and matching native client contracts.
The separate 30x10-second control reduces Rust late cgroup/file means to
217.75/19.23 MiB versus Go 355.43/171.68 and 371.88/159.30 MiB. Daemon late RSS
153.29 MiB remains essentially equal to experimental Go153.27 MiB, not a robust
process-memory win. Metadata throughput19,777 ops/s and writer throughput24.20 MiB/s
still lose to both Go references. Repetition, isolated workloads, observer controls
and the full joint gate remain open; the earlier checkpoint is not overwritten.

**Further confirmed amplification/correction:** the FUSE cursor consumed at most
128 names while standard S3 LIST asked for up to 1,000. An expired 1,024-name
traversal therefore made nine overlapping requests. Align bounded consumption
with the supported backend-page size, preserving namespace capacity, exact types,
small-limit/error behavior and kernel-buffer continuation. The native 8,192-name
regression first fails on 65 LIST calls instead of 10, then passes alongside all
nine directory contracts.

**Subsequent outcome:** a new 30x10-second mixed control verifies 720 phases/four
full-byte/generation checks. Rust metadata throughput is 31,315 ops/s versus Go
42,015/42,256; pooled p99 is 0.03241 versus 0.02656/0.02799 ms. Late cgroup/file
means are 216.25/19.54 MiB versus Go 357.97/173.84 and 371.53/156.78 MiB.
Experimental Go misses three cycle deadlines; other implementations miss none.
Shared-host ordering and one design do not establish repeatability or a plateau.
Writer/random metrics and some tails still lose; inspect the warm metadata CPU
path and writer wire/ownership costs next. Duplicate missing-name writer probes
remain visible, but negative-cache policy, checksum/signing and post-commit
verification must not be bypassed simply to improve the comparison.

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

The integrated checkpoint adds complete fixed-work, paced and observer-off mixed
controls. Recorded filesystem/client and backend interval counter deltas are zero;
all owned mounts/processes stop. The single observer-off control preserves the
metadata-tail improvement but still loses Go metadata/writer throughput. Its
memory gauges are unobserved, and different orders/shared-host scheduling still
prevent a claim of zero observation cost. Isolated sub-10 ms metadata windows
also lack required late/allocator samples; original startup/budget observations
must not be substituted for those missing in-window measurements.

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

**Separate close probe:** both references recover full backend bytes after close
(43.37/43.44 seconds in the local fixture); Rust does so after 0.26 seconds.
The unchanged open-handle probe still fails for Go through 75 seconds and passes
for Rust. Clarify the required recovery contract independently of reference
weaknesses; the close result is not recovery for the original workload.

## 6. Linux hardlink-as-symlink returns EIO after creating the alias

**Observed and explained:** Rust and both pinned Go binaries fail the mounted
probe on Linux 6.8. The symlink is already visible after EIO. Linux
`fuse_link/create_new_entry` requires reply/source file types to match; returning
a symlink for a regular source violates that requirement.

**Where:** `src/fuse.rs` link callback and compatibility flag help;
public kernel reference in [compatibility](compatibility.md).

**Correction:** the Rust callback returns EOPNOTSUPP before mutation; the unused
core alias constructor is removed. A native regression verifies errno, no alias
and unchanged source/backend bytes. Both Go failures remain preserved. This
closes unsafe post-error mutation, not required hardlink-emulation compatibility.

## Remaining acceptance and quality gates

- Sampler and allocator accounting now stream the complete temporal latter half,
  with >1,000-sample/irregular/missing/zero regressions. Shared validation and
  nearest-rank pooling feed gates and summaries; daemon/client, heap and file/kernel
  components cannot hide behind combined RSS. Late sample means still do not
  establish a plateau; observer overhead needs repeated whole-workload controls.
- Actual-service IAM, signing/encryption/PATCH, complete Linux CLI parity and
  imported multipart layouts remain incomplete; loopback tests do not replace
  those checks. See [backend options](backend-options.md).
- Per-open adaptive history now has interleaved/new-handle/borrowed-read proofs;
  cache, fills, cancellation and budgets remain shared. Clean fsync preserves
  reusable loans. Their combined measurements retain random/concurrent, mixed
  writer/metadata and equal-rate cgroup losses; passing those proofs is insufficient.
- Heap-observer automatic mutation kills 24/24 sites and shared metrics 30/30,
  including the former exact-two-sample boundary survivor. Affected duplication
  >= 0.8 is absent. Both allocators now have real entrypoint/native coverage, and
  the locked native-notice/source bundle is verified. Whole affected-file CRAP
  still fails; larger rename/lookup/write routines, benchmark orchestration and
  broader mutation scope remain open. See [testing](testing.md) and
  [distribution notices](distribution.md) for exact scopes.
- A merge can preserve this experimental checkpoint, but does not establish the
  agreed all-workload speed/memory goal or authorize production replacement.
