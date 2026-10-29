# Read-ahead refill execution log

## Contract and decisions

1. Rechecked the fixed-source causal report and public Mountpoint, Goofys, S3FS
   and RocksDB issue/code evidence. Recorded exact links and delivered versus
   proposed status in `docs/dependency-decisions.md` before changing the runtime.
2. Traced FUSE request admission, `read_reply_data`, the caller-owned fill future,
   JoinSet/RangeSink cancellation, cache backing charges and the earlier FIFO
   regression. Reuse that request lifetime; do not detach cache-hit work.
3. Added a public callback-read regression: consume cached bytes halfway through
   a one-MiB window, block the following backend range, require prompt correct
   reply plus an already-started caller-owned refill, then verify boundary reuse.
   Its backend barrier is a real reachable dependency delay, not a private-method
   or implementation-call-order assertion.

## Verification

The unchanged runtime fails the new regression at its intended assertion:
`a cache-hit reply must start the next window before its first demand read`,
with `Ok(false)` instead of `Ok(true)`. The bytes assertion passed first; cargo
exits 101 with exactly one failed test. The original compiled-source hashes match
the frozen baseline. Command in the owned Kubernetes build Pod:

```bash
cargo test --locked --offline --test read_refill cache_hit_starts_next_window_before_boundary_without_delaying_reply -- --exact --nocapture
```

The first SSH exec transport remained pending before starting the worker. Its
local process group was stopped; an explicitly bounded container-side launcher
then ran the test. A missing local psutil import during transport cleanup was
corrected using the existing process tools; neither was a regression-test result.

4. Added caller-owned cache-hit continuation and shared the existing streamed
   window loader. Reuse cache/in-flight coverage, conditional generation identity
   and FIFO permits; skip unavailable optional admission and preserve unread
   current backing. Recheck coverage after claiming the fill interval.
5. The first candidate compile rejected `gen`, a reserved Rust keyword. Renamed
   that local binding; this setup failure is separate from the baseline red.
   Eight new tests and all 77 selected reader/cache/contract cases then passed.
6. Factored shared interval-gap traversal and small refill decisions without
   introducing another state machine. Repeated the focused suite after that
   change. Added the ninth adjacent-fragment/EOF case and made the callback
   truncate scenario retain a remote-readable prefix so its guard is observable.
7. Eight independently inserted faults fail the intended public assertions;
   Mutator kills the selected strict gap-boundary replacement. Restore exact
   hashes and clean the package. Full Kubernetes verification passes 201 Rust,
   37 Python cases, rustfmt, all-target/all-feature Clippy and 339-package audit.
8. Crapper measures new refill helpers at CC 1–9, with coverage/CRAP unavailable.
   The shared existing loader is CC 19; broader old complexity remains. Dryer
   retains the same three previous duplicates; unselected mutations remain open.
   Release and matched native measurements follow before any performance claim.

9. Built the exact System-feature release and completed mounted checks (17 pass,
   known hardlink EIO retained), 96 matched native phases / 288,000 file checks,
   and four direct socket/FUSE phases / 12,000 checks. The scheduling mechanism
   is repaired, but both Go p95/p99 remain better and excess payload increases.
10. Preserved 1,180 indexed artifacts and the numeric report. An independent
    source-row/aggregate check verifies 9,424 numeric cells and resource windows.
    Published the adverse traffic result alongside speed, CPU and memory metrics.

Continue with `_bmad-output/refill-eviction-20261029.md`. Overall acceptance
remains UNACCEPTED until the broader project gates are met.
