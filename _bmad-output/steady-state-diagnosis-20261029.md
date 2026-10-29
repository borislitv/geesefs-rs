# Steady-state diagnostic execution log: 2026-10-29

## Intent

Investigate the fixed Rust and Go executables before another runtime change.
The operator prioritizes normal long-lived operation over cold startup and
authorizes additional test CPU and memory to separate resource limits from code.

## Actions and reasons

1. Rechecked project rules, frozen-source/binary provenance and the immutable
   3,000-file fixture. Preserved early compiler, namespace-mapping, aggregate
   decoder and empty-quiescence-parser failures rather than counting them as
   valid profiles. No runtime source change or allocator-default change is made.
2. Captured owned-daemon CPU samples and client syscall/scheduler spans. Matched
   init-namespace thread identities explicitly. Separate FUSE/CFS events exposed
   large quota overlap but did not establish a causal percentage.
3. Executed a four-allocation CPU screen and a same-mount revisit run. Kept two
   runtime workers while changing capacity. A same-capacity 10 ms period screen
   reduced p99 but worsened p95 and throughput, so it is not adopted.
4. Applied the requested CPU/memory increase through the Kubernetes resize
   subresource, preserving container identities and dataset generations. Actual
   8+16 CPU limits removed throttling. The first 8 GiB filesystem/client cap still
   caused reclaim, so increased only that cap to 16 GiB and repeated the matrix.
5. Verified all real limits and CPU/memory pressure deltas. The 16+12 GiB fixture
   has no phase limit events or filesystem/client reclaim. Ordinary host scheduling
   and small backend fault/pressure observations remain explicit.
6. Compared two and eight runtime workers with identical resource limits for all
   implementations. More Rust workers increase CPU/RSS without removing its tail;
   the improved Go measurements remain in the comparison. All 216 unprofiled
   phases pass 648,000 full-file checks, with native before/after verification.
7. Captured two separate expanded-fixture profiles and rendered eighteen CPU,
   daemon off-CPU and client off-CPU SVGs using pinned upstream FlameGraph tools.
   Joined all 68,438 reads per implementation, with no missing syscall matches.
   Filtered FUSE events by the owned mount's superblock device. Remaining Rust
   tail time is mostly blocked request service, not runqueue/quota delay.
8. Resolved exact libc symbols and attributed copy leaves to FUSE replies, TCP
   receive and admitted SDK fragments. Preserved unknown callchain roots. The
   YANET2 LTO clue is documented as a candidate, not a proved GeeseFS cause.
9. Collected and verified 3,040 private artifact hashes. Exported public numeric
   evidence and SVGs without private identifiers or fixture credentials. Updated
   the authoritative scope, latest-results links and incomplete project state.

## Verification scope and remaining work

These are diagnostic tooling and documentation changes. New reader matrices
exercise the same compiled executables and independent byte/metadata oracle;
they do not re-establish all prior Rust unit, writer or failure-injection coverage.
No new runtime regression test is warranted for unchanged runtime source.
Public evidence validation checks hashes, all numeric summaries/table cells,
JSON/SVG structure, links, formatting and provenance boundaries.

The next unresolved join is from a slow FUSE request to byte admission, fill
publication and S3 header/body waits. The representative random/hot-set/writer
matrix, long-term memory plateau, full compatibility and production acceptance
remain incomplete. The diagnostic fixture is retained under its finite deadline
with the requested Kubernetes limits; completed FUSE mounts, daemons and profilers
are stopped. A later source change must still have meaningful red/green evidence.
