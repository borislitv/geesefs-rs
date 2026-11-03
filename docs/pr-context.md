# PR context — 2026-11-03

[PR2](https://github.com/borislitv/geesefs-rs/pull/2) remains open and draft,
targeting `main`, without auto-merge. Overall status is **UNACCEPTED**. The
authoritative requirements are [project brief](project-brief.md) and
[AGENTS.md](../AGENTS.md); detailed execution journals stay in ignored evidence.

## Current contract

- Linux/S3 readers and writers, with read-only nginx/rsync/golist serving as the
  first performance priority. The primary deployment target is LXC.
- Go 0.43.9, source `80c214ef14f620d93407454fc76ca01dd71fa25f`, is the primary
  reference. The unmerged `5c0884c` Go experiment is historical evidence only.
- Compare correctness-equivalent behavior. Do not reproduce stale permissions or
  weaker durability for speed. Go-equivalent bounded negative caching is accepted:
  read-only comparisons explicitly use 64 misses per directory and a one-minute
  stat TTL. The cache-size default remains zero.
- Use [paired confidence intervals](performance-methodology.md), at least eight
  independent mount pairs and genuine same-executable/configuration A/A controls.
  Sparse, biased or missing controls remain inconclusive.
- Reuse released public dependencies; local unsafe code, library forks, FUSE
  passthrough and disk-backed caching remain excluded from this stage.

## Delivered mechanisms and verification

The [architecture](architecture.md) and [history](change-history.md) explain cache
admission wakeups, directory-local invalidation, bounded LIST continuation, exact
metadata observation age, per-open read history, upload/reply ownership and safe
notifications. The retained rename correction removes a reproduced lock cycle.
Lookup-overlap/cached-observation trials remain absent and inconclusive; inline
first polling is rejected for receive-thread liveness. Additional receive loops
retain independent allocation/rate objections.

Explicit `allow_other` now uses supported fuser session access while preserving
kernel permission checks and the owner-only default. The public
[cross-UID fixture](testing.md#cross-uid-mount-access--2026-11-02) verifies readable
bytes, default denial and private-file denial, including an omitted-wiring fault.
Earlier access failures and performance measurements describe their frozen
pre-fix executable, not this source.

## Scoped serving trial — 2026-11-02

The operator explicitly authorized replacing one read-only served reader with
the source-matching `allow_other` build. The trial retains traffic and the existing
payload/entry limits. This addressed exception does not accept the implementation
generally or authorize changes to other readers and writers.

On 2026-11-03 a separate operator authorization extends the trial to a second
read-only reader, retaining the same executable and budgets. Its ordinary
unmount/consumer handoff, independent peer-host HTTPS and rsync byte controls,
consumer mount namespaces and healthy recovery no-op pass. This is addressed
operational coverage, not comparative memory/performance or general acceptance.
Its initial passive window also retains one `open()` EIO/HTTP 500; subsequent
native-object and served-HEAD checks pass, without establishing the transient
failure's cause or an error-free serving claim.

The operator subsequently authorizes a third read-only reader on the same build
and budgets. Its first ordinary-unmount refusal preserves the old reader and
restores consumers. An archived continuation verifies empty consumer cgroups
before ordinary unmount, then passes complete HTTPS/Range/rsync bytes, consumer
namespaces and healthy recovery no-op. The earlier two readers remain serving.
These controls do not erase retained HTTP/FUSE errors or establish sustained
compatibility, writer or failure-injection acceptance.

The third reader's flushed five-minute canonical common-path 404 rate rises
from 58/5402 to 178/5361. Current native/served HEAD controls on all 65 newly
observed missing URIs agree on absence, including one separately retained native
TLS timeout followed by 404. The second reader's 17 checked URIs also agree;
the first reader's 62 distinct checked URIs now yield 58 missing pairs and four
present pairs. No checked URI has native 200 with served 404. Request populations
and generations differ; these existence checks do not prove historical byte
parity, explain every 404 or establish a causal non-regression.

The first reader's historical post window retains EAGAIN/HTTP 500, and a later
window has two `pread()` EIOs despite zero logged 5xx. That object's native short
Range independently returns backend InternalError/500 while served Range times
out. The second reader also retains an upstream connection timeout. A successful
HEAD or logged 200/206 is insufficient to establish complete payload delivery.

External failure recovery is configured to quiesce filesystem consumers, replace
only a verified disconnected mount and restore its consumers, excluding native-S3
listings. Deployment-helper regressions and a real healthy-event no-op pass;
production crash/OOM recovery was not injected and remains unobserved end to end.

Passive successful-request comparisons show a lower upper tail but a higher
aggregate median under differing cache/load conditions. HTTP 500 and `open`/`pread`
`EAGAIN` errors remain correctness gaps. FUSE admission pressure and metadata race
checks can both return that errno; their live contributions are not attributed.
These before/after observations do not replace independent paired acceptance.

Anonymous memory continues growing across later samples, with a variable rate.
Live cache/namespace occupancy, eviction and allocator retention are not attributed;
neither a leak nor its absence is established. A short interval does not prove a
sustained plateau.
Cache enlargement to 64 GiB is deferred pending that evidence. Deployment
identities, raw traces and operational journals remain private.

## Remaining work

- [Read-only target measurements](performance-read-shape.md) and
  [listing controls](performance-listing-diagnosis.md) cover transformed local
  workloads. The bounded real-S3 attempt has only five complete blocks; it cannot
  pass the eight-pair gate. No short-window memory result proves a plateau.
- Verify the exact target profile, service UIDs, signing/extended metadata and
  sustained large-namespace memory envelope before general deployment acceptance.
- Continue the [specific unresolved performance/compatibility work](next-work.md).
  Whole-file CRAP, broader mutation and the public generic matrix remain open.
- FUSE io_uring is an [upstream watch](fuse-io-uring-assessment.md), not an active
  integration or another kernel-installation task.

Raw traces, source/binary manifests, original failures and the pre-cleanup document
archive remain under ignored `benchmark-results/private/`. Use new result labels;
receipts are immutable. Before delivery, inspect status, final diff and PR
draft/auto-merge state, verify checks/provenance and update the existing report.
Private operational identifiers and credentials do not belong in public source.
