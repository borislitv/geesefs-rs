# Read-history lifetime investigation

## Why this follows the refill work

The two refill candidates improve measured behavior but do not close experimental
Go p99. The premise that refill scheduling alone explains the remaining tail is
insufficient. In the authoritative protected-refill trace, 653 of 683 joined
tail dependencies are initial object reads; response-header waits dominate the
observed service intervals. This is not a causal percentage of global p99.

The ownership difference is concrete: Go has one access history per file handle;
Rust stores it on an inode shared across opens. The Go constructor resets history,
while a Rust reopen can reuse a previous promoted window. Direct request records
show 100 MiB initial bursts for large-file revisits versus Go's 5 MiB start.
Investigate the lifetime assignment rather than add another speculative queue.

## Execution

1. Read the pinned public `NewFileHandle`, `OpenFile`, Rust FUSE open/read/release
   and inode open-count paths. Record the exact source contract and narrow
   correction in `docs/dependency-decisions.md` before runtime changes.
2. Add a public regression that opens a stream, reads sequentially into the large
   window, releases the final open, then reopens at zero. Ordinary pressure has
   evicted the initial backing. Correct bytes pass, but the final range is
   `(0, 4194304)` instead of configured `(0, 1048576)`: expected assertion-level
   cargo exit 101 on `d0d7b99`. The first attempt had a `usize`/`u64` configuration
   compile error; it was corrected and is not counted as regression evidence.
3. Reset the existing `Pattern` only on a zero-to-one open-count transition.
   Keep retained payload and other active handles' history. No new task, cache,
   registry or dependency. Concurrent handles still share inode history; do not
   claim complete per-handle parity.

```bash
cargo test --locked --offline --test read_stream_history -- --nocapture
```

4. The regression passes with the minimal fix. Add positive checks that a second
   active handle does not reset history and a reopened stream reuses cached
   payload. Three deliberate faults fail the intended assertions; both selected
   open-count Mutator replacements are killed. Restore exact source and pass
   the three behavior tests again.
5. Full Kubernetes checks pass 207 Rust and 37 Python tests, rustfmt, Clippy with
   `-D warnings` and the public-source audit. Crapper measures `open` at CC 6;
   coverage/CRAP remain N/A, three previous duplicates and broad mutations remain
   open. The System-feature release is being built for exact-source comparison.

6. Completed the exact System-feature release, 96 native phases /288,000 checks,
   eight socket/FUSE phases including both Go references, and mounted checks.
   Warm excess bytes are zero in all twenty candidate phases and the direct
   range union, but p99 still loses to both Go means. Relative to prior Rust,
   p95 and CPU regress slightly; these results are retained explicitly.
7. Verified 3655 artifacts, 25 compiled inputs and 9424 independent numeric cells.
   Rejoined all 3000 initial reads per executable without exclusions, rather than
   compare different tail-selected populations. The observed remaining mean gap
   is primarily before request write and after required socket bytes. Investigate
   those internal stages before another fix; do not attribute them to server wait.

See `docs/read-stream-history.md` and its numeric report. Overall UNACCEPTED;
remaining p99, representative workloads and long-term compatibility gates stay
open. No production replacement or universal performance claim follows.
