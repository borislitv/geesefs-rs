# Progress tracker for disk-cache-plan.md

Status: implementation authorized; checks are local. Deployment and target-host
acceptance are separate later work.

## Execution stages

[X] Stage 1: Qualify the storage dependency and freeze resource contracts
[X] Stage 2: Add CLI configuration and regex policy
[X] Stage 3: Verify CLI and policy boundaries
[X] Stage 4: Implement the isolated L2 adapter
[X] Stage 5: Verify capacity eviction, failures and recovery
[X] Stage 6: Integrate demotion, reads, lifecycle and metrics
[X] Stage 7: Verify cross-tier correctness and resource progress
[ ] Stage 8: Local mounted verification, measurements and draft delivery

## Notes

- 2026-11-09: baseline `a5aa9c003d219d7e683cc2053f8e32c653173742`.
- Configuration was simplified to three CLI flags: directory, maximum size and
  regex. No YAML loader, payload TTL or inactivity policy is required. Queue
  limits remain bounded implementation settings within the total payload budget.
- Stage 1 is blocked: Foyer 0.22.6 loses completed reinserted records on reopen.
  The local qualification suite has three passing tests and one failing test;
  no test was ignored or weakened to accept the dependency.
- The failure reproduced on tmpfs and ext4. An isolated upstream cutoff-only
  diagnostic produced RED → GREEN → restored RED. This is not an adopted fix.
- No storage dependency is accepted. Stages 2–8 wait for compatible
  reinsertion/recovery semantics or qualification of another maintained engine.
  Stalled-I/O, corruption/collision and large-inventory resource checks remain open.
- Operator follow-up authorizes continued dependency work and parallel subagents,
  including other maintained candidates and a maintained Foyer fork/fix. Continue
  autonomously; engine-choice approval is no longer a stopping point.
- Subsequent clarification: fixes and PRs belong only in `borislitv/foyer` and
  `borislitv/geesefs-rs`. Premature upstream submissions were withdrawn. Stay on
  the maintained fork until equivalent upstream corrections are verified; any
  external submission requires a separate explicit operator decision.
- 2026-11-10: the historical released-engine blocker above is corrected in fork
  `d60dd0462c1de1e0853590791371208b659c90bf`. Guarded admission, bounded CLOCK,
  streaming recovery, tombstone wrap and geometry/lifecycle checks pass 130 native
  tests, three doctests, default-feature build and all-target Clippy. No automatic
  reset is permitted on interrupted recovery. Application funding remains explicit.
- CLI/policy stages are implemented with an intentional enabled-runtime startup
  guard. Mark completion only after final application-pin verification.
- Separate opt-in metadata byte admission was added after the operator's choice.
  Independent review exposed and corrected retained-result, encoding, rename
  pre-admission and listing progress defects. Full mounted/performance acceptance
  remains pending; no serving settings were changed.
- 2026-11-10 checkpoint: application tests on the exact new pin pass, including
  engine ownership/integrity and CLI boundaries. The isolated adapter uses guarded
  Store APIs, owns directory/funding through cancellation, and reuses completed
  records across child-process interruption. Stage 5 still needs native quota/
  inode-exhaustion coverage; stages 6–8 remain outstanding.
- Backend scope is stable only for explicit known static credentials and anonymous
  access. Opaque/rotating/profile/IAM providers explicitly bypass persistence;
  one resolved non-expiring credential does not prove a stable access context.
- Metadata review closed seven reproduced defects, including pre-remote rename
  cancellation; 31 focused metadata tests pass. Synthetic variable-name/metadata
  pressure stays under the configured ledger ceiling while preserving pins.
- The full all-feature application suite passes with the fixture venv on PATH.
  The initial default run's performance harness failed because the system Python
  lacked psutil; this environment failure is retained separately. Neither this
  suite nor synthetic ledger checks establish mounted/performance acceptance.
- 2026-11-10 integration: L1 capacity victims alone feed L2 after cache-lock release;
  validated reads/refill consult exact L2 coverage, and async recovery falls back to
  the source. Funding is deducted from L1 without changing ingress/writer reserves.
  Independent review found and corrected an old-size/new-generation admission panic.
- All 460 active all-feature tests pass; 24 explicitly opt-in native/diagnostic
  entries remain ignored in that command. Native ext4 block/inode ENOSPC and
  permission checks were run separately and passed without resetting existing media.
  Clippy with warnings denied, formatting and source audit pass.
- Exported observations distinguish admission from completion, region-reclaim
  starts from evicted records, operation latency from pure device latency, funded
  index bytes from measured heap, and captured st_blocks from live physical usage.
- Stage 8 harness is implemented but its mounted/performance matrix remains pending.
  The operator's subsequent metadata-compaction work is additive: preserve this L2
  implementation, then verify compact shared owners before final measurements.
