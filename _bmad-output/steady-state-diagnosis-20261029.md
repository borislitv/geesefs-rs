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

At the first checkpoint, the unresolved join was from a slow FUSE request to byte
admission, fill publication and S3 header/body waits. The representative
random/hot-set/writer matrix, long-term memory plateau, full compatibility and production acceptance
remain incomplete. That fixture was retained under its finite deadline with the
requested Kubernetes limits; completed FUSE mounts, daemons and profilers were
stopped. A later source change must still have meaningful red/green evidence.

## Refill causality continuation

1. Extended the owned process capture with GET Range/HTTP response headers and
   completed socket-read bytes. Compared the frozen Rust cache-hit path with
   experimental Go `ReadFile`/`LoadRange`/`GetHoles`, verifying the public Go blob.
2. Preserved the original fixture's finite-lifetime expiration and failed raw-data
   recovery. The interrupted window-size A/B and lost late network capture are
   not used as authoritative evidence. Removed the two failed recovery helpers.
3. Staged a replacement fixture with the same immutable manifest, executable
   hashes and 8+16 CPU / 16+12 GiB limits. Made backend GOMAXPROCS explicit and
   verified it in the live environment after each controlled read-only restart.
4. Completed the 4/16/16/4 backend-worker control: 48 phases, 144,000 complete file
   checks. More backend workers consume more backend CPU and do not explain away
   Rust's tail deficit. Retained all means, dispersion, CPU/RSS/cgroup scopes.
5. Configured a 5 ms delay only for noninitial GETs and measured actual intervals through the same proxy
   in control and treatment, in 0/5/5/0 ms order. All 24 phases / 72,000 file checks
   pass. In warm treatment, 932/1272 Rust GETs reach the proxy after consumption
   begins, exposing 5828.685 ms of inserted wait; both Go references have zero
   such overlaps. Retained proxy overhead and temporal-drift limitations.
6. Repeated the direct no-proxy socket/FUSE experiment with sixteen backend workers.
   All 205,314 client syscalls match. Of unambiguous noninitial GETs, 484/659 Rust
   requests begin after demand, compared with 0/884 for each Go. Required bytes
   are already received before every joined Go consumer. All response bodies
   complete; initial-object and other request-service waits remain separate.
7. Collected both archives and verified 961 indexed artifacts plus all 25 frozen
   compiled inputs. Exported the numeric receipt and the source-level mechanism.
   No owned FUSE mount, filesystem daemon, profiler or delay proxy remained at
   the collection checkpoint.
8. Independently validated 40,137 numeric cells/interval values, 2,427 direct
   refill dependencies, table rows, links, formatting and public boundaries.
   Signaled the backend controller's normal completion, verified its server
   exited, then deleted both exact owned diagnostic Pods with normal grace.
   Verified both absent and retained private identity/readback receipts.

The source-level prerequisite is now met for a bounded next-window refill repair.
The [causal report](../docs/read-ahead-refill-causality.md) defines its regression
target and preserved ownership/admission/generation constraints. Runtime source
remains frozen; this diagnostic does not claim a completed repair or acceptance.
