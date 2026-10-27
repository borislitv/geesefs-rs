# Requested-range replies with caller-owned read-ahead

The public Go reference's `LoadRange` waits for requested bytes rather than its
entire read-ahead window. A gated public filesystem test reproduced Rust waiting
for an unrelated parallel tail: two initial publication assertions timed out,
while the requested-dependency error case passed. A broader delegation-only
contract run also reproduced waiting followers and speculative errors incorrectly
cancelling a healthy requested range. These are behavior failures, not build or
fixture setup failures.

## Ownership and error contract

`Filesystem::read_reply` consumes a one-shot callback as soon as all bytes of the
requested result are ready. Cache hits, empty reads and validation failures also
consume it exactly once. The caller continues polling that same future until its
bounded read-ahead completes; a returned error belongs to the tail after a
successful reply. Errors from a chunk intersecting the requested range still
fail that reply. An optional-tail error cannot cancel a still-loading healthy
requested dependency, irrespective of completion order.

The fuser adapter calls this path inside its existing request-admission permit
and whole-request deadline. The callback sends the owned reply synchronously;
the same admitted task retains the JoinSet, byte reservations, generation snapshot
and fill guard. Tail errors are logged. Shutdown still waits on the request
permits, and dropping or timing out the future cancels its bounded child tasks.
There is no extra detached prefetch task, per-read channel, increased budget or
weakened write/fsync contract. The ordinary awaitable `Filesystem::read` retains
its complete-window result/error behavior for callers that need completion.

Completed cache blocks notify overlapping followers through the existing watch
channel. Followers subscribe before rechecking the cache, preventing publication
between lookup and subscription from becoming a lost wakeup. Admission remains
before borrowing a cache block, preserving the earlier FIFO-pressure regression.
Old generation blocks cannot satisfy a new writer generation. A delayed
conditional tail may correctly fail ESTALE after fsync changes the remote ETag;
the already published bytes remain valid and new reads see the new write.

## Verification boundary

Nine public tests in `tests/read_reply.rs` cover early publication, waiting
followers, both dependency-error orders, cancellation after reply, complete bytes
across parallel chunk boundaries, validation/EOF and writes/fsync during a pending
tail. Backend range gates identify exact offset/length pairs so that unrelated
writer materialization is not accidentally held. All observations and joins use
finite deadlines. Existing pressure, ownership, concurrent-window and writer
assertions remain intact.

The initial full Kubernetes check passed 170 Rust cases, formatting,
warning-denying all-target/all-feature Clippy, fourteen Python source/oracle
cases and the 336-package public audit. The Rust evidence wrapper also runs the
23 benchmark Python cases. Four independent faults failed their intended
assertions: deferring the result, omitting progress notification, cancelling a
requested dependency on speculative failure, and omitting generation identity.
Restored focused tests passed. The first selected tool run reached its outer
deadline and exposed an unbounded receive in a newly added test; it is retained
as an unsuccessful attempt, not counted as a killed mutant. Bounded observations
are verified in the follow-up. Both selected publication-condition tool mutants
are killed after forcing package rebuilds for each baseline/worker run; restored
source hashes match. A baseline initially failed despite restored source bytes
matching exactly; a package-clean rebuild passed. Preserved source timestamps and
a shared mutation target therefore require explicit invalidation before final
verification rather than trusting a prior executable. Both failed attempts and
the subsequent rebuild receipts remain retained.

A separate dependency-free Cargo reproduction confirmed the timestamp failure:
the baseline returned 42 and passed; mutation to 43 failed; restoring the exact
original bytes with their older mtime still executed the `Fresh` mutant artifact
and failed with 43. A package-clean rebuild of those unchanged restored bytes
passed. This does not modify production sources or attribute a filesystem bug
to the runtime. It establishes why mutation workers and source upload must not
reuse a stale artifact solely on restored timestamps.

Crapper reports the callback/awaitable wrappers at CC 2, the existing read-inner
function at CC 55, and the progress notifier at CC 2. Coverage/CRAP remain
unmeasured; the three prior duplication candidates remain. Tool success is not
completion of those broad quality gates.

This change publishes only completed backend chunks. The public Go implementation
also publishes each 128 KiB body buffer before its GET completes; Rust's current
backend still validates and finishes its complete conditional GET before returning
a block. Those are separate latency boundaries. This library fix alone does not
establish mounted throughput, RSS, CPU or tail improvement, and does not close
the representative workload or compatibility gates. Overall acceptance remains
**false**.

## Matched native result

The comparison at `d17a5a6` completed all forty phases in 681.567 seconds:
120,000 complete file hashes/metadata checks, unchanged native conditional
hashes before/after, 97 exact listings and 280 independently recomputed CPU
windows. The [numeric receipt](requested-read-reply-results-20261027.json)
preserves all group means, primary dispersion, source/executable hashes and
client/allocator/cgroup/backend memory and CPU. All 460 raw indexed artifacts
were verified after collection, as were compiled inputs against both commits.
The candidate executable SHA-256 is
`481c71765fbad97b85ffad63066fb8acb907c085f76b61b62c63a7312db9c6ff`.

The sealed dataset, default allocator, budgets, explicit metadata workload and
five interleaved orders are unchanged. RSS values below are decimal MB, peak /
final-fifth mean; CPU is whole filesystem-cgroup usage. Fresh does not mean
controlled cold host/backend caches.

| Implementation/state | Read MB/s | Daemon RSS peak / steady MB | Cgroup CPU s | Read p99 ms | Listing s |
| --- | ---: | ---: | ---: | ---: | ---: |
| Go experimental fresh | 597.264 | 1268.474 / 1212.213 | 30.124 | 11.297 | 0.457 |
| Go experimental warm revisit | 591.960 | 1300.260 / 1206.504 | 29.438 | 11.228 | 0.033 |
| Go packaged fresh | 615.907 | 1280.913 / 1209.035 | 29.191 | 10.790 | 0.459 |
| Go packaged warm revisit | 597.647 | 1295.326 / 1223.737 | 28.941 | 10.996 | 0.035 |
| Rust requested reply fresh | 563.202 | 537.638 / 368.317 | 31.597 | 18.483 | 1.481 |
| Rust requested reply warm revisit | 538.924 | 649.971 / 363.534 | 31.590 | 18.095 | 0.021 |
| Rust prior views fresh | 560.411 | 538.938 / 362.251 | 31.160 | 19.017 | 1.385 |
| Rust prior views warm revisit | 543.090 | 630.777 / 364.948 | 31.162 | 18.429 | 0.021 |

This boundary change does not resolve the measured deficit: fresh throughput is
nearly unchanged, warm throughput is lower, whole CPU is higher than matched
prior, and Go still wins throughput/p99/CPU. Some memory and listing metrics also
regress. The gated behavior fix is valid; the premise that whole-window waiting
dominates these measurements is not. The next isolated boundary is the complete
conditional GET: test body-prefix publication while preserving range/generation,
checksum, EOF, cancellation and full ownership. File-cache pressure and missing
Go heap/backend request metrics still preclude whole-memory acceptance.
