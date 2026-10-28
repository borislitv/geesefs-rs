# Absolute CPU attribution before another read-path change

## Scope and method

After the borrowed-reply and initialized-fragment comparisons did not show a
reproducible overall win, reject the premise that redundant application copies
and initialization alone dominate the remaining sustained cost. Count the actors
before selecting another mechanism: cache lookup/maintenance, allocation and
kernel page work, transport/body delivery, and cold metadata resolution.

Nine separate diagnostics rotate fresh mounts of exact initialized Rust
`e000ecfd0d05aac39fc5809bfadc784ad1a75946`, Go experimental
`5c0884c9e8799bbab93e2d4102fdc15e9c079e0d`, and packaged Go
`80c214ef14f620d93407454fc76ca01dd71fa25f`. Each reads and checks the same
3,000 synthetic files / 8,298,727,528 bytes, with complete metadata work.
The filesystem/client container has 2 CPU / 4 GiB, the separate sealed S3
container 4 CPU / 6 GiB; payload budget is 512 MiB and entry limit 100,000.

Record only the owned daemon PID:

```bash
perf record -F 149 -e cpu-clock -g --call-graph dwarf,8192 -p PID -o PROFILE
perf report --stdio --no-children --call-graph none --sort dso,symbol \
  --percent-limit 0 --fields overhead,period,sample,dso,symbol -t '|' -i PROFILE
perf script -i PROFILE -F period,event,ip,sym,dso
```

Convert summed event periods from nanoseconds to sampled CPU seconds. Compare
with independently sampled daemon user/system CPU and stage counters, not only
hot-function percentages. All nine profiles have zero reported lost samples;
27,000 complete file hashes, native post-run verification of 3,000 objects and
97 exact listings pass. Independently reconcile all raw profile/report hashes,
decoded stack periods, per-file oracles and process counters.

## Observations

Three-run means, decimal MB. RSS is process peak / mean final fifth, not a
stationary plateau. These are **instrumented diagnostics, not acceptance timing**.

| Implementation | Daemon CPU user / system / total s | RSS peak / final-fifth MB | Minor faults | Selected cache leaf CPU s | Selected fault leaf CPU s |
| --- | ---: | ---: | ---: | ---: | ---: |
| Rust initialized | 5.707 / 10.433 / 16.140 | 565.447 / 425.669 | 1181876 | 1.591 | 1.168 |
| Go experimental | 4.557 / 8.930 / 13.487 | 1328.346 / 1217.618 | 320207 | 0.000 | 0.345 |
| Go packaged | 5.003 / 9.287 / 14.290 | 1271.924 / 1211.841 | 302507 | 0.000 | 0.306 |

The cache category selects Rust Moka and standard BTree leaf symbols; its zero
for Go does **not** mean Go caching is free. The fault category selects
`do_user_addr_fault`, `do_anonymous_page`, `__handle_mm_fault`, and
`clear_page_rep`; it is not all kernel cost. Recorded CPU covers approximately
96.8–97.3% of actual Rust CPU versus 92.4–93.3% experimental Go and 92.2–92.9%
packaged Go. Do not assign unrecorded CPU to a symbol or claim exact category
differences from unequal capture coverage.

Rust stacks pass through `ReadCache::covering` for a mean 1.781 sampled CPU
seconds, insertion for 0.340 and payload admission for 0.163. These inclusive
stack groups overlap other categories and must not be added to leaf totals.
Inspection identifies repeated predecessor hash lookups and owner clones even
when a block cannot cover the requested bytes. The next candidate filters those
spans in the existing index before consulting Moka.

Fault stacks resolve mainly through TCP receive buffers and then admitted Tokio
fragment copy, rather than proving that our output pre-zeroing remains the sole
cause. The page-fault count alone does not establish allocator causality. Mean
daemon stage costs are listing / metadata / payload: Rust 0.930 / 0.377 / 14.833 s,
experimental Go 0.157 / 1.227 / 12.103 s, packaged Go 0.173 / 1.330 / 12.787 s.
Cold listing remains a separate issue; removing metadata checks is not a fix.

The retained source report SHA256 is
`1e6ed6b72e2ae7f6ddb53cfeb36fbc6a9f66872cd5b53c530ff51737122507c4`.
Unknown/truncated application frames and missing Go heap series remain gaps.
No allocator configuration, concurrency, cache budget or production mount changed.

## Upstream findings checked on 2026-10-28

- [Moka #498](https://github.com/moka-rs/moka/issues/498): maintenance and blocking
  concurrent-table rehash can delay read/write operations. Maintainer discussion
  distinguishes implicit maintenance from table rehashing and discusses papaya;
  this is relevant context, not proof that its large-cache reproduction is ours.
- [Moka #412](https://github.com/moka-rs/moka/issues/412): an async-cache scheduling
  busy loop was fixed in 0.12.6. We use sync 0.12.16, the latest published release
  checked; blindly upgrading for that issue is inapplicable.
- [jemalloc #2824](https://github.com/jemalloc/jemalloc/issues/2824): large
  allocations, size-class rounding, purge and page-fault costs can interact.
  The previous unchanged-binary allocator trial already traded higher RSS for a
  small mixed performance change; retain that result, not a default tuning claim.
- [AWS SDK #1030](https://github.com/awslabs/aws-sdk-rust/issues/1030) requests
  AsyncRead-to-ByteStream upload construction. It is not a demonstrated fix for
  the measured download path; do not mistake the common ByteStream name for a
  matching cause.
- [FUSE passthrough evaluation](fuse-passthrough.md): the selected library has
  safe APIs, but the measured kernel rejects the capability. Its local backing
  file and writer-coherence requirements need a separate design and experiment.

Search discovery was followed by direct public GitHub API reads of issue bodies,
maintainer comments and release metadata. Saved-copy page extraction did not
produce usable text; no unseen page is treated as verified evidence.
