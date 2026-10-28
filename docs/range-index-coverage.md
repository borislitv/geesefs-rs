# Range-index coverage filtering

## Mechanism

[Absolute CPU profiles](read-cost-attribution.md) identify repeated cache
predecessor lookups as a concrete application cost. Previously the index stored
only replacement stamps: every predecessor required another BTree root search
and a Moka `get` to discover that its bytes did not cover a request. Successful
but unusable gets cloned payload owners and recorded cache popularity.

Store each immutable block's length beside its stamp in the existing bounded
index. Use one reverse BTree range iterator to skip spans that cannot contain
the requested bytes, then consult Moka only for a covering candidate. Continue
to check the returned block's real bounds; the index and Moka are not a single
transaction under concurrent clones/deferred eviction. An older overlapping
block may cover a range that the nearest short block cannot.

The index retains no payload owner. Stamp-protected replacement, entry limits,
generation isolation, gaps, full backing charges, FIFO admission before borrow,
conditional GET validation, final EOF and writer/fsync paths remain required.
No cache/allocator tuning, protocol, dependency, unsafe code or extra GET is added.

## Verification boundary

Three public `ReadCache` tests cover overlapping short spans, replacement growth
and shrinkage with an old pinned view, gap/entry eviction and the last
representable offset. They pass on the unchanged source first: this is a
behavior-preserving optimization, so no artificial functional red is claimed.
The retained matched performance deficit is the performance red. Independent
index-length, exact-end and stale-eviction-stamp faults must fail the observable
byte/coverage/ownership assertions before restoring and checking the candidate.

Full Kubernetes checks and an uninstrumented, exact-source comparison against
initialized `e000ecf` and both frozen Go binaries are required. Until those
measurements are reconciled, no speed, tail-latency or whole-memory win is claimed.

The [quality receipt](range-index-coverage-quality-20261028.json) verifies 184 Rust
and 37 Python cases, formatting, all-target/all-feature warning-denying Clippy,
the 336-package public-source audit, three assertion faults and one selected tool
mutant. Twenty-eight cache sites remain unselected; coverage/CRAP and three prior
duplication candidates remain open. It binds all 25 compiled inputs to release
SHA256 `e39a70ba3e8c492398005a13a95aebd7242c27c6badfc02b3ed0df7a1c70e047`.
