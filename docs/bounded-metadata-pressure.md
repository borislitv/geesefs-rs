# Bounded directory metadata resolution and read-pressure progress

## Existing measurement, not a post-change result

The parent supplied an actual owned-cgroup diagnostic pair in
`benchmark-results/private/paired-smoke-20261026-v1/`:

| Receipt | Correctness | Metadata throughput | Elapsed, 1024 operations |
| --- | --- | --- | --- |
| `5c0884c9-cold-metadata-0-go.json` | true | 5697.5527 ops/s | 0.179726 s |
| `5c0884c9-cold-metadata-0-rust.json` | true | 678.7162 ops/s | 1.508731 s |

The receipts request 64 MiB filesystem memory, 100000 entries, client concurrency
4, one CPU quota and a 1024 MiB cgroup memory maximum. This is one cold diagnostic
pair, not the complete repeated acceptance matrix. It predates the changes below.
No post-change paired measurement or speed/memory improvement is claimed here.

## Directory resolution

Standard LIST does not supply authoritative user metadata. Concrete symlink and
special-file types must still be resolved before returning safe fuser dirents.
The old selected-page loop awaited one lookup at a time. It now reuses maintained
`futures-util::FuturesUnordered` and the same public Filesystem lookup path.

Each lookup future obtains an owned filesystem-wide Tokio semaphore permit
**before** entering the unordered set. `--max-requests` bounds admitted resolver
jobs across concurrent readdir calls, not once per page. No spawned per-entry
tasks wait on a semaphore; each caller has at most one admission waiter while it
also polls its admitted jobs. Outer FUSE admission bounds concurrent callbacks;
direct library callers remain responsible for the number of caller futures they
create. This resolver limit is not a limit on unrelated direct lookup/getattr
operations or a whole-process memory bound.

Admission and each admitted lookup have the configured HTTP timeout. Cancellation
or the first error drops the borrowed futures synchronously, releasing permits;
no detached directory jobs are created. Sorted page publication, namespace epoch,
inode generation, metadata and type checks remain intact. Authoritative extended
metadata still avoids extra HEADs; internal rmdir emptiness probes still require
no type resolution. Existing namespace races may still return EAGAIN.

`Filesystem::effective_budgets()` now reports:

- `max_metadata_resolution_jobs`: the global resolver limit;
- `metadata_resolution_jobs_in_use`: admitted jobs, including not-yet-polled ones.

Two gated-head public-interface tests verify concurrent progress, the global
bound across two directories, exact ordered file/symlink/FIFO types, cancellation,
dependency failure and successful retry. Sequential code failed because a second
HEAD did not start before the first response. A deliberately per-page semaphore
failed the cross-call assertion with four starts instead of the global limit two;
the mutation was restored.

## Cached read admission

The pressure candidate was reachable through public Filesystem operations. With
the unchanged 16 MiB configuration (5 MiB shared data lane), the test retains an
owned reply, polls a cache fill and then an older large miss into FIFO admission,
releases the reply, and lets the fill install a cache entry after eviction. A
cache hit can then pin that entry while its small output reservation waits behind
the older large request. The older request needs the very permits pinned by the
hit, so progress depends on cancellation or the admission timeout rather than on
available I/O work.

Output admission now occurs before borrowing any cache block. On a cache miss,
the temporary output reservation is released before the existing atomic
window/scratch/reply reservation. Cache hits still copy only into a charged reply;
pinned external allocations remain charged through their last owner. Lane sizes,
read windows, full shared cache policy and the pressure/error policy are unchanged.

The deterministic test uses explicit polls and a biased join, independently checks
all returned bytes, and drops consumed replies rather than artificially pinning
them until both operations finish. Its deadline failed on the original path and
again when that path was deliberately restored; the fix passes. No sleeps create
the interleaving, and no uncharged copy or extra payload lane is introduced.

## Verification

- Focused directory-resolution and read-window suites passed.
- `cargo fmt --all -- --check` passed.
- `cargo clippy --locked --all-targets --all-features -- -D warnings` passed.
- `cargo test --locked --all-targets --all-features`: all 123 Rust tests passed,
  none ignored, including the parent's existing FUSE lifecycle case.
- Public dependency metadata audit: 336 packages, no non-public/path sources and
  no missing license declarations. Futures-util was already transitive; its direct
  use is documented in the dependency decision record.

Only core, related Rust tests, the dependency declaration/lock and documentation
changed. No FUSE/main, scripts, benchmark implementation or CI edits; no VCS, SSH,
production or mounted/benchmark execution. Fresh paired runs with unchanged
budgets and full accounting remain the parent's next verification step.
