# Progress tracker for disk-cache-plan.md

Status: implementation authorized; checks are local. Deployment and target-host
acceptance are separate later work.

## Execution stages

[ ] Stage 1: Qualify the storage dependency and freeze resource contracts
[ ] Stage 2: Add CLI configuration and regex policy
[ ] Stage 3: Verify CLI and policy boundaries
[ ] Stage 4: Implement the isolated L2 adapter
[ ] Stage 5: Verify capacity eviction, failures and recovery
[ ] Stage 6: Integrate demotion, reads, lifecycle and metrics
[ ] Stage 7: Verify cross-tier correctness and resource progress
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
