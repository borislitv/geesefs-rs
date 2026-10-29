# Bounded refill after a cached reply

## Contract

The [fixed-binary investigation](read-ahead-refill-causality.md) established that
cache hits did not advance the next window, exposing GET latency at demand
boundaries. The callback read APIs now publish the cached result and keep their
existing caller-owned future alive to refill a missing future interval.

This uses the existing FUSE request slot, deadline, JoinSet cancellation and
bounded RangeSink delivery. It introduces no detached prefetcher, executor, cache,
transport, allocator or dependency. A block-returning `read` has no separate
post-publication lifetime and retains its immediate cache-hit behavior. Mounted
FUSE reads use `read_reply_data`, which owns the new continuation.

## Scheduling and ownership

- Inspect the existing adaptive window and a half-window-ahead coverage point.
  Completed cache fragments and current-generation in-flight intervals both count.
- Find the first missing interval from the consumed position, excluding cached
  and already-owned ranges. Bound it by the configured/effective window, the next
  cached/in-flight interval and EOF. Register it through existing fill coalescing.
- Recheck cache coverage after taking interval ownership, because another fill
  can complete between planning and registration.
- Attempt FIFO-aware byte admission without waiting or flushing. Evict only old
  LRU owners while preserving this reader's unconsumed current-generation backing;
  skip optional work when sufficient admission is unavailable.
- Capture the remote-only snapshot after admission and recheck its version,
  generation and visible range. Share the existing streamed window loader, full
  backing charges, conditional GET, checksum/EOF validation and publication logic.
- Cancellation drops the owned task set and fill interval, waking required
  readers. A prefetch error remains observable after a successful cached reply.

The half-window trigger follows a public Mountpoint example, not a universal
performance claim. Upstream implementations, merged fixes and open proposals are
distinguished in [dependency decisions](dependency-decisions.md#read-ahead-refill-scheduling-2026-10-29).

## Behavior verification

The primary public regression first fails on the frozen implementation after
verifying correct cached bytes: the following range never starts. It then passes
with the refill continuation. Nine new scenarios in `tests/read_refill.rs` cover:

- Prompt cached reply and next-window loading before its first consumer.
- Repeated hits sharing the same in-flight refill.
- Cancellation releasing admission and waking a required follower.
- Insufficient headroom preserving unconsumed data instead of funding speculation
  by discarding the current window.
- Existing future cache ranges limiting the GET to the missing gap.
- Concurrent writes excluding stale-generation speculative bytes.
- A callback truncate preventing dirty-snapshot speculation.
- Observable optional failure, retained owned reply and subsequent retry.
- Adjacent fragments, exact missing-tail coverage and EOF.

Eight independent code faults cause the intended assertion-level failures;
the selected `first_gap` boundary mutation is also killed. Sources are restored
and the package cleaned before final verification. Kubernetes checks pass:

```bash
cargo fmt --all -- --check
cargo test --locked --offline --all-targets --all-features
cargo clippy --locked --offline --all-targets --all-features -- -D warnings
python3 -m unittest discover -v -s tests -p 'test_*.py'
python3 -m unittest discover -v -s bench -p 'test_*.py'
python3 scripts/audit_public_sources.py
```

Results: 201 Rust cases, 37 Python cases, warning-denying Clippy and a 339-package
public-source audit. Crapper reports new refill decision helpers at CC 1–9;
coverage and CRAP remain N/A, not zero or a completed coverage gate. The extracted
existing window loader is CC 19 and older complex methods remain. Dryer finds the
same three prior duplicate candidates; full mutation coverage remains open.

## Performance and acceptance

The System-feature release, actual mounted reader/writer checks, matched
comparison against exact previous Rust and both Go references, and direct
socket/FUSE confirmation are in progress. Unit success alone does not prove
lower CPU/RSS or better p95/p99. Overall compatibility/performance acceptance
remains **UNACCEPTED**; the fuser fork and production replacement stay deferred.
