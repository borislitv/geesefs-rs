# System allocator with explicit request accounting

The [allocator screen](system-allocator-screen.md) justifies a full comparison,
not a default change. Build the candidate explicitly:

```bash
cargo build --release --locked --features system-allocator
```

Ordinary builds retain jemalloc. The optional, public MIT-licensed stats_alloc
0.1.10 crate wraps `std::alloc::System` using its stable `INSTRUMENTED_SYSTEM` API.
The wrapper delegates alloc, alloc_zeroed, realloc and dealloc to the platform;
no application allocator, pool, unsafe block or malloc tuning is introduced.
Its source release is
[`be31f749696faab326bf18b5dd75eaf93010693d`](https://github.com/neoeinstein/stats_alloc/tree/be31f749696faab326bf18b5dd75eaf93010693d).
The release is from 2022, not a newly released implementation.

## Metric contract

Existing `--stats-file` samples identify the allocator and metric scope. For this
candidate, `allocated_bytes` is a sampled difference between requested allocation
and deallocation bytes, with all six raw wrapper counters retained. It is **not
exact live heap, allocator-reserved memory, process RSS or cgroup memory**:

- Counters are loaded separately and updated before platform allocation calls.
- Failed allocation/reallocation requests are included. The request balance may
  remain inflated after a handled allocation failure.
- Concurrently inconsistent negative differences become null rather than zero.
- Native allocations bypassing Rust's global allocator, allocator overhead and
  retained/free pages are outside the request balance.
- The wrapper's atomic operations are included in every candidate measurement,
  including when `--stats-file` is not supplied.

Jemalloc samples retain their size-class allocation statistic and identify that
different scope. Neither statistic substitutes for independent RSS, cgroup,
file/kernel memory and backend measurements. Do not equate these two allocator
columns or count missing Go heap data as zero.

## Verification and pending experiment

Kubernetes package-clean checks pass: 185 Rust cases, formatting, warning-denying
all-target/all-feature Clippy, default-feature compilation and the 338-package
public dependency audit. The existing filesystem/S3/FIFO/cancellation tests are
unchanged; they do not themselves establish the CLI allocator's mounted behavior.

The [verified native release](system-allocator-accounting-quality-20261028.json)
has SHA256 `98280f9792419af709e372950ec089ebf7ca56bb59647aa8a35e104e58477399`.
All 25 compiled inputs and their exact change set were independently reconciled.
Crapper reports run CC 30 without coverage/CRAP; three prior duplicate candidates
remain, and five main.rs mutation sites were scanned but not executed.

The full five-repetition rotated comparison is pending.
Use the frozen receive-buffer jemalloc binary and both frozen Go binaries, the
sealed dataset, unchanged budgets/concurrency and complete metadata/file oracles.
Check actual CLI counter samples and preserve every adverse memory/tail result.
Overall compatibility/performance acceptance remains open.
