# Canonical metadata: frozen allocation diagnostic

This is a synthetic allocation result, **not** a serving-speed, Go-relative or
production acceptance claim. AFTER source is
`f53f4908d2c0f912c687b9688c1856206bcf6725`. The pre-compaction build-input manifest
SHA256 is `eaa98b56f49fe96eeb821c5867d8bb702e64c858afc7fb5f2caf4f3f111b20db`.
Both use Rust 1.99.1, default jemalloc, release/thin LTO, codegen-units 1,
2 GiB memory, zero swap and a one-CPU quota. The later loan-pressure correction
does not change these metadata representations, but is not a new measurement.

## Contract and scope

The same 24 cases ran in separate processes: 14 namespace cases, four inode-only
cases and six controls. Namespace counts are 100k/200k/500k for 64-byte keys with
no remote head, empty metadata, three or four attributes. Long-key cases use
200k entries/four attributes only. Payload admission remains 1000 MiB with zero
payload use; constructor-matrix metadata admission is disabled in both arms.

Real `Inode`/`Namespace` implementations are measured, not a model replacement.
Separate enabled-budget controls exercise actual `Filesystem` ownership with a
deterministic in-memory backend. They verify shared cached-listing reinstall,
no duplicate backing charge, final-owner release and retained upload receipts.
No S3/FUSE workload or eight-mount/A/A speed experiment is implied by this matrix.

## Selected measurements

Allocated B/entry is the filled-minus-starting jemalloc allocation divided by
entries, excluding the root. RSS is absolute **filled-point / recorded VmHWM**;
it is not a steady-state measurement or a claimed whole-run maximum. CPU is
whole-child **user / system ns per entry**, including startup, construction and
drop, not elapsed time or request latency. One sample per case is retained.

| Entries / key bytes / attributes | Allocated B/entry before → after | RSS filled / peak bytes, before | RSS filled / peak bytes, after | CPU user / system ns/entry, before → after |
| --- | ---: | ---: | ---: | ---: |
| 200k / 64 / none | 1226.731 → 906.575 | 255549440 / 255549440 | 193527808 / 193527808 | 1780.635 / 634.685 → 1913.705 / 621.910 |
| 200k / 64 / empty | 1258.740 → 1050.577 | 262111232 / 262111232 | 222760960 / 222760960 | 2033.825 / 713.580 → 2125.370 / 719.275 |
| 200k / 64 / 3 | 2218.909 → 1258.633 | 456097792 / 456097792 | 264818688 / 264818688 | 3215.000 / 1030.915 → 3068.965 / 747.705 |
| 200k / 64 / 4 | 2490.977 → 1314.583 | 511176704 / 511176704 | 276393984 / 276393984 | 3415.575 / 1142.130 → 3269.740 / 822.050 |
| 500k / 64 / 4 | 2506.743 → 1330.610 | 1267335168 / 1267335168 | 680505344 / 680505344 | 3555.414 / 1144.236 → 3420.684 / 855.716 |
| 200k / 128 / 4 | 2618.996 → 1378.602 | 537493504 / 537493504 | 289746944 / 289746944 | 3479.860 / 1187.815 → 3383.050 / 930.430 |
| 200k / 256 / 4 | 2874.998 → 1506.603 | 590389248 / 590389248 | 316108800 / 316108800 | 3848.385 / 1344.535 → 3837.590 / 802.860 |
| 500k / 64 / empty, adverse CPU sample | 1274.676 → 1066.619 | 644841472 / 644841472 | 545980416 / 545980416 | 2018.584 / 570.986 → 2204.362 / 649.188 |

The 200k/four-attribute allocation reduction is **47.23%**. The last row also
retains **10.19% greater total CPU/entry** despite lower allocation. Without
repetition and A/A controls neither favorable nor adverse constructor timing is
a statistically established serving-speed result. `none` means the historical
constructor with no remote head and known local empty metadata, not unknown wire
metadata. All raw cases, including adverse timings, are retained separately.

## Ownership and cgroup boundaries

Clean layouts: Inode 456 B, State 296 B, Snapshot 120 B, RemoteRecord 40 B.
The measured inode Arc allocation is now 512 B versus 896 B; the separate owned
flush gate remains 64 B. Inline layout sizes are not additional heap allocations.
All 21 after-drop allocation checks return within baseline + 1 MiB; all 12 exact
component checks return thread-net allocation to zero. RSS may stay retained.

| Whole 24-case cgroup | Total peak bytes | File bytes at end | Kernel bytes at end |
| --- | ---: | ---: | ---: |
| Before | 1278070784 | 180224 | 544768 |
| After | 690348032 | 299008 | 577536 |

These cgroups include the wrapper and 24 sequential children. File/kernel values
are end snapshots, not a decomposition of the peak. Per-case cgroup peaks,
components at peak and sustained steady-state memory were not recorded.

Enabled-owner controls retain one backing charge across canonical clones;
snapshot wrappers have separate charges. Dropping the final retained metadata
owner releases 8325 admitted bytes and enables a previously rejected allocation.
The lost-successful-PUT control retains operation credit with its receipt:
4087 → 27748 → 4191 → 1873 charged bytes through dirty, pending, durable retry and
normal eviction. Final charge equals a fresh root-plus-replacement control.
Those logical charges are not equated with allocator classes or RSS.

Source/build/probe hashes and raw receipts remain outside tracked source. Matrix
executable SHA256:
`4adf85b923f9273984e4e28d4735e40a863a092694e3d6757ac56e3f91c150f7`.
This result does not attribute an entire production heap or authorize deployment.
