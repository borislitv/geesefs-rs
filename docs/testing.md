# Testing and verification boundary

**Experimental read/write filesystem; compatibility and performance unaccepted.**
Build and test in isolated fixtures with disposable data. The operator also
authorizes the OpenCode VM for the 2026-10-31 native experiments. Never use
served mounts or production credentials. Use finite subprocess/operation deadlines.

## Directory ownership and progress — 2026-11-12

`tests/directory_pages.rs` retains a pending remainder and then a second cached
remainder across real count pressure. Neither owner can be evicted until its last
pin is released. `tests/metrics.rs` checks cached versus kernel-owned identities,
repeated references, partial/last/duplicate/unknown FORGET and reacquisition.
Ten selected counter-transition mutants are killed; discovery is not a whole-file
mutation score. The new helper CC values are at most nine; coverage/CRAP are N/A.
Dryer retains three existing score-1.00 namespace-bookkeeping pairs.

The native nested fixture requires exact 524800-byte `du` at both 2048 and 128
inode bounds, actual small-bound eviction and unchanged open-anchor bytes. A
separate 1536-file fixture holds 1024 real files open under a 2048-inode bound,
forcing nonempty pressure prefixes without capacity exhaustion. Both metadata
charging modes require exact 1572864 bytes and preserved open bytes. Injecting
empty-remainder-as-EOF yields exit0/523264 bytes and fails the independent oracle;
restored source passes with one and three receivers.

The handle fixture first fails EMFILE with two request slots and 32 idle open
directories. Shared entry-bounded file/directory admission permits all 32, rejects
an extra directory or file with EMFILE, still delivers the existing directory
contents, and releases open ownership after unmount. This does not retrospectively
attribute every target EMFILE to the server; preserve client/resource evidence.

**Known progress limitation:** the strict 8192-file/2048-inode flat fixture still
fails ENOMEM. Its expected full size is 8388608 bytes; explicit ENOMEM is not a
pass. Invalidation is not a synchronous capacity-release handshake. Both attempted
pending-PLUS headroom and continuous finite notification batches remained RED and
were reverted. The three baseline-reproduced native failures documented below
remain separate; the whole opt-in native suite is not green.

Source-frozen default and all-feature suites each pass 526 active tests with 30
explicit opt-in ignores across 61 targets. The opt-in directory suite has 15 passes
and four failures with one/three default-allocator receivers and three
system-allocator receivers: the strict flat gate plus the three baseline gaps.
Both all-target denied-warning Clippy configurations, fmt, 16 project Python
tests, 159 benchmark tests (two native opt-in skips) and the 388-package
public-source audit pass. These scopes do not establish target traversal,
completed physical L2 recycling or sustained Go-relative acceptance.

```bash
timeout 180s cargo +1.99.1 test --locked --test directory_pages --test metrics
timeout 90s cargo +1.99.1 test --locked --test fuse_directory native_pressure_prefixes_advance_exhausted_pages_without_false_eof -- --ignored --exact --nocapture
GEESEFS_TEST_FUSE_THREADS=3 timeout 90s cargo +1.99.1 test --locked --test fuse_directory live_directory_handles_use_the_handle_budget_not_request_concurrency -- --ignored --exact --nocapture
timeout 180s cargo +1.99.1 test --locked --test fuse_directory -- --ignored --test-threads=1
```

## L2 region observations and mirror selection — 2026-11-12

The existing real-filesystem L2 metrics fixture first fails because the region
capacity gauge is missing. It then verifies an empty preallocated 16-MiB file has
only fifteen 1-MiB data regions, one populated writer region consumes its entire
reservation, and recovery restores occupancy independently of reset I/O counters.
An empty drain fence can reserve another writer region without writing payload;
the test distinguishes that reservation from the recovered evictable region.
Prepared/disabled snapshots omit unavailable region gauges, and repeated scrapes
do not perform storage/S3 I/O or change ownership.

Real fixed-pool pressure exercises repeated recycling. A gated real-file reclaimer
shows one in-progress region, then clean reusable capacity after actual completion.
Removing decrements fails the fifteen-region conservation assertion; dropping the
reclaiming gauge fails the in-progress assertion. Both fault tests clean up actual
owners before asserting. Four selected occupancy-arithmetic mutants are reported
killed; 62 discovered sites are not a whole-file mutation score. The new bridge and
region exporter have CC at most three; coverage/CRAP remain unmeasured. After
delegating required no-op trait methods to mixtrics, Dryer finds no candidates at
score >=0.8 in the two scoped files.

The checked-in mirror regex is parsed through the real CLI. The initial deb/rpm
policy fails the ISO example; the expanded policy accepts disk-image/archive and
package examples while rejecting metadata, sidecars, partial names and a matching
directory with a nonmatching child. This is filename selection, not a size filter.

```bash
timeout 180s cargo +1.99.1 test --locked --test disk_cache --test disk_cache_metrics --test disk_cache_lazy --test disk_cache_config
timeout 180s cargo +1.99.1 test --locked --all-features --test disk_cache --test disk_cache_metrics --test disk_cache_lazy --test disk_cache_config
```

No measured target improvement or complete-object residency follows from these
fixtures. Distinct-object counts and useful live payload byte totals remain
unavailable. The ongoing target workload retains its source/configuration until
its own recorded completion/restart phase.

## FUSE admission backpressure — 2026-11-12

`tests/fuse_admission.rs` mounts disposable real FUSE fixtures. With one request
slot, an early read reply leaves a gated read-ahead owner active; a second native
buffered read or safe rustix sendfile requests independently specified L1 bytes.
Both fail with EIO on the pre-fix adapter and pass with receiver-thread
backpressure, including one and three fuser receivers. The fixture disables only
kernel speculative readahead; application read-ahead and its ownership remain on.

Protocol tests use fuser's actual parser/reply owners to check queued lookup
success, a shared admission/execution deadline on an explicitly driven paused
current-thread runtime, expired WRITE input cleanup without mutation, O_SYNC
failure followed by successful fsync recovery, and shutdown queued behind saturated
admission. DESTROY acknowledgement precedes the filesystem drain hook, so the
fixture separately joins actual shutdown and rejects extra replies. Existing
post-reply/deadline/logging checks remain; warning fixtures use real namespace
EAGAIN conflicts rather than obsolete overload rejection.

