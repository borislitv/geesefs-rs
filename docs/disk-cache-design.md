# Persistent payload cache and active read ownership

The application has no retained clean RAM payload cache. The 2026-11-17 contract
uses unmodified crates.io Foyer with its native hash index and enabled reinsertion.
This supersedes the former fork and L1 victim-cache design. Full compatibility,
inventory turnover and performance/production acceptance remain separate gates.

## Data path

```text
filesystem read -> active same-generation fill reuse
                -> eligible validated Foyer record
                -> conditional S3 range -> validated fill -> optional Foyer enqueue
```

There is no `ReadCache`, clean-payload LRU/range index, flush-loan registry or
cache-hit-triggered refill. Independent reuse executes Foyer's positioned file
read, deserialization and integrity checks. The backing file uses buffered I/O
and can benefit from the kernel page cache; this is not zero-copy I/O or a second
retained decoded RAM tier. FUSE's inode page cache is a separate mechanism.

Authoritative metadata remains in RAM with its existing TTL, bounded negative
cache and optional metadata admission. Dirty extents, upload bodies, conditional
commit and fsync recovery retain their existing ownership and durability rules.
The writer flush reserve is never lent to clean readers.

## Supported library reuse

The locked Foyer packages are version 0.22.6 from crates.io. Foyer owns persistent
record I/O, the sharded HashMap, regions, checksums, recovery and reclamation.
The adapter uses public `Store::{load,enqueue,wait}` with a bounded copy/job pool,
region picker and passive memory `Piece` source. Void enqueue is an offer, not an
admission or publication receipt; `may_contains` is only a hint, including pending
Keeper work. No custom index or shadow occupancy map is maintained.
The selected runtime engine is Psync, not io-uring merely because that crate is
present in the lockfile.

