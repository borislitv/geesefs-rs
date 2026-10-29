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

The first candidate `6e165250e48bbf8f0f9e6c916fecc48f8586033b` was measured with
System-feature binary SHA256
`eb130f3eb61524b512d59be0401abeea2dcaa96746c37e9e3ea67043828f58c4`.
The [numeric report](read-ahead-refill-results-20261029.json) retains all 96 native
phases, resource windows, provenance, counters, allocator data and limitations.
Four rotated mounts per executable each ran a fresh phase and five warm revisits:
288,000 full-file checks, followed by 3,000 native hashes and 97 listings. Every
phase passed correctness checks, with no measured CPU throttling or memory-event
increments. Filesystem/client limits were 8 CPU / 16 GiB, backend 16 CPU / 12 GiB,
and parent 24 CPU / 28 GiB; payload budget remained 512 MiB.

Warm arithmetic means (20 phases per executable, four independent mounts):

| Executable | MB/s | p50 ms | p95 ms | p99 ms | Whole filesystem/client cgroup CPU s | Daemon peak / final-fifth RSS MB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Rust refill `6e16525` | 1334.350 | 0.089432 | 1.198103 | 3.403519 | 28.603719 | 806.200 / 804.097 |
| Prior Rust `3016433` | 1228.349 | 0.092999 | 1.423830 | 3.658706 | 29.298234 | 805.536 / 803.742 |
| Go experimental `5c0884c` | 1049.420 | 0.212630 | 1.091975 | 3.163167 | 28.815647 | 1322.557 / 1228.448 |
| Go packaged `80c214e` | 1007.693 | 0.223593 | 1.138923 | 3.310254 | 30.529088 | 1331.052 / 1225.851 |

These are means of phase percentiles, not pooled percentiles. Final-fifth RSS is
not a proven memory plateau. The report separately retains client/backend RSS,
cgroup/file/kernel memory, daemon and client CPU, normalized costs and dispersion.
The first refill improves prior-Rust throughput by about 8.6%, p95 by 15.9% and
p99 by 7.0%, while RSS is essentially unchanged/slightly higher. It still loses
p95/p99 to both Go references and does not win every CPU metric.

The direct network diagnostic (four additional phases, 12,000 full checks) joins
136,876 syscalls without unmatched reads. Among unambiguous noninitial GETs,
prior Rust started 491 of 666 after consumer entry; the refill candidate started
all 659 before entry. Socket-byte lead medians change from -1.676 ms to +3.761 ms.
These instrumented timings confirm the repaired mechanism, not acceptance or an
explanation of all waits.

An adverse result remains: mean warm excess response payload increases from
13.817834 MB to 88.796175 MB. Direct interval analysis finds 105.480429 MB of
duplicate candidate payload, concentrated in two files. Cross-reader eviction is
being investigated with a separate failure-sensitive regression and candidate;
these first-candidate measurements must not be attributed to that later change.

Actual mounted verification passes 17 ordinary reader/writer/failure checks but
retains `hardlink-emulation:FAILED:errno=5`. The original export verifies 1,180
artifacts and 25 compiled inputs; an independent check additionally verifies
9,424 source-row/aggregate numeric cells and the resource intervals. Overall
compatibility/performance acceptance remains **UNACCEPTED**.

The subsequent [cross-reader eviction comparison](read-ahead-eviction.md) retains
this first candidate as its exact baseline and reports the remaining p99 gap.