The pre-existing early-reply test raced a real 100-ms deadline during a parallel
full-suite run: its live-owner observation occurred after valid cancellation.
Keep the deadline and assertions, synchronize on the actual gated tail, then
advance the paused clock to expiry. This is deterministic cancellation evidence,
not a latency measurement or an increased timeout.

Three source-addressed fault controls prove sensitivity: changing expiry `>=`
to `>` admits a WRITE at its deadline; renewing the execution budget after waiting
violates the total deadline; dropping the permit before job completion violates
both native ownership assertions. All fail the intended assertions, then the
correct source is restored. Mutator's selected boundary result is corroborated by
the explicit assertion log; its 187 discovered sites are not 187 executed tests.

Final source-frozen default and all-feature suites each pass **522 tests**, with
26 explicit opt-in ignores across 61 targets. Native admission passes separately
with one/three receivers and with system-allocator features. All-target Clippy
with denied warnings passes in both configurations, as do fmt, the 16 project
Python tests, 159 benchmark tests (two native opt-in skips) and the 388-package
public-source audit. Crapper reports CC5/6/7 for `spawn_traced`/`admit`/`run_job`;
coverage and CRAP remain N/A. Dryer finds no score >=0.8 candidates in the two
changed runtime files. The native limitations below remain separate failures.

```bash
timeout 180s cargo +1.99.1 test --locked --test fuse_admission -- --ignored --test-threads=1
GEESEFS_TEST_FUSE_THREADS=3 timeout 180s cargo +1.99.1 test --locked --test fuse_admission -- --ignored --test-threads=1
timeout 180s cargo +1.99.1 test --locked --test fuse_deadline_review --test fuse_lifecycle --test fuse_errors
timeout 180s cargo +1.99.1 test --locked --test fuse_directory -- --ignored --test-threads=1
```

**Known native limitation:** on kernel `7.3.1-070301-generic`, the broader directory
suite has 12 passes and three failures with both one and three receivers. A pristine
`420f30330a8b69d23241805722cce7cc26c3be72` worktree independently reproduces the
same three assertions with three receivers:

- `failed_plus_buffer_does_not_transfer_unsent_lookup_references`: unexpected
  successful directory iteration despite the injected metadata failure;
- `plus_common_prefixes_preserve_directory_marker_permissions_and_ownership`:
  zero observed lookup references where one is expected;
- `plus_pages_deliver_attributes_and_lookup_ownership_without_overlapping_lists`:
  11 root LIST calls instead of ten.

These checks remain unchanged and failing; this is not a fully passing native
compatibility suite. No new target deployment or sustained Go/A-A campaign is part
of this correction. The [HDD checkpoint](target-disk-cache-functional.md) retains
its historical failures independently of the local admission proof.

## Metadata compaction and persistent L2 — 2026-11-11

Default and system-allocator configurations pass 518 active tests, with 24
explicit opt-in native/diagnostic entries, plus denied-warning Clippy. Meaningful
regressions cover canonical/key owner lifetime, COW baseline/conflict handling,
postcommit metadata credit, stale-size admission, byte-pressure demotion, genuine
loan revocation and inherited directory-lock release. The allocation matrix uses
default jemalloc separately; an all-feature run is not that allocator evidence.
Further real-engine tests cover lazy new-media creation, maximum rounded record
admission, funded FIFO reads, actual cancelled-worker ownership, namespace/inode
contention, a parent rename during the wait and a slow missing prefix before warm
lookups. Independent fault controls fail the intended assertions before restoring
the reviewed source; successful setup alone is not failure sensitivity.
Validated disk-backing tests additionally reject redundant fragment writes,
incorrect retained offsets, invented partial coverage and a blanket disk-origin
demotion skip. The refill control proves live-index removal and persisted
readmission; it does not claim physical target-region recycling.
The combined pipeline overlaps bounded source gaps with healthy disk discovery
and transfers a decoded payload into its existing reservation without another
payload copy. Post-WOOF regressions cover an admission-local bounded namespace
scan after FORGET, finite exhaustion with interleaved admissions, and a recovered
32-block range whose first disk read times out. Source delivery completes while
that actual worker retains its slot/output/credit; healthy later lookups still
receive their own deadline after slow source backpressure. The engine gate exists
only in test builds. Public synchronous Namespace installation and speculative
preload finish their own finite scan; async publication remains bounded after
yielding pre-admission. New regressions first fail for the reproduced defects;
ten targeted injected faults are rejected before restoring the correct source.

```bash
timeout 1800s cargo +1.99.1 test --locked --all-targets --no-fail-fast
timeout 1800s cargo +1.99.1 test --locked --all-targets --all-features --no-fail-fast
timeout 1800s cargo +1.99.1 clippy --locked --all-targets --all-features -- -D warnings
timeout 60s cargo +1.99.1 fmt --all -- --check
timeout 180s python3 -m unittest discover -s bench -p 'test_*.py'
```

Put the documented Python 3.12 fixture environment on PATH: Rust tests invoke
Python subprocesses too. The benchmark suite has 161 tests: two require an
explicit locally verified `GEESEFS_WEED_BINARY` and otherwise skip. The native
enabled run passes all 161 without skips. The project Python suite has 16 tests.
Keep the native provider opt-in out of the Rust process environment: its embedded
Python gate has a separate finite deadline. Run the verified native-enabled Python
suite standalone. Use an owned disk-backed TMPDIR for real-engine tests rather
than relying on space in shared tmpfs. The public-source audit checks 388 resolved
packages, including only the exact approved Foyer fork/revision/package set.

