# Sequential read window regression follow-up

## Status and scope

The bounded core fix and regression checks are complete. Whole-filesystem
compatibility and performance remain **UNACCEPTED**: no post-fix mounted paired
measurement has been obtained in this task. Existing unfavorable benchmark
receipts are unchanged. No cache budgets, benchmark settings, FUSE negotiation,
dependencies or production mounts were changed.

The preceding full-matrix experiment stopped at its 5400-second deadline after
42 of 280 runs. The parent reported five cold sequential 256 MiB repetitions
averaging about 3.55 seconds for Go and 422.67 seconds for Rust. These are
pre-fix measurements, not evidence of improvement from this change.

## Contract and root cause

The public Go reference is
[`core/file.go` at 5c0884c9](https://github.com/yandex-cloud/geesefs/blob/5c0884c9e8799bbab93e2d4102fdc15e9c079e0d/core/file.go).
`trackRead` (lines 521–541), `getReadAhead` (543–555), and `ReadFile` (559 onward)
establish the following contract:

- Choose a window from the preceding sequential-run history, then record the
  current nonempty caller request, including requests satisfied from cache.
- Accumulate contiguous request lengths into the current run. Only a seek records
  the completed run; individual small requests are not separate run samples.
- Retain at most `small_read_count - 1` completed runs. A count of one disables
  completed-run history rather than promoting every small request to small-read
  mode. Do not wait for a full history before applying the small-read policy.

Rust previously updated the pattern only on cache misses and sampled individual
request lengths. A sequential stream of small kernel reads therefore looked like
small random reads after the first cached windows. Both errors mattered:
tracking cache hits alone did not restore the required window selection.

`Filesystem::read` now chooses and records the window once per nonempty request,
before the cache-hit return. `Pattern` retains completed contiguous runs and
selects from preceding history. Output admission still precedes borrowing cached
blocks, preserving the earlier FIFO-pressure progress fix. Zero-length and EOF
reads do not advance tracking.

## Public-interface regression evidence

`tests/read_windows.rs` exercises `Filesystem` with a nonconstant 32 MiB object,
256 consecutive 128 KiB reads, unchanged 64 MiB configured memory, exact returned
bytes and EOF checks. Expected ranges are independent explicit examples:

```text
offset MiB    length MiB
0             5
5             5
10            5
15            5
20            12
```

The original implementation performed 139 backend range reads for this case.
Moving tracking onto cache hits without correcting history still performed 121.
The corrected implementation performs exactly the five ranges above. These are
deterministic fixture request counts, not measured mounted throughput.

Additional public tests cover:

- Cancellation while materializing the promoted window after 160 sequential
  128 KiB reads; all transient owners release, retry returns the correct bytes,
  and cache/data-lane usage remains within the unchanged capacities.
- Real 64 KiB seeks, which must still reduce windows under default history;
  interleaved zero-length reads must not distort it. A count of one must retain
  default windows for the same seeks.

Failure sensitivity was observed on the original code. Tracking hits alone also
failed the sequential count assertion. A deliberate restoration of the old
full-history requirement failed the seek-window assertion; the correct condition
was restored before final verification.

Final verification passed: `cargo fmt --all -- --check`,
`cargo clippy --locked --all-targets --all-features -- -D warnings`, and
`cargo test --locked --all-targets --all-features` (126 tests, none ignored).
This includes existing large-window, retained-cache, FIFO-pressure, cancellation,
namespace and backend contract coverage.

## FUSE handoff and remaining verification

No public `Filesystem::read` signature change or extra wrapper-level tracking is
required. The parent owns FUSE changes and fresh release/mounted measurements.
For the existing public fuser 0.18 API:

- `KernelConfig::set_max_write` controls maximum write size; setting it to 1 MiB
  does not guarantee 1 MiB kernel READ requests.
- `set_max_readahead` is bounded by the kernel-advertised maximum. It is distinct
  from the filesystem's S3 prefetch window.
- fuser enables `FUSE_MAX_PAGES` when advertised and derives its private
  `max_pages` from maximum write/readahead sizes. There is no public
  `set_max_read` or `set_max_pages` setter in this version.
- Inspect negotiated capabilities and actual READ request sizes rather than
  assuming client read sizes survive unchanged through the kernel. Do not change
  benchmark workloads or budgets to conceal the regression.

Pattern ownership remains per inode, whereas Go tracks per open file handle;
interleaved independent readers can affect this heuristic. This bounded fix does
not claim full per-handle parity. Rebuild the release binary and obtain new paired
receipts with equivalent datasets, generations, budgets, quotas and cache state.
Preserve the incomplete matrix and report new correctness, memory, throughput
and latency results separately before revisiting acceptance.
