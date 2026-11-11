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
- The contention/lazy/max-record/backing follow-up passes 502 tests in default and
  system-allocator configurations, with 24 explicit opt-in entries.
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

The [follow-up qualification](performance-disk-cache-followup.md) uses qualified
SeaweedFS Range behavior and independently reviewed fixed work. All 456 mounts
complete: 454 pass the full byte/state contract; two concurrent cold-requested
mounts fall back on the existing read timeout despite completed backend hits.
Random-hot and eviction CPU/throughput regressions remain. Correct native
capacity reuse and representative passes do not close these repeated-mount gates.

The validated-backing correction removes repeated fragment demotion and improves
warm-random throughput/CPU in eight Rust-relative pairs plus eight A/A controls.
All 48 candidate mounts in its six-group comparison pass, but two baseline
timeouts leave concurrent comparison incomplete. This is not absolute L2/RAM or
Go acceptance. Next: attribute actual timeout delay and investigate fragmented
source GETs under eviction, preserving all measurements.
Controlled media-cold, ongoing postallocation I/O errors, Go-relative and target
acceptance remain open. All owned completed-run resources were cleaned up;
serving settings were not changed.
