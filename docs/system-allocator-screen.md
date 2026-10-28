# Standard system allocator screen

After the receive-buffer fix, retained profiles place 0.604 sampled CPU seconds
in admitted-fragment page faults, versus 0.181 s in TCP receive. Test the standard
Rust `std::alloc::System` before another allocator assumption or buffer mechanism.
Its Unix backend uses platform malloc; no application allocator or unsafe code.

Build a reversible diagnostic patch over exact `f2e0f04`: only main.rs changes the
global allocator and labels system heap statistics as missing instead of reporting
unrelated jemalloc counters. All 185 Rust cases, fmt, Clippy and native release
pass in Kubernetes. Restore the original source and package-clean afterward.
The patch is included in the [numeric receipt](system-allocator-screen-20261028.json);
binary SHA256 is `6bef37fb725148e17f0e1de1b6e7d4b3d15dd1c739b9dbb3ef238465e0ba51ee`.

Three rotated same-source pairs run fresh/warm, repetitions 90–92. All twelve
phases, 36,000 complete file/metadata checks, native before/after hashes and 97
listings pass. Independently reconcile 84 CPU windows and 147 artifact hashes.
The dataset, 2 CPU/4 GiB filesystem quota, separate 4 CPU/6 GiB backend, 512 MiB
payload budget, entries, concurrency and metadata checks match the preceding
comparison. This is a screening diagnostic, not the full acceptance matrix.

Three-phase means, decimal MB. RSS is peak / final-fifth mean, not a plateau.

| Allocator/state | Read MB/s | Daemon RSS peak / final-fifth MB | Whole cgroup CPU s | Read p99 ms |
| --- | ---: | ---: | ---: | ---: |
| Jemalloc fresh | 527.942 | 558.002 / 477.060 | 33.973 | 12.701 |
| Jemalloc warm-revisit | 484.986 | 556.930 / 466.976 | 35.567 | 14.699 |
| System fresh | 593.952 | 574.611 / 566.657 | 30.442 | 10.845 |
| System warm-revisit | 534.451 | 590.748 / 581.833 | 32.578 | 12.402 |

System improves mean throughput 12.50% fresh / 10.20% warm, whole CPU 10.39% /
8.40%, and p99 14.61% / 15.63%, but peak RSS rises 2.98% / 6.07% and final-fifth
RSS **18.78% / 24.60%**. Retain both sides of the tradeoff. The receipt includes
dispersion, process CPU, client/cgroup/file/kernel/backend memory and missing
system heap series; missing does not mean zero. Do not infer an overall Go win
from this two-allocator screen.

The next candidate needs truthful heap accounting and a full exact-source
Go/jemalloc/system comparison before choosing a default. Maintained wrappers must
preserve platform realloc/calloc behavior, and their measured counter overhead
must be included. Production, cache budgets and durability are unchanged.

The [opt-in accounting candidate](system-allocator-accounting.md) now preserves
platform realloc/calloc and explicitly reports request-counter limitations.