The opt-in disk recovery tests document their isolated ext4 block/inode ENOSPC
and permission-error command. The mounted harness `bench/disk_cache.py` requires
frozen binaries/manifest, a delegated cgroup, nginx/rsync and an explicit deadline.
Measured workloads require `--fixture-weed` for the qualified SeaweedFS 4.51
fixture; Moto remains functional/fault-only. Declare fixed cycles/bytes/operations
identically across baseline/candidate/A-A mounts. Capacity preparation is outside
the measured interval; the later eviction witness probe has separate I/O counters.
Final source/hash errors invalidate success and exit nonzero while preserving rows.
Unavailable prerequisites fail rather than count as a successful skip. The
small-object protocol and 16 corrected large-read/eviction/writer representatives
pass, but the [earlier full 456-case qualification](performance-disk-cache.md)
failed. Later source-matched matrices are preserved in the
[follow-up report](performance-disk-cache-followup.md#combined-pipeline-checkpoint):
449/456 absolute L2/RAM byte/state passes and 96/96 Rust-relative passes. The
post-WOOF repairs have correctness verification, not a new paired qualification.
[Metadata allocation results](performance-metadata-compaction.md) are
not substituted for that missing performance acceptance.

Final direct-tree quality checks run `crapper --no-coverage` and
`dryer --threshold 0.8` on the four changed runtime files. Matching source
coverage is unavailable, so CRAP is N/A; complexity includes `load_disk_range`
25, `reclaim_batch` 16 and `admit_metadata` 12. Dryer reports three score-1.00
pairs in the existing namespace operation-marker bookkeeping, not zero
duplication. `mutator --scan` is discovery only; the ten executed fault
controls above are separate, source-addressed mutation evidence. Tool success
does not establish compatibility or performance acceptance.

## Bounded subtree listing — 2026-11-09 (unreleased)

`tests/subtree_listing.rs` checks a complete small traversal and marker ownership
with exactly one flat LIST and no HEADs. Before the change these require nine and
two LISTs respectively. Further fixtures check incomplete-page fallback, sibling
warming without sealing an ancestor, standard-metadata HEADs, expired attributes
and error propagation/retry. Deliberately sealing a truncated response as complete
fails the independent expected-name assertion; restore the range check afterward.

Existing directory-page tests retain lexical resume, eviction, local mutation and
in-flight rename/removal checks. Their request-count expectations distinguish the
new single flat attempt from delimiter continuation. HEAD deadline/concurrency
fixtures explicitly omit LIST metadata so they continue exercising real HEADs.

The opt-in `native_du_traverses_a_small_subtree_with_one_flat_page` mounts a
disposable in-memory namespace, checks exact allocated bytes, readlink and payload
bytes, and requires one flat LIST with no HEAD in that namespace. Unrelated native
mount observer probes are excluded by the synthetic namespace boundary.

```bash
cargo test --locked --test subtree_listing --test directory_pages --test directory_resolution --test lookup_preload
cargo test --locked --test fuse_directory native_du_traverses_a_small_subtree_with_one_flat_page -- --ignored --exact
cargo test --locked --all-targets --all-features --no-fail-fast
```

Request-count reduction in these synthetic fixtures is not a full-tree performance
or memory acceptance measurement. Directory block accounting is unchanged.

## Severe backend context — 2026-11-07 (unreleased)

`severe_replies_retain_backend_status_without_debug_history` drives real fuser
wire requests against loopback HTTP at the `error` level. Before implementation,
the terminal error lacks backend context; a second failing regression isolates
the missing GET-body observation after successful SDK headers. Cases cover
exhausted gzip/503 extension retries, non-retried 403, malformed successful XML,
exhausted SDK HEAD retries, truncated streaming and checksummed GET bodies, plus
successful retry recovery without false error records. An absent object followed
by a failed directory-marker HEAD retains the latest failure and counts both
observations in one slot. Assertions independently
check status/class/attempts and exclude disposable keys, credentials, URLs and
remote error text. The HEAD fixture disables optional parent LIST speculation;
it does not change production lookup behavior. These fixtures do not identify
historical EIO or establish mounted compatibility/performance acceptance.

Default and all-feature/all-target suites, warning-denying Clippy, formatting,
source audit and both Python suites pass. One additional full run reaches the
unmodified performance-evidence wrapper's 30-second Python deadline; a subsequent
`--no-fail-fast` run collects every target and passes. The separate Python bench
suite takes about 28 seconds locally, so that timing-sensitive failure remains
recorded rather than changing its deadline or assertions. Two selected SDK
finalizer mutations report killed. Duplication screening at 0.8 finds no
candidates; new helper complexity is at most 9, while coverage/CRAP remain unknown.
These scoped checks do not establish observer cost or durable log delivery.

## Severe diagnostic output under debug pressure — 2026-11-07 (unreleased)

A blocked-stderr fuser regression first loses a lone EIO reply record behind
successful debug traffic, while verifying independent FUSE replies still finish.
The reserved 16-record error/warning worker preserves that record under ordinary
lane saturation; an error burst separately verifies observable reservation drops.
The aggregate counter still includes losses from both bounded queues.

A separate fuser/socket child regression first fails for missing journal priority
framing. It verifies error/warn/info/debug priorities on matching device/inode
identity, and plain JSON for wrong device, wrong inode, malformed numeric input,
missing separator and absent `JOURNAL_STREAM`. Slow HEAD timing exercises info;
admission rejection and injected HEAD failure exercise warn/error. Both fixtures
check path/payload privacy. They test sender framing and ownership, not a running
journald receiver, its retention policy or lossless delivery.

Full default/all-feature/all-target Rust suites, warning-denying Clippy, format,
public-source audit and Python fixture/benchmark suites pass. Five selected
stream-identity/reservation mutations report killed; this is a scoped screen,
not repository-wide coverage or per-site compiler/assertion classification.
Coverage/CRAP remain unknown. The bounded worker adds one thread only in debug
mode; its performance and mounted compatibility acceptance remain separate.

## Extended LIST error-body retries — 2026-11-07 (unreleased)

Real loopback HTTP tests send invalid gzip error bodies with HTTP 429, 500 and
503, followed by an independently specified successful object listing. The
original adapter fails before retry with `TransportOrProtocol`; the corrected
adapter recovers the expected key and size. Additional cases verify zero/one
configured retries, exhausted errors retaining their HTTP code, no retries for
HTTP 403 or malformed HTTP 200, and no standard LIST fallback.

Default and all-feature/all-target Rust suites, Clippy with `-D warnings`, format,
public-source audit and Python fixture/benchmark suites pass. Three selected
retry-predicate mutations report killed; this is not repository-wide coverage.
Duplication screening of `src/backend/s3.rs` finds no candidates at 0.8. Complexity
screening reports CC 23 for `list_extended_inner`; coverage and CRAP remain unknown.
These checks prove the fixture correction, not the cause of historical errors or
mounted compatibility/performance acceptance. The published v0.1.5 is unchanged.

## Diagnostic levels and debug request traces — 2026-11-07 (unreleased)

Real fuser wire requests verify successful lookup/open/read lifecycle correlation,
executor entry, paired phase scope IDs, cache hit/miss, fill ownership and exact
payload bytes. CLI tests first reject the new levels; the wire regression then
fails for missing request records before implementation. Existing loopback SDK
fixtures verify error/cancellation phases and bounded reasons. Blocked stderr and
full-queue tests retain FUSE progress, dropped-record visibility and privacy.
Normal levels exclude fast successful traces; info includes slow/inflight work,
warn includes transient failures and error retains unexpected failures. Distinct
rate slots prevent warning/info candidates from suppressing an error slot.

Final review exposes a separate debug-mode sampling gap: severe replies still
consume their normal one-per-second slots even while debug is enabled. The real
wire burst regression first observes 299 rate-suppressed errors from 300 requests.
Select bypass by configured debug mode, not by each event's severity. After the
correction the same burst has zero rate suppression while still exposing bounded
queue drops and permitting independent successful FUSE work.
The admission fixture also waits for joined session shutdown before reading
counters: receiving socket bytes can precede the synchronous callback's subsequent
diagnostic accounting. The exact expected suppression count is unchanged.

Both Rust configurations, all-target/all-feature warning-denying Clippy, format,
public-source audit and Python fixture/benchmark suites pass. Python-dependent
Cargo fixtures require the fixture environment on `PATH`; the system Python alone
does not provide boto3. The first fixture run also exposed a test length mismatch
(24 requested bytes versus a 25-byte expected payload), corrected in the test.

Six selected mutator sites report killed: debug rate bypass, separate warning and
error slots, debug filtering and equal-level admission. This is not a full mutation
or coverage result, and per-site compiler-versus-assertion classification is not
provided by this screen. Complexity screening separates log rate admission from
record formatting; LLVM coverage/CRAP remain unavailable. Duplication screening
of the three changed source modules finds no candidates at 0.8. Existing larger
filesystem functions and closed errno/error-class mappings are not refactored by
this diagnostic change. Debug observer overhead and kernel-mounted acceptance
remain unmeasured; the published v0.1.4 executable is unchanged.

## READDIRPLUS directory attributes — 2026-11-06

Five non-mounted fuser `Session::from_fd` regressions use real protocol parsing
and replies with a faithful gated store. They verify overlapping marker HEADs,
authoritative permissions/owner, shared admission across two directory handles,
deadline cancellation, warm-cache progress under a busy metadata lane, basic
READDIR without extra HEADs, full-frame continuation and unsent lookup ownership
on errors. The serial implementation first fails the overlap assertion. Three
manual faults produce four assertion failures: premature permit release exceeds
the global bound; removing the fresh fast path yields ETIMEDOUT; premature lookup
ownership breaks full-frame and failed-frame counts. All faults are restored.
Eight selected mutator sites additionally report killed with no reported survivors;
this is a selected screen, not repository-wide coverage or a classified per-site
compiler-versus-assertion accounting. Complexity-only CRAP remains unavailable
without coverage; duplication screening of the two changed modules finds no
candidates at 0.8.

```bash
cargo +1.96.0 test --locked --test fuse_deadline_review readdirplus_
cargo +1.96.0 test --locked --test fuse_deadline_review readdirplus_directory_latency_diagnostic -- --exact --ignored --nocapture
cargo +1.96.0 fmt --all -- --check
cargo +1.96.0 clippy --locked --all-targets --all-features -- -D warnings
cargo +1.96.0 test --locked
cargo +1.96.0 test --locked --all-features
```

The explicit diagnostic checks names and unchanged work: 110 implicit directories,
110 HEADs, 112 LISTs, fixed 2-ms simulated HEAD delay, eight metadata slots and one
runtime worker. Two same-executable waves per revision each use eight fresh
filesystem/session instances. Warm-wave request medians are approximately
381 ms serial and 65 ms pipelined, with peak concurrent HEADs 1 and 8. These are
unoptimized fuser-protocol fixtures, not kernel mounts, real S3 latency, independent
mount pairs or a Go comparison. The historical production wait remains unattributed.

For the corresponding warmed eight-session harness jobs, `/usr/bin/time -v`
records user/system CPU 0.66/0.13 s and peak RSS 97,508 KiB before, versus
0.52/0.13 s and 97,520 KiB after. These include Cargo/harness work, not isolated
daemon CPU or steady RSS; heap, cgroup and kernel memory are unmeasured. They do
not establish a memory reduction or production CPU improvement.

Default and all-feature suites pass 316 cases with 18 explicit ignores, including
the new diagnostic. Formatting and warning-denying all-target/all-feature Clippy
pass, along with the public-source audit and Python fixture/benchmark suites.
The existing local native-mount limitations below remain; served readers
are not test fixtures.

## Bounded phase and failure diagnostics — 2026-11-06

Real SDK/loopback HTTP and fuser wire fixtures verify attempt/retry accounting,
presigning exclusion, observer reconfiguration, unchanged write-signing options,
terminal replies, admission/source attribution, owned READ tails, busy scrapes,
cancellation and rate-limited path-free records. An invalid extended LIST reaches
READDIR as EIO with `invalid_response` and a failed `s3_list` phase; cancelling a
gated HTTP LIST produces one ETIMEDOUT reply and a cancelled, not failed, phase.
The source-class and phase assertions first fail before their corrections.
Blocked-stderr and real-rate full-queue controls verify continued FUSE replies and
admission, bounded enqueue drops, complete JSON and independent off/error/warn
filtering. The synchronous writer and missing CLI option are observed failing
before the nonblocking correction; four additional selected logger mutations fail
behavioral assertions, including lossless backpressure and filtering at equality.

Both default and all-feature Rust suites pass **311 cases**, with **17 ignored**:
thirteen native cases, two namespace scaling diagnostics and two observer-cost
diagnostics. Formatting, warning-denying all-target/all-feature Clippy, the
349-package public-source audit, 14 Python fixture and 99 Python benchmark cases
pass with Rust 1.96.0 and the fixture virtual environment on `PATH`.

Selected mutation receipts distinguish 15 behavioral failures (six SDK and nine
diagnostics) from two compiler-rejected SDK mutants. The SDK's unreachable default
for already-initialized execution state was removed; no selected mutant survives.
This is a scoped failure-sensitivity check, not repository-wide mutation coverage.
Complexity-only screening of the two new modules has CC 12 for the closed errno
mapping and 10 for the error-class mapping; without LLVM coverage, CRAP is unknown.
Duplication screening at 0.8 finds no candidates in those two modules.

The [observer fixtures](metrics.md#observer-cost-fixtures) compare the same release
executable with detailed diagnostics on/off. Their client/runtime/backend work is
in the measured process; they do not replace kernel-mounted or Go-relative
acceptance. Native isolation remains blocked locally by noninteractive sudo and
the denied user-namespace `uid_map`; served mounts are not substitutes.

## Namespace progress — 2026-11-06

`tests/namespace_progress.rs` adds fourteen public-API regressions for RO/RW
resolution during namespace maintenance, unrelated-state-independent flush,
rename and nested listing, every writer mutation, error/cancellation/newer-write
generations, dirty rename/unlink/implicit parents, lexical pagination, cooperative
wide scans and concurrent resolver/eviction ownership. Initial progress cases
fail before correction; removing the recreated-name identity guard independently
loses the new backend object. The [lock audit](locking.md) covers all remaining
application mutex/RwLock domains and explicit residual risks.

Existing cache-index, read/refill/window/view, loan, FUSE-wire/deadline/lifecycle,
SDK and writer suites cover the follow-up cache/lock corrections. Faults that
export a loaned cached reply or omit a loan-key insertion respectively retain
128 KiB of revocable memory and leave loans unreclaimed; the existing assertions
fail, then pass with the required ownership/index updates restored.

```bash
cargo +1.96.0 fmt --all -- --check
cargo +1.96.0 clippy --locked --all-targets --all-features -- -D warnings
cargo +1.96.0 test --locked --all-targets
cargo +1.96.0 test --locked --all-targets --all-features
```

Both final Rust configurations pass 300 cases, with 15 explicitly ignored cases
as scoped below. Formatting, warning-denying Clippy, the 344-package public-source
audit, 14 Python fixture tests and 99 Python benchmark tests pass. The default
jemalloc release executable also builds with Rust 1.96.0.

The Python fixture/benchmark environment must be on `PATH` for the embedded
performance gate. Also run the CI public-source audit and Python test/bench
suites. The package's `unsafe_code = "forbid"` applies to all compiled targets;
there are no application unsafe blocks or local FFI shims. The initial local
toolchain was too old and nightly lacked Clippy; Rust 1.96.0 plus its supported
formatter/Clippy were installed for minimum-version verification. Its linter
also exposed an existing equivalent XATTR boolean expression, now simplified.

### In-process scaling diagnostics

The ignored `tests/namespace_scaling.rs` fixtures run without FUSE, private
credentials or a live backend. Use the identical fixture on baseline `b76269b`
and runtime correction `0d8c587`, build each once, retain executable hashes, and alternate
AB/BA order in fresh processes. They are **unoptimized test-profile/System
allocator** diagnostics on Linux 6.17, not release, jemalloc or mounted results.
The subsequent writer-input copy handoff does not alter these timed flush/read
paths; writer input admission/copy happens before each flush timer starts.
At 100,000 clean cached children, eight process pairs give these median per-run
p50 values; eight resolver threads perform 200,000 total lookups:

| Operation | Baseline | Candidate |
| --- | --- | --- |
| Idle flush | 22.207 ms | 0.420 µs |
| One dirty-generation flush | 22.660 ms | 0.116 ms |
| Expired implicit directory lookup | 70.649 ms | 0.030 ms |
| Root listing with nested cached descendants | 97.349 ms | 0.157 ms |
| Concurrent inode resolver batch | 883.810 ms | 37.622 ms |
| Process user/system CPU during measured work | 8.275 / 0.600 s | 0.250 / 0.000 s |
| Post-setup RSS | 111982 KiB | 117632 KiB |
| Post-workload RSS / VmHWM | 112404 / 112404 KiB | 118086 / 118086 KiB |

Four additional 10,000-entry pairs diagnose scaling; four same-executable A/A
pairs at 100,000 entries retain noise controls. The extra weak index costs about
5.5 MiB at 100,000 entries; this is not a metadata RSS reduction or an extrapolated
large-mount bound. Backend bytes and identical operation counts are checked.

The warm-cache fixture has eight readers, 256 byte-checked 1 MiB reads each, with
no new backend reads. Eight AB pairs and eight same-executable AA pairs per
fragment size distinguish copying from a single-backing control:

| Cached backing size | Wall, baseline → candidate | Median reader p99 | User/system CPU | RSS / VmHWM, baseline → candidate |
| --- | --- | --- | --- | --- |
| 64 KiB fragments | 751.024 → 208.348 ms | 4.913 → 1.307 ms | 1.160/0.050 → 1.360/0.060 s | 52892/52892 → 53088/53088 KiB |
| One 1 MiB backing | 73.354 → 74.841 ms | 0.101 → 0.096 ms | 0.445/0.010 → 0.460/0.005 s | 36544/36544 → 36784/36784 KiB |

Fragment throughput/latency improve while consuming more process CPU. The
single-backing control has a small raw wall increase, not demonstrated superiority.
These are fresh-process pairs, **not independent mount pairs**; cgroup/heap/kernel
memory, client/daemon separation, sustained target workloads and Go-relative
acceptance remain unmeasured. Raw results, original failures and binary/source
hashes stay outside tracked source.

```bash
GEESEFS_SCALE_ENTRIES=100000 cargo +1.96.0 test --locked --test namespace_scaling namespace_scale_diagnostic -- --ignored --exact --nocapture
GEESEFS_CACHE_FRAGMENT_KIB=64 cargo +1.96.0 test --locked --test namespace_scaling cache_scale_diagnostic -- --ignored --exact --nocapture
GEESEFS_CACHE_FRAGMENT_KIB=1024 cargo +1.96.0 test --locked --test namespace_scaling cache_scale_diagnostic -- --ignored --exact --nocapture
```

Native isolated mount verification is blocked locally: noninteractive sudo
requires a password, and user/mount/network `unshare` fails writing `uid_map` with
`Operation not permitted`. The thirteen existing native cases remain explicitly
ignored by default; these diagnostics add two separately ignored large fixtures.
Do not count that as native acceptance or reuse an existing served mount.

## Independent review regressions — 2026-11-03

Four targeted suites cover the corrected review failures:

```bash
cargo test --locked --test write_review_regressions --test namespace_review_regressions --test startup_review_regressions --test fuse_deadline_review
```

The 24 cases include applied PUT/completion errors through the real SDK and
loopback HTTP, cancellation/newer writes/truncate boundaries, stale metadata
responses at the same content ETag, both pending-rename parents, implicit marker
discovery, absence of startup HTTP for invalid budgets, and actual fuser wire
replies with deadline/early-publication ownership. Baseline failures were observed
before correction; an early-publication control was additionally fault-injected.
These fixtures do not require a kernel mount or production credentials.

Both allocator suites and explicit native-mount tests remain separate checks.
Scoped mutation/complexity results do not imply repository-wide coverage or a
complete CRAP threshold pass.

## Prometheus observations — 2026-11-03

The configuration regression initially rejects `--metrics-listen`; after adding
the listener, tests verify numeric/IPv6 parsing and the disabled default. Seven
metrics regressions cover cold/warm/EOF accounting, backend failure, unchanged
admission and backend-call counts during scrapes, retained versus pinned bytes,
entry/byte pressure, namespace eviction/failure, real HTTP routes and contention,
bind errors, partial-header deadline and shutdown. Both allocator suites pass
262 Rust cases with thirteen native cases ignored by default. The benchmark
Python gate requires the project virtual environment; an initial system-Python
run failed on missing `psutil`, not a runtime regression.

Formatting, all-target/all-feature warning-denying Clippy, default release build
and the 342-package public-source audit pass. Fresh default/System LLVM coverage
and native Python coverage keep new metrics modules/fixture below CRAP 10 (worst
8.4); the broader filesystem CRAP gate remains open, including existing high-CC
read/lookup/rename methods. Duplication screening at 0.8 finds no candidates in
the changed runtime/fixture scope. Eleven selected mutations in HTTP/allocator
observations and cache/namespace counters are killed; this is not repository-wide
mutation coverage. Initial report-path/unsupported-report-option failures were
tool setup errors, not a quality pass.

The explicit real-binary fixture uses disposable local S3 and private mount/network
namespaces. It verifies independent bytes, a cold miss then a direct-I/O warm hit,
OpenMetrics output, existing stats-file JSON and normal unmount/listener exit:

```bash
timeout --kill-after=5s 130s unshare --mount --net --propagation private \
  python3 scripts/metrics_smoke.py --binary "$GEESEFS_RUST_BINARY" \
  --output "$GEESEFS_METRICS_RESULTS"
```

Run it in an owned resource-limited unit as described below. Results are local
fixture checks, not a new serving deployment, comparative performance result or
cache-size recommendation. See the [metric semantics and decision criteria](metrics.md).

## Cross-UID mount access — 2026-11-02

The public `scripts/mount_access.py` fixture requires explicit root, private mount
and network namespaces, a disposable local backend and a finite deadline. For
example, inside an owned resource-limited test unit:

```bash
timeout --kill-after=5s 130s unshare --mount --net --propagation private \
  python3 scripts/mount_access.py --binary "$GEESEFS_RUST_BINARY" \
  --output "$GEESEFS_ACCESS_RESULTS"
```

The output directory must not already exist. A child with UID/GID 65534 and no
supplementary groups reads via an inherited mount-root descriptor, so private
fixture ancestors cannot explain `EACCES`. All three real-FUSE cases pass:

- Default owner-only session: readable mode-0644 file is denied to the other UID.
- Explicit `ro,allow_other`: the other UID reads the independently specified bytes.
- Explicit `allow_other` plus mode 0600: the other UID is still denied by kernel
  permissions. The mount also retains `nodev,nosuid,noexec,default_permissions`.

The normalization test first rejects `allow_other` on the original parser. After
accepting it, an entrypoint fault that omits `SessionACL::All` makes the native
readable-file assertion fail with `EACCES`; restoring the wiring passes. A prior
interpreter-path setup failure is retained separately, not counted as fault
sensitivity. All positive mounts unmount normally and daemons/scopes exit.

For this source, both allocator suites pass 253 cases with 13 native tests ignored
by default; the explicit access fixture supplies the three cases above. Formatting,
all-target/all-feature Clippy, release build and all 99 benchmark Python tests pass.
Fresh LLVM/Python profiles give affected CRAP scores at most 8.3; affected
duplication at 0.8 is absent. The broader-file CRAP gate still fails. The automated
mutation scan found zero eligible sites and supplies no mutation score; the native
omitted-ACL fault is the failure-sensitivity evidence. These local checks do not
establish a new performance result or production deployment.

## Listing configuration and reader-access diagnosis — 2026-11-02

The [latest diagnostic checkpoint](performance-listing-diagnosis.md) uses the
unchanged frozen release in 40 independently mounted runs: default Rust,
explicit negative64, its identical-config A/A, Go 0.43.9 and its A/A. All byte,
listing, rsync and final generation checks pass; resource events, task/cgroup
exit and mount cleanup are verified. Separate traced mounts identify repeated
miss requests, then their absence with explicit caching. Traced timing is not
used for performance screening. New Go RSS controls are biased, not a memory win.

A real-FUSE external-create probe demonstrates the option's natural TTL delay.
Omitting the cache option makes its visibility assertion fail; restoring the
correct configuration passes again. That frozen executable's option probes reject `allow_other`,
`dev`, `suid` and Go's `--pprof`; a separate mount returns correct bytes to its
owner but `EACCES` to UID 65534 via an inherited mount-root descriptor. These are
observed integration gaps, not passing full serving compatibility. Read-only
production memory observations preserve reader/mount identity but cannot assign
live-heap owners without a working existing profiling listener.

The change is documentation only. Full Rust/allocator and Python suites retain
their earlier recorded scope; they are not rerun or relabelled as new coverage.
Private probes do not close the public generic matrix or broad quality gaps.

## Statistical gate and adapter quality — 2026-11-02

`bench/paired.py` and `bench/compare.py` now require eight mount pairs, a real
same-executable control arm and predeclared practical margins; 17 paired-assessment
cases plus three-arm scheduling, margin, zero/overflow and writer-file cases pass
with the whole 99-case Python suite. Mutation of `bench/paired.py` kills 40 of 42
sites. A public `assess` boundary case distinguishes an interval touching the loss
limit from a demonstrated regression, killing the previously surviving `> -> >=`
mutation. The two bootstrap percentile-rank mutations remain undetected; they are
an open test gap, not proven equivalent or impossible to distinguish.

Both allocator LLVM runs pass **265 Rust cases**, including thirteen native
directory contracts (new receive-thread liveness and xattr error contracts) and
the public `errno` mapping. CLI child profiles are now merged into both reports.
`crapper --threshold 10 src/fuse.rs` **passes** (worst `write` 9.5): `errno` and
`fallocate` delegate to `backend_errno`/`allocate`, `getxattr` is covered natively.
`compare.py` orchestration functions that need real mounts (`_measure_run`,
`_run_case`, `main` and helpers) still exceed CRAP 10 at 0-50% unit coverage;
the updated three-arm native matrix is still unverified end to end. Earlier
integration runs do not establish coverage of the changed orchestration. Clippy,
formatting and `dryer --threshold 0.8` pass in the recorded review scope.

## Actual LXC read-only preflight — 2026-11-02

The exact current jemalloc release passes available rw/ro nginx, rsync and failure
contracts on the development VM. It also passes an isolated read-only fixture on
the authorized LXC kernel `5.15.179-15.2mds`, with the installed nginx 1.24.0 and
rsync 3.2.7. Buffered and direct-I/O mounts pass byte checks, four-client unaligned
Range/EOF, implicit listings, `EROFS`, nginx full/Range/416 and rsync full/delta.
Buffered shared mmap passes; direct-I/O shared mmap returns `ENODEV` on this kernel.
A changed backend byte causes the expected SHA256 assertion failure; restoring
the fixture passes. Both success and failure clean up normally.

The fixture uses an owned private mount namespace, temporary paths, a 1 GiB unit
with a two-CPU quota, and a disposable backend reached through a temporary SSH forward.
It does not test the served configuration, golist, real S3 or sustained load.
See [target evidence and remaining measurements](performance-target-preflight.md).

## Loaded read controls and mount cleanup — 2026-11-02

The [read-shape checkpoint](performance-read-shape.md) completes a private
40-mount nginx/golist/rsync design with actual Go A/A controls, independent bytes
and final backend generation checks. A separate real-S3 LXC attempt stops on a
dependency timeout with five complete blocks; it does not meet the eight-pair
gate. The guard's twelve local-wire cases and ten relevant faults verify request,
byte, concurrency, rate, path, credential and redirect boundaries. Full/partial
body corruption and removed-oracle controls verify the native client byte checks.
These private runners do not verify the public generic matrix end to end.

A host audit finds three stale disposable mounts. Two retain kernel-blocked
threads under zombie leaders from earlier failed rename tests; `df` waits in
FUSE getattr. The idle mount unmounts normally, but the other two return busy.
After ownership is proved, only those two disposable connections are aborted
and ordinarily unmounted; the tasks exit and bounded `df` passes again.
Unrelated FUSE mounts are unchanged. This corrects the earlier cleanup conclusion:
inactive/failed units and `MainPID=0` do not establish absence of live threads.

Run native tests in a private mount namespace or a dedicated unit with
`PrivateMounts=yes`, with finite run/stop deadlines. Verify actual remaining
processes/cgroups and mount identities after failures, including zombie leaders
with live threads. Never infer ownership from a connection number or abort all
FUSE connections; preserve unrelated/served mounts. A private namespace prevents
host mount propagation, but does not replace verifying that blocked tasks exit.

## Retained rename correction — 2026-11-02

This block records the earlier rename-only source and verification scope.

The reduced source retains only removal of redundant post-rename entry
notifications; receive-loop CLI and first-poll read trials are discarded.
Both jemalloc and all-feature System-allocator LLVM runs pass **261 Rust cases**,
including eleven native directory contracts, and 75 benchmark Python cases.
The native suite also passes separately with two and four receive loops through
the test-only `GEESEFS_TEST_FUSE_THREADS` environment setting. The new four-client
rename/list/read/write regression hangs on the old notification path and passes
after correction, including full dirty-close bytes after unmount.

Matching default/System instrumented executables and the exact frozen default
release pass available rw/ro, literal nginx, rsync, Range/EOF/mmap and injected
fsync failure/recovery contracts. These are disposable Linux fixtures, not the
actual LXC/nginx/rsync/golist deployment or positive LINK/service acceptance.
Formatting, warning-denying all-target/all-feature Clippy and public-source audit
pass. All 85 frozen inputs match the committed source; only `src/fuse.rs` and
`tests/fuse_directory.rs` differ from the observation-age baseline.

Whole affected-file `crapper --use-existing-coverage --threshold 10 src/fuse.rs`
still **fails**: `errno` 23.8, `getxattr` 14.2 and `fallocate` 11.0. Adapter `rename`
is CC6/79.2% coverage/CRAP6.3 in both matching native reports. `dryer --threshold
0.8` finds no duplicates. Broad mutation remains open; historical restored-fault
hangs are failure-sensitivity evidence, not a completed new mutation campaign.
Two default-allocator CLI child profiles are preserved separately but were not
merged after switching allocators; do not claim complete child-entrypoint coverage.
Original setup/report-option failures and corrected completion receipts remain
private. Successful test/build commands do not turn these quality gaps green.

## Library and contract checks

Use Rust 1.96.0+ with rustfmt/clippy, Python 3.12+ and the public dependencies from
`requirements-fixture.txt`. The idle flush-cache policy passes 211 Rust cases,
formatting, all-target/all-feature warning-denying Clippy and the public-source
audit in Kubernetes. The unchanged runner also passes 57 Python cases.
Default and System-allocator ThinLTO builds pass; the default release passes
30 backend-contract, cache-loan and read-view cases.
These checks are not full reference parity or production acceptance.

The 2026-10-31 admission/directory correction passes 218 Rust cases with all
targets/features, formatting, warning-denying all-target/all-feature Clippy, the
public-source audit and a default release build on the VM using Rust 1.99.1.
The gated admission and unrelated-directory regressions each fail before their
fix, then pass. A further directory-removal race first exposes the missing final
binding check. Directory tests also retain TTL, inode eviction, parent/descendant
rename and in-flight response coverage. The Python fixture dependencies must be
on the selected interpreter's environment; setup failures are not behavioral RED.

Installed `mutator` kills all three selected epoch/binding-guard faults using the
directory public-interface suite. This is a three-site check, not whole-file or
repository mutation coverage. `dryer --threshold 0.8` finds no duplicate candidates
in the three changed runtime files. `crapper --no-coverage` reports complexity
only: coverage and CRAP remain N/A, and large existing methods remain open work.

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
python3 scripts/audit_public_sources.py
```

Public-interface tests cover SDK loopback HTTP/TLS, conditional ranges, checksum
and EOF barriers, metadata/configuration, generation races, sparse/truncate writes,
multipart/PATCH recovery, bounded admission and cancellation. Gated faithful
storage fakes and channels control concurrency without sleep-based interleavings.
Expected content, ranges and observable side effects are independent of production
algorithms. Each bug fix needs an observed behavior failure before the correction;
selected faults/mutations verify sensitivity and must be restored before rerunning.
Broader mutation coverage and large-method quality findings remain open.

The integrated freshness/handle correction also runs nine explicitly requested
native tests in `tests/fuse_directory.rs`; they fail if mounting is unavailable.
They cover PLUS paging/refcounts/error rollback, real kernel TTL, handle pressure,
marker attrs, side-effect-free unsupported hardlinks, writer metadata/xattrs,
fallocate/truncate/rename/removal and durable close. Run separately on an authorized
Linux fixture:

```bash
timeout --kill-after=10s 180s unshare --mount --propagation private \
  cargo test --locked --test fuse_directory -- --ignored --test-threads=1
python3 -m unittest discover -s bench -p 'test_*.py'
```

Both default-jemalloc and all-feature System-allocator LLVM runs pass 251 Rust
cases, including those nine native mounts. Separate instrumented executables also
pass read/write and read-only rsync/aiohttp/nginx/failure contracts, validate the
reported allocator and effective budgets, and exit normally. Jemalloc page gauges
are checked through the real statistics file; this is not performance timing.
The exact frozen default release separately passes the same available rw/ro
contracts, including literal nginx, rsync, injected fsync recovery and EROFS;
213+3 real allocator/page/budget observations and both daemon exits are checked.
Child-process profiles are preserved and merged only with matching source/build.
Python branch coverage covers 75 bench tests; the separate fixture suite passes
14 tests. CLI receipts, malformed schema/budgets, actual worker protocol and
checksum/failure-sensitive workload tests supplement pure gate-policy tests.
Automated mutation kills 24/24 Go-heap and 30/30 shared-metrics sites, including
exactly two samples and zero values. This is not repository-wide mutation coverage.
Affected runtime/accounting duplication >= 0.8 is absent. With matched native
entrypoint coverage, mount options and serving have CRAP 9 and 8; the extracted
directory pipeline, standard LIST decoding and benchmark gate have CRAP 6, 8
and 7 respectively. Whole affected-file CRAP still exceeds 10: existing rename
is 62.9, and uncovered matrix orchestration reaches 56. These remain failures,
not excluded files or a relaxed threshold. A successful release build is not
coverage or full compatibility acceptance.

SDK loopback tests retain malformed/confinement/order checks, V1 prefix-only
continuation, exact 1,000-entry bounds and opaque V2/extended tokens. Controlled
faults in each boundary trigger the intended assertions and are restored before
the passing suite. Native 8,192-name traversal requires 65 bounded LIST calls,
including EOF, without larger directory buffers or inaccurate file types.

Key read regressions live in `tests/read_windows.rs`, `read_reply.rs`,
`read_refill.rs`, `read_stream_history.rs`, `cache_loans.rs`, `receive_ownership.rs` and
`backend_contract.rs`. Keep FIFO progress, full backing charges, If-Match,
generation, checksum, cancellation and caller-owned task lifetimes intact.
The loan tests verify cache reuse without a higher total, owned reply isolation,
writer/multipart progress, concurrent uploads, cancellation and failed-fsync retry.
Five deliberate semantic faults trigger the intended assertions, including the
history-reset regression after explicit independent cache pressure. These checks
are not a whole-repository mutation or coverage/CRAP result.

## Disposable mounted verification

The fixture needs Linux `/dev/fuse`, `fusermount3`, rsync, Go, Python dependencies
from `requirements-test.txt`, a built Rust executable and the pinned public Go
checkout:

```bash
cargo build --locked
timeout --kill-after=30s 1200s python3 scripts/integration.py \
  --go-reference "$GEESEFS_GO_REFERENCE"
```

By default the runner owns Moto, a unique bucket, credentials and mounts. An
external backend requires explicit disposable-test opt-in and dedicated test
credentials. Ambient AWS credentials are not reused. Logs and partial failures
belong in ignored `benchmark-results/private/`; cleanup is confined to ownership.

Actual mounts exercise full/unaligned Range/EOF, mmap, implicit directories,
aiohttp full/Range, rsync, symlinks, mode/mtime, xattrs, FIFO, fsyncdir,
sparse/truncate, fresh-reader durability, rename and concurrent writer/fsync.
Rust injected upload/fsync failure propagation and recovery passed; the reference
Go open-file recovery remained EAGAIN through its deadline. Hardlink-as-symlink
formerly returned EIO after mutation. Corrected Rust returns EOPNOTSUPP without
mutation, so the required hardlink compatibility probe remains failed.

Actual-service IAM/SSE/signing/PATCH, imported multipart layouts, full CLI parity
and broader durability cases remain open. A separate native fixture checks literal
nginx 1.28.3 HEAD/full checksum, unaligned/suffix/short-EOF 206 and unsatisfiable 416
responses through all three mounted filesystems. No all-xfstests pass is claimed.
CI covers deterministic checks, not mounted or
performance acceptance. See [compatibility](compatibility.md) for the inventory.

The cache-policy release separately completes 120 healthy-warm four-binary records
with 30 frozen generations verified, then 60 persistent mixed cycles per binary
(1,440 checked passes). Independent raw-series analysis verifies fixed work,
unchanged budgets and quota/memory counters. Rare successful 30-second Rust reads,
other adverse metrics and unisolated observer cost keep performance unaccepted.
These healthy runs do not close the known mounted compatibility/failure gaps.

The final admission/directory binary separately completes 1,440 fixed-work and
720 equal-offered-rate passes on the VM, with all eight final full-byte/generation
checks passing. Owned mounts, clients, daemons and backends stop successfully.
The paced control has no missed cycles but retains a late-run metadata-tail
regression and cgroup-memory losses. Both raw-series summaries verify equivalent
work and unchanged budgets; all 25 compiled inputs and four executable hashes are
checked against retained provenance. These comparisons neither repeat the same
design independently nor pass compatibility/performance acceptance.

The later integrated checkpoint independently verifies 240 isolated healthy
cold/warm records, 60 generation/full-byte groups and 2,880 mixed phases with
16 further checks. The original isolated controllers retain backend-startup and
final-bookkeeping errors; independent complete-grid validation does not relabel
them. The mixed controls complete, preserve recorded interval counters and stop
their owned processes/mounts. Short isolated allocator/late windows remain missing,
and throughput/memory nonwins remain acceptance failures; see the
[latest measurements](performance-integrated.md).
