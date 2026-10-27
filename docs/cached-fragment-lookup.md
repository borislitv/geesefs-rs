# Cached-fragment lookup continuation

## Mechanism and preserved contract

The [body-prefix matched run](streamed-read-body.md) improves p99 against its
exact Rust prior while regressing throughput, CPU and final-fifth RSS. Separate
validated profiles and code inspection identify a concrete cache bookkeeping
cost: a request spanning small fragments first searches older predecessors for a
single block covering the whole reply, then repeats coverage lookup to compose it.

`ReadCache::read` now locates the starting byte once. If its immutable block covers
the full requested range, return the same charged view. Otherwise join subsequent
same-inode/generation fragments into the already admitted output. A gap still
returns a miss; byte bounds and complete output remain mandatory. This removes
the futile whole-span predecessor scan without replacing Moka, changing the cache
budget or changing conditional GET publication/checksum semantics.

Admission and pressure reclamation remain before borrowing cached allocations.
Queued fragments and views retain complete GET admission and physical allocation
ownership. Writer materialization and generation changes retain their existing
paths. No allocator tuning, extra GET, detached producer or dependency is added.

## Verification

The unchanged-source baseline and changed source pass isolated Kubernetes
verification: 178 Rust cases with no failures/ignored cases, 14 source/oracle and
23 benchmark Python cases, formatting, warning-denying all-target/all-feature
Clippy and the 336-package public-source audit. The restored 56-case affected
suite preserves real HTTP cached-span/generation checks, pinned-view ownership,
FIFO-pressure progress, concurrent windows and cancellation.

Seven independently inserted faults still fail intended behavior assertions:
deferred publication, missing full-GET charge, lost cached-span coverage, premature
checksum publication, ignored short reads, hidden late failures and lost offsets.
One selected cache completion equality mutant is killed. The tool scans 27 sites;
the other 26 were not executed. Package clean is explicit during mutations and
before final full verification to avoid stale-mtime artifacts.

The [quality receipt](cached-fragment-lookup-quality-20261027.json) binds actual
source/fault/tool hashes and check counts. Cached composition remains CC 6;
filesystem read-inner CC 49, three prior duplication candidates and unmeasured
coverage/CRAP remain open. No private-method call-count or artificial timing test
is presented as a functional regression: the retained matched loss is the
performance red, and it requires a new measured comparison.

## Performance gate

The candidate must be frozen and compared against exact body-prefix source
`20a9dcd2f125e30f251e02510fe3f44501a290f6` and both unchanged Go references on the
same sealed native fixture. Preserve budgets, default allocator, generation/full
bytes/metadata checks, repeated orders and complete CPU/memory/tail metrics.
Prior adverse comparisons remain retained. A smaller source diff and green unit
tests do not establish a speed or whole-memory win. Overall compatibility and
comparative performance remain **UNACCEPTED**.