Foyer memory `get_or_fetch` coalesces exact keys, starts an independently owned
producer and exposes the result after producer completion. That contract does
not provide overlapping byte ranges, early validated-prefix replies or
caller-owned cancellation. The narrow active-fill coordinator therefore remains;
it is not another persistent cache engine. See the
[dependency decision](dependency-decisions.md#removal-of-retained-application-payload-cache--2026-11-14).

## Active buffers and progress

Admit the complete source/output/disk fallback plan before allocating payload.
Charged `Block` clones and slices share immutable ownership and retain the full
backing allocation's charge. A borrowed FUSE reply uses a covering backing
directly; fragmented results use their already admitted composition output.
Do not add a whole-object staging allocation or return incomplete bytes.

Same-generation overlapping readers can subscribe to a caller-owned producer.
Its validated chunks live only until producer completion and subscribed-consumer
cleanup. The inode registry retires the interval on success, error and cancellation;
disjoint ranges can progress independently. Followers reserve output before
waiting with published owners. If that reservation would queue, release the
subscription before waiting: retaining its payload behind an older large FIFO
request can deadlock that request. Generation checks still apply before reuse.
While the producer is still admitting its complete window, followers instead wait
without output permits so they cannot consume the producer's required capacity.

Read-ahead remains bounded by the existing byte/task budget. A per-handle bounded
planned-interval hint avoids sliding an entire speculative window on every small
sequential read. It is not proof of cache coverage: required bytes always consult
Foyer/source, including after cancellation or a dropped population. Without an
eligible funded persistent tier, fetch only the required span. Metadata and
writer overlays must not trigger speculative clean-payload retention.
Seeks outside either boundary retain adaptive read-ahead rather than treating the
entire earlier file prefix as already planned.

## Population and fallback

The first eligible validated source fill starts a proven-new deferred tier.
Existing inventory recovers on attachment. Mere lookup, metrics scraping and
writer flushes do not start or populate an unused fresh tier.

Populate from validated source chunks independently of RAM pressure. Split larger
backings into records of at most 128 KiB without retaining a whole backing in the
adapter. An admitted record receives its bounded visible copy and existing
Foyer serialization/submission funding. Loaded disk-origin blocks are not copied
back into the persistent store.

Population is best-effort. One source window has a cumulative admission-wait
allowance of at most `min(request_timeout, 250 ms)`, not a renewed allowance per
record. Source transfer time does not consume that admission allowance. A delayed
or failed optional tier can drop population; subsequent required reads must still
return exact source bytes. Startup and actual disk work retain their funded owners
through cancellation and are joined by shutdown. Required disk reads have their
own bounded wait; timeout leaves actual I/O owned until its real completion.

Release namespace and inode locks before copying or doing I/O. Recheck namespace,
inode/generation and immutable source identity around awaited work. Stale or dirty
chunks cannot populate a replacement generation. Writer admission/fsync remains
independent of optional cache population.

## Identity, selection and freshness

The regex selects regular files by their UTF-8 mount-relative path with a leading
`/`. Keep the [checked-in mirror pattern](mirror-payload.regex), existing CLI and
resource settings. No general configuration file or payload TTL is added.

Persistent keys contain a format version, backend namespace, exact object key,
opaque ETag, object size and covered offset/length. Process-local inode numbers
and hashed read generations are not persistent identities. The backing filename
separates the larger-region [native media layouts](disk-cache-operations.md#cache-geometry-and-cold-transition)
from legacy media and from each other.
Validate identity, length and integrity before returning a record. Short EOF/partial
coverage is explicit; a missing span is never a zero-filled hit. No source GET is
extended solely to complete a cache record.

Payload has no inactivity expiry. Metadata TTL, permissions, rename/delete and
local generation rules remain authoritative. ETag/size changes invalidate old
payload reuse; unchanged ETag does not bypass permission or metadata validation.
Recovery restores inventory, not fresh authoritative S3 attributes, and must not
HEAD/GET every record. The scoped single-user downloaded-copy contract does not
add a GET merely to reauthorize KMS on each local hit.

## Capacity, policy and observations

Fixed Foyer funding is deducted from the existing data lane. Application reads,
copies and jobs are bounded; native serialization/index/recovery allocations have
nominal allowances rather than enforced native caps. Small records can exceed
the nominal full-record entry count. Neither admission nor funding is a heap,
RSS or whole-cgroup limit. Report allocator, process and cgroup/file/kernel memory
independently; removing retained payload does not prove a measured memory win.

Bounded approximate-recency/second-chance region selection and hot-record
reinsertion use public library hooks. Some hits can be lost during best-effort
recovery after reinsertion; returned bytes must still validate. Region reclamation
is not a per-record LRU or a count of complete cached files. An optional enqueue
waits on the store's flush and reclaim work, not a per-record write receipt. If
that wait lasts 30 seconds, the adapter reports a spill timeout and keeps serving
validated hits. Each late wait keeps its original spill permit: more fills can
be dropped under pressure, but do not queue unbounded native waits. Progress
releases the permit without reopening the store. A persistent shortage of clean
regions does not imply that existing records are unreadable.

Foyer receives a Handle to a dedicated Tokio runtime. Close stops admission and
cancels optional drain observers, joins application work, then gives native close
its own five-second drain budget before shutting down that runtime.
Device, directory lock and funding owners survive actual blocking I/O, including
cancelled observers. A caller timeout does not release a live operation's owners.

The [exporter contract](metrics.md) distinguishes requested `inflight`, `l2`,
`s3` and `overlay` delivery from all consumed SDK GET body bytes, adapter I/O and
population operations. Retention/loan/eviction gauges and the old `l1` delivery
label are removed. `admitted_demotions` counts admitted validated-fill copies;
`engine_offered_demotions` counts native enqueue attempts, not durable records.
Dashboard queries require the matching schema and complete same-reader cohorts;
missing series are not zeros. Neither descriptive byte ratios nor passing unit
tests establish causal savings, full coverage or production acceptance.
