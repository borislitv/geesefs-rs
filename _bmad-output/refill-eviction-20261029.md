# Cross-reader refill eviction investigation

## Observations and scope

The first refill candidate `6e165250e48bbf8f0f9e6c916fecc48f8586033b`
improves the matched warm throughput and tails relative to prior Rust, but still
loses p95/p99 to both Go references. Its mean warm excess response payload is
88.796175 MB versus 13.817834 MB previously. These adverse results remain in
`docs/read-ahead-refill-results-20261029.json` pending independent export checks.

Direct socket traces show repeated payload in `wide/file-0124` and
`wide/file-0128`. Joining their request timestamps to client read spans places
the repeated GETs at current demand positions, not a second traversal. This
does not identify every eviction caller, nor prove a single cause for all tails.

## Actions and reasons

1. Rechecked project contracts, current branch and fixture identities. The former
   build Pod had completed normally; started a new bounded disposable build Pod.
   The original native fixture and its collected first-candidate evidence remain
   available without replacing binaries or existing phase paths.
2. Read `ReadCache::covering`, optional and required admission, streamed insertion,
   and the pinned public Go `readRanges` and buffer queue. The current optional
   protection is inode-local; it can discard a different reader's unread block.
   Recorded reuse and upstream limitations before changing runtime code.
3. Added a public two-reader regression: two 2 MiB windows share a 5 MiB lane;
   optional refill of one reader must not cause a GET on the other's next read.
   Against unchanged `6e16525` runtime, correct bytes pass, then the GET-count
   assertion fails: actual 4, expected 3. Cargo exits 101, one test failed.

```bash
cargo test --locked --offline --test read_refill optional_refill_preserves_another_readers_unconsumed_backing -- --exact --nocapture
```

4. Add a contiguous-consumption hint to each existing bounded cache-index entry.
   Optional admission stops at a foreign unread LRU owner. Required admission
   remains free to evict it, preserving bounded progress and FIFO. Reset hints
   on insertion/replacement; no additional background task or reader registry.
5. Add positive reclamation and disjoint-tail regressions. The focused suite
   passes 81 tests, including 12 refill cases, against the candidate.
6. Eleven deliberate faults, including disabled foreign protection, disabled
   consumption tracking and treating a disjoint tail as consumed, fail public
   assertions. Mutator also kills the selected gap-boundary mutant. Restore
   original source hashes and clean the package before full checks.
7. Kubernetes verification passes 204 Rust tests, 37 Python tests, rustfmt,
   all-target/all-feature Clippy with `-D warnings`, and public-source audit.
   Crapper reports `consumed` CC 2 and `evict_consumed_lru` CC 4; the existing
   loader is CC 20. Coverage/CRAP remain unavailable. Dryer retains the same
   three existing duplicate candidates; broad mutations remain open.
8. Build the System-feature binary for a new matched comparison against the
   first refill candidate and both unchanged Go references. Preserve the first
   comparison's outputs and source-bound measurements separately.

## Acceptance

UNACCEPTED. The public failure confirms the cross-reader defect, not its share
of measured payload amplification. The candidate needs native comparison, and
remaining Go tail/CPU and broader compatibility/workload gates remain open.
