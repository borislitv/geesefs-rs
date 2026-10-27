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
**false** pending matched measurements.
