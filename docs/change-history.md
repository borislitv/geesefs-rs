# Change history and reasons

Concise engineering context for the experimental Linux/S3 rewrite. Compatibility
and comparative performance remain **UNACCEPTED**. Detailed journals, failures,
source/binary manifests and pre-cleanup documents stay in ignored evidence;
measurement pages retain their original tables and source scopes.

## Second addressed reader trial — 2026-11-03

A separate operator authorization extends the read-only serving trial to one
additional reader with the same source-matching executable and cache/resource
budgets. Ordinary unmount, consumer namespace propagation, independent peer-host
HTTPS and rsync bytes and healthy recovery no-op pass. Preserve the prior live
error/anonymous-growth gaps and the second reader's transient EIO/HTTP 500;
later successful controls do not attribute that failure. A cold new cache is
not a fair memory comparison.
[Combined scope and limits](pr-context.md#scoped-serving-trial--2026-11-02).

## Scoped serving trial and recovery boundary — 2026-11-02

The operator authorized one read-only served-reader replacement, separately from
general acceptance. Deployment recovery now recognizes the addressed Rust mount,
quiesces filesystem consumers and preserves native-S3 listings; healthy delivery
does not restart the reader. Actual production crash/OOM recovery is unobserved.
Live errors and continued anonymous-memory growth still need attribution; defer
cache enlargement.
[Trial scope and remaining uncertainty](pr-context.md#scoped-serving-trial--2026-11-02).

## Cross-UID mount access — 2026-11-02

Owner-only fixtures missed a serving contract: the frozen Rust binary rejected
`allow_other` and denied another UID even on a readable file. Accept the explicit
option and select supported fuser `SessionACL::All`; keep the default owner-only
policy and kernel `default_permissions`. Preserve other supported mount options
and reject unknown ones.

The parser regression fails before the change. An omitted-ACL wiring fault then
fails the real-FUSE readable-byte assertion; restored wiring passes alongside
owner-only and mode-0600 denial controls. Both allocator suites, explicit native
access checks, formatting, Clippy and 99 Python tests pass. Scoped quality passes
do not close broader-file CRAP or production acceptance. [Verification](testing.md).

## Reference, cache policy and statistical contract — 2026-11-02

Go 0.43.9 is the primary acceptance reference. The unmerged Go experiment remains
historical evidence, including favorable and adverse measurements. Go-relative
superiority requires equivalent correctness: demonstrated stale permission
metadata or weaker durability is not behavior to reproduce for speed.

The operator accepts Go-equivalent bounded negative caching. Use explicit Rust
negative64 and one-minute stat TTL in read-only comparisons; 64 is a per-directory
capacity, not seconds. The cache default remains zero. Eight controlled blocks
attribute the default listing loss to five absent-name checks/15 S3 requests;
the existing option removes those requests. The separate five-second expiry
probe demonstrates accepted external-create delay. [Listing diagnosis](performance-listing-diagnosis.md).

Withdraw strict per-pair/all-metric inference from short trials. Require paired
confidence intervals, eight independent mounts and genuine same-executable A/A
controls; superiority and practical non-inferiority are separate decisions.
Lookup-overlap and cached-observation trials remain absent/inconclusive.
[Protocol](performance-methodology.md).

## Rename notification deadlock and dispatch controls — 2026-11-02

A native four-client rename regression hangs before correction. Synchronous
post-reply entry notifications can wait for a directory operation while holding
the notifier lock. Linux/VFS already maintains rename dentries; remove the
redundant notifications while retaining last-close inode invalidation and durable
unmount. No extra executor hides the cycle.

Supported receive-loop controls retain allocation/rate objections. Inline first
polling has inconclusive mixed performance after A/A calibration, but a native
contended read stalls the sole receive thread: reject it on liveness. Trial
runtime/tests/CLI remain absent. Backend errno/fallocate helpers preserve behavior.
[Measurements and public-source analysis](performance-classic-dispatch.md).

## Read-only target and bounded native verification — 2026-11-02

The clarified primary target is LXC serving nginx/rsync/golist, with read-only
performance first and writer correctness preserved. Isolated actual-kernel
nginx/rsync checks pass byte/Range/EOF/416 and read-only contracts. Buffered shared
mmap passes; direct I/O returns `ENODEV`. Corrupt-byte controls fail correctly.
[Target scope](performance-target-preflight.md).

The transformed read-shape comparison completes eight five-arm blocks. A separate
guarded real-S3 attempt stops after five complete blocks on an upstream HEAD
timeout, below acceptance requirements. Retain cumulative budgets, setup failures
and cleanup receipts. An audit also finds stale disposable rename mounts despite
failed units: verify actual task exit, ordinary unmount and unchanged unrelated
mounts rather than trusting unit status. [Read controls](performance-read-shape.md).

## Metadata observation age and bounded directory work — 2026-11-01

Standard LIST renewed cached user metadata's observation time, extending permission
TTL without fresh metadata. A source-age regression and native mode-only update
fail before correction. Carry actual observation timestamps through bounded
selection/type resolution and return remaining TTL. Protect newer HEAD/commit
observations. Equal-TTL probes independently show stale Go modes at unchanged
content ETag; lower RPC counts are not equivalent freshness.
[Diagnosis](metadata-expiry-diagnosis.md), [corrected-source measurements](performance-observation-age.md).

Consumed directory prefixes were repeatedly copied/validated and could trigger
extra LIST after inode eviction. Resume only the unconsumed suffix without
extending TTL or pinning inodes. Then align bounded FUSE consumption with the
backend page size: an 8192-name traversal drops from 65 LIST calls to the expected
10, including EOF. Kernel-buffer continuation, accurate types, lookup ownership
and error boundaries remain tested. The READDIRPLUS_AUTO trial stays absent.
[Directory measurements](performance-directory-remainders.md).

## Owned uploads and clean-close cache retirement — 2026-11-01

A retry regression sends 302 HTTP body bytes for nine dirty bytes because a known
contiguous body used the generic aws-chunked path. Reuse supported Bytes/Smithy
known-body APIs for covered extents, retaining charge/activity through the last
retry owner. Sparse/composed/remote streams, signing/checksum policy and
post-commit validation are unchanged. Byte/checksum/lifetime faults verify the
correction. [Owned-upload measurements](performance-owned-uploads.md).

Owned-superblock/page attribution identifies 256 MiB of clean payload pages left
after close although the next open discards them without `FOPEN_KEEP_CACHE`.
Use supported best-effort inode invalidation after last clean close; release
userspace locks before notification and preserve mmap/reopen owners. The retained-
page regression fails before and passes after correction. Lower temporal file-
cache means do not prove process-memory superiority or a plateau.
[Integrated measurements](performance-integrated.md).

## Freshness, handles and admission progress — 2026-10-31

FUSE/HTTP joins place late metadata bursts inside LOOKUP: cached name pages retain
older child attributes. Validate freshness/type/binding, refresh authoritative
extended LIST and return remaining kernel TTL. Supported bounded READDIRPLUS
transfers lookup ownership only on successful replies. V1 marker/V2 start-after
avoid repeated standard-LIST prefix scans while opaque continuation retains
priority. Preserve accurate common-prefix metadata and lexical boundaries.

Move adaptive history to each open handle; cache/fills/budgets remain shared.
Reserve handles before create so `EMFILE` cannot leave a dirty name. Clean fsync
preserves cache loans when no generation needs uploading. Linux hardlink-as-symlink
fails after mutation in the reference-style path; Rust now rejects unsupported
LINK before mutation. [Architecture](architecture.md), [compatibility](compatibility.md).

A rare 30-second read is reproduced by a gated public test: required admission
does not revisit cache owners published after its first reclamation sweep.
Notify-driven reclamation retains FIFO place and unchanged budgets. Separately,
directory-local epochs prevent unrelated writes invalidating all cached listings;
in-flight binding checks preserve rename/delete correctness.
[Native checkpoint](performance.md#native-vm-admissiondirectory-checkpoint--2026-10-31).

## Read path and benchmark accounting — 2026-10-27–30

Coalesce overlapping generation-scoped fills and publish into the current cache
instead of an orphan clone. Charged immutable views avoid redundant cache/reply
copies; incremental SDK body publication retains checksum barriers. Drop consumed
transport owners promptly and transfer charges to actual fragment backings.
Maintained `lru` supplies synchronous payload eviction; metadata stays in Moka.

Cache-hit progress now plans refill after reply publication within the existing
owned lifetime. Speculation cannot evict another reader's unread backing; required
admission retains its policy. Reopening resets promoted history without discarding
payload. Required admission can borrow idle flush capacity, revocable by uploads;
owned replies cannot export a revocable backing. Byte-budget/lifetime regressions
fail for the original mechanisms. [Historical cache-policy measurements](performance-cache-policy.md).

The runner now warms the actual operation, excludes cold superiority from warm
acceptance and reports complete temporal CPU/memory scopes. A bounded Go observer
reads live HeapAlloc without requesting GC; missing samples stay missing. Go's
open-handle fsync recovery failures remain failures, distinct from recovery after
close. ThinLTO/System-allocator comparisons and observer-on/off partial attempts
retain their exact configurations. None establishes all-workload acceptance.

## Dependency reuse and suspended transport work

Reuse public fuser, AWS SDK, Tokio and maintained ownership/cache APIs instead of
custom protocols or unsafe adapters. Actual-service coverage remains incomplete.
Linux 7.3 pool/zero-copy research and standalone C controls do not supply a fitting
released Rust transport. FUSE io_uring stays an [upstream watch](fuse-io-uring-assessment.md);
host boot/package recipes and execution checklists are archived outside the PR.
