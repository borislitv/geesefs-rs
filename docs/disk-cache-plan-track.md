# Progress tracker for disk-cache-plan.md

Status: implementation and local correctness delivered; **performance qualification
failed and remains open**. No deployment or production acceptance is implied.

## Execution stages

[X] Stage 1: Qualify the storage dependency and freeze resource contracts
[X] Stage 2: Add CLI configuration and regex policy
[X] Stage 3: Verify CLI and policy boundaries
[X] Stage 4: Implement the isolated L2 adapter
[X] Stage 5: Verify capacity eviction, failures and recovery
[X] Stage 6: Integrate demotion, reads, lifecycle and metrics
[X] Stage 7: Verify cross-tier correctness and resource progress
[ ] Stage 8: Complete mounted/performance qualification and acceptance evidence

## Delivered scope

- Baseline `a5aa9c003d219d7e683cc2053f8e32c653173742`; three disk CLI flags,
  no payload TTL/inactivity, capacity-only best-effort population and source fallback.
- Maintained Foyer fork `d60dd0462c1de1e0853590791371208b659c90bf`: guarded Store
  admission, bounded CLOCK/index/recovery, tombstone-wrap and lifecycle corrections.
  Its 130 native tests, three doctests, default-feature build and Clippy pass.
  Released-engine REDs remain independent evidence; no automatic inventory reset.
- Changes/PRs belong only in the operator's repositories. External upstream
  submissions require a separate decision; premature earlier submissions were withdrawn.
- Application implementation includes owned async recovery/shutdown, stable
  static/anonymous access scopes, byte-accounted L2 funding, metrics and exact
  generation/coverage checks. Opaque/rotating/profile/IAM scopes explicitly bypass.
- Independent metadata admission and canonical metadata/key compaction preserve
  writer baselines and lifetime charges. The [frozen allocation diagnostic](performance-metadata-compaction.md)
  verifies actual memory reduction rather than inferring it from a count limit.
- The later loan-first starvation correction retains disabled/excluded loan
  behavior while demoting Ready eligible victims before borrowing. Policy ceilings
  include revocable loans; actual selected residency can differ and is reported.
- Final runtime `fac68b40d8a31c15dcd03d87f0537068137c22ff`: 480 tests pass in
  default and system-allocator configurations, with 24 explicit opt-in entries.
  Clippy, format and source audit pass. Native ext4 block/inode ENOSPC and EACCES
  checks run separately; they do not establish EDQUOT or power-loss durability.

## Stage 8 outcome and remaining gate

The small-object mounted protocol passes: exact recovered L2 bytes without GET,
nginx full/Range/EOF, rsync read/write, metadata, fsync failure/recovery and startup
preallocation ENOSPC fallback. Ongoing postallocation I/O-error injection is not
claimed by that startup test.

All **456 planned matrix cases were attempted**: 239 measured/correct and 217
failed. Nine groups have eight paired mounts and eight A/A controls, but none
passes all declared screens. Large-hot and eviction preparation times out on the
loopback fixture; one allocator-frame capture failure leaves sequential cold at
seven pairs. Measured L1/metadata/writer resource regressions remain visible.
See [the frozen result](performance-disk-cache.md); neither incomplete groups nor
improvements in a subset of metrics are promoted to acceptance.

Next gate: isolate the fixture's full-object-before-Range cost and qualify a
supported bounded/streaming fixture before another large matrix. Investigate
measured runtime CPU/RSS/cgroup side costs independently; a fixture explanation
does not remove them. Native large-set eviction, controlled media-cold behavior,
Go-relative and target-environment acceptance remain open. All owned measurement
mounts, workers and cgroups were cleaned up; serving settings were not changed.
