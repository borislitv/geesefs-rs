# Bounded metadata-page reuse

The real-object read-only diagnostic at source `ba405d3` completed all 18 phases
correctly, but Rust was slower. Its three fresh runs spent a summed 54.95–55.41
worker-seconds in file `stat`, versus 18.07–18.45 for experimental Go and
26.24–29.96 for packaged Go. These sums overlap across four readers and are not
wall time. Warm directory listings took 3.575–3.964 seconds, versus 0.036–0.052
and 0.491–0.567 respectively. See the [complete diagnostic](k8s-readonly-results-20261027.json).

Public Go `core/dir.go` batches unknown-name lookups through listing preloading.
The Rust path instead used individual HEADs, sequential implicit-directory
probes and a new backend LIST for every repeated directory page. These are
specific differences to test; the whole throughput/p99 regression is not
automatically explained or resolved by changing them.

## Repeated directory pages

The existing Moka synchronous weighted cache retains returned pages keyed by
directory inode, namespace mutation epoch, lexical cursor and requested limit.
Each returned name consumes one unit of the unchanged entry-limit capacity;
empty pages consume one rather than having unbounded zero weight.

- TTL starts before fetching, not when a potentially slow fetch completes.
- Create, unlink and rename change the existing namespace epoch.
- Reuse checks that every listed inode still exists: READDIR does not retain
  lookup references and namespace pressure can evict them.
- Only bounded pages are retained. Raw/unresolved rename traversal is not cached.
- Zero stat TTL disables reuse, including empty pages.

## Extended lookup preloading

An unknown name in extended-listing mode may preload one immediate directory
page. The existing backend caps a page at 1,000 entries. Only authoritative file
metadata with an ETag is installed into the existing bounded namespace. Existing
inodes, dirty state, directory-marker semantics and normal HEAD/implicit-directory
probes are retained. Standard metadata, a missing requested name or speculative
failure never establishes a negative lookup.

A per-inode Tokio mutex coalesces overlapping attempts. A bounded Moka timestamp
cache records attempts, including failed ones, without storing another inode tree.
Attempt timestamps are not negative directory coverage and need not be discarded
on every local create. Namespace epoch validation still rejects an in-flight
snapshot that overlaps a local namespace mutation. Published inode freshness
starts before the listing. Existing positive and negative lookup TTLs are retained.

## Regression evidence

The new directory-page regression failed on the preceding implementation with
four LISTs instead of two. Removing each epoch, TTL and retained-inode guard
independently failed its dedicated public-interface regression. The lookup
preloading regression also failed before implementation. Deliberately disabling
its in-flight epoch check, accepting absent metadata as authoritative, and removing
attempt serialization each failed the corresponding stale-generation, symlink-type
or explicitly polled overlap test. All faults were restored.

Additional cases cover an object beyond the first page, local dirty bytes during
a speculative listing, cancelled ownership, repeated local creation, zero TTL and
a denied LIST with successful authoritative HEAD/full-payload reads. Verification
runs in an isolated Kubernetes build Pod without S3 credentials. These tests prove
bounded behavior and preservation checks, not a measured whole-filesystem win.
