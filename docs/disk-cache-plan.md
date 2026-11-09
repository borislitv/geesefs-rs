# Selective disk payload cache: implementation plan

## Task

Add a configuration-selected, bounded disk payload L2 below the existing RAM L1
for regular files, initially `.deb` and `.rpm`. Implement the selected CLI-only,
no-payload-TTL, best-effort on-eviction, persistent block-cache contract in
[the design](disk-cache-design.md), preserving filesystem/S3 correctness and
writer durability. Approximate LRU is accepted; qualify the proposed CLOCK-style
mechanics and engine. Implementation is authorized with local verification only.

Qualify Foyer 0.22.6 through supported APIs before selecting the disk dependency.
Stop at the dependency gate if ownership, resource bounds or the accepted eviction
contract cannot be met; update the design and dependency decision before proceeding.
Do not silently substitute a custom engine or weaker contract.

Current gate: Foyer 0.22.6 passes isolated persistence, fixed-container recycling
and live second-chance retention, but fails persistence after reinsertion. See
the [dependency decision](dependency-decisions.md#selective-disk-payload-cache-qualification--2026-11-09).
Stage 1 is incomplete; stages 2–8 are blocked. The qualification test deliberately
asserts the required contract and remains failing on the released dependency.

## Plan structure

Contract and dependency qualification; configuration and policy; isolated disk
storage; read-path integration; failure/coherence verification; mounted workloads,
performance and delivery. Each implementation stage starts with a meaningful RED
behavioral test and finishes GREEN; the following verification stage expands the
matrix. Keep raw receipts and the action/reason log outside tracked source.

Commands below are for the future implementation, not checks already run. Use
Rust 1.99.1 explicitly, with its binary directory and the documented Python
fixture environment on `PATH`; do not change the global default toolchain.

### Stage 1: Freeze the contract and qualify the storage dependency

**What to implement:**

- Retain the operator-selected three CLI flags, no payload expiry, on-eviction population,
  approximate LRU, persistent reuse and overload fallback. Specify bounded
  CLOCK-style sweeps and initial 128-KiB logical records shared with L1/S3 reads.
- Create an isolated public-data qualification fixture for Foyer 0.22.6. Exercise
  `Store::load`, bounded insertion ownership, restart, disk corruption and overload
  without replacing `ReadCache`. Preserve a hard reserve for request/writer progress.
- Account for backing owners, keeper entries, serialization, read buffers, index
  memory and engine pools. Demonstrate a stalled writer cannot consume the reserve.
- Verify full-key collision rejection, in-flight delete behavior and tombstone/
  recovery semantics. Establish an actual access signal through supported APIs;
  test hot/cold region reclamation, bounded reinsertion and nonblocking admission
  when a sweep cannot free space. Do not treat an all-`None` Foyer picker chain as
  a pause: its manager falls back to random eviction.
- Measure recovery time, index memory and transient allocations at a terabyte-
  representative record count. Verify index/header-only recovery and on-demand
  identity/payload validation without a full cache download or startup HEAD storm.
- Record one dependency selection and its public version/license/API evidence.
  Complete this gate before implementation stages 2 onward.

**Files:** `docs/disk-cache-design.md`, `docs/dependency-decisions.md`,
`tests/disk_cache_engine.rs`, `Cargo.toml`, `Cargo.lock`.

**Documentation/examples:** versioned Foyer sources linked in the design;
`tests/budget_limits.rs`, `tests/cache_loans.rs`, `tests/support/mod.rs`.

**Verification:**

```bash
timeout --kill-after=10s 1800s cargo +1.99.1 test --locked --test disk_cache_engine
timeout --kill-after=10s 120s python3 scripts/audit_public_sources.py
```

### Stage 2: Add CLI configuration and cache policy

**What to implement:**

- Add `--disk-cache-dir`, `--disk-cache-max-size`, `--disk-cache-regex` to `Config`
  and compile `CachePolicy::matches` with regex. Require the complete option set;
  retain existing defaults and clap/fstab normalization. No YAML/TOML loader.
- Compile the mount-relative expression once. Reject malformed/oversized patterns
  and invalid sizes. Derive bounded queue limits inside the existing memory limit.
- Validate configuration and local-directory constraints before SDK startup;
  explicitly disabled/default operation performs no disk-cache activity.

**Files:** `src/config.rs`, `src/cache_policy.rs`, `src/lib.rs`, `src/main.rs`,
`tests/disk_cache_config.rs`, `Cargo.toml`, `Cargo.lock`.

**Documentation/examples:** regex and storage sources linked in the design;
`src/config.rs::validate`, `src/main.rs::configuration`, `tests/config.rs`.

**Verification:**

```bash
timeout --kill-after=10s 900s cargo +1.99.1 test --locked --test disk_cache_config --test config
timeout --kill-after=10s 60s cargo +1.99.1 fmt --all -- --check
```

### Stage 3: Verify configuration and policy boundaries

**What to implement:**

- Extend actual-binary tests for incomplete flag combinations, unknown options,
  invalid regex, literal escaping, mixed-case suffixes, prefix mounts, malformed
  sizes and invalid directory relationships. Preserve RAM-only defaults.
- Verify `.deb`/`.rpm` inclusion and metadata/other suffix exclusion without
  changing file visibility. Prove invalid config causes no backend/mount effects.
- Prove sensitivity by inverting the match or omitting the configured policy,
  observing wrong membership, restoring it and rerunning the affected tests.

**Files:** `tests/disk_cache_config.rs`, `tests/config.rs`,
`tests/startup_review_regressions.rs`.

**Examples:** current CLI subprocess helper and startup loopback fixture.

**Verification:**

```bash
timeout --kill-after=10s 900s cargo +1.99.1 test --locked --test disk_cache_config --test config --test startup_review_regressions
```

### Stage 4: Implement the isolated L2 adapter

**What to implement:**

- Add `DiskKey`, a versioned immutable record envelope, and
  `DiskCache::{open, read_block, try_demote, close}` around the qualified engine.
- Implement stable backend/object/range identity and bounded checksum/length
  validation. Share the current logical read-block definition; preserve shorter
  tails and partial coverage without extending GETs merely to populate L2.
- Own one locked local directory, lifecycle, bounded job/byte admission and
  managed disk-space accounting, including pending and retired storage.
- Implement known-invalid-first reclamation and bounded CLOCK/second-chance
  eviction through the qualified engine. Add high/low watermarks and atomic space
  reservation before write admission. Keep pinned/retired bytes charged.
- Pack logical records in engine-managed regions. Bound hot-record reinsertion;
  a saturated reclaimer skips new insertions rather than blocking required reads.
- Recovery restores compatible completed records from persistent inventory;
  corrupt/incomplete records become misses. Restore approximate recency in batches.
  Keep unaccounted recovered space unavailable for new writes until inventoried.
- Use supported safe filesystem APIs; own and drain workers with explicit
  deadlines. Distinguish caller cancellation from still-running blocking I/O.

**Files:** `src/disk_cache/mod.rs`, `src/disk_cache/key.rs`,
`src/disk_cache/store.rs`, `src/budget.rs`, `src/lib.rs`, `tests/disk_cache.rs`.

**Documentation/examples:** qualified engine APIs; `src/budget.rs`,
`tests/cache_index.rs`, existing tempfile/owned-channel fixtures.

**Verification:**

```bash
timeout --kill-after=10s 900s cargo +1.99.1 test --locked --test disk_cache --test disk_cache_engine --test budget_limits
```

### Stage 5: Verify real storage, capacity eviction and restart behavior

**What to implement:**

- Verify an old unchanged record remains usable without pressure, irrespective
  of age. Exercise cold eviction, second chances for hot records, all-hot/pinned
  sets, byte-weighted reclamation, watermarks and occupied-plus-reserved limits.
- Exercise graceful reopen and child-process interruption during publication;
  corrupt/truncate a stored record and verify exact source fallback bytes.
- Test two-process directory exclusion, budget shrink/recovery, exhausted inode
  or space admission, permission/I/O failures, pinned reads and queued writes.
- Verify actual managed allocation and index/worker bounds under concurrent
  pressure. Use a disposable quota-limited filesystem for native ENOSPC checks.
- Test large-inventory restart without reading all payload or contacting S3 for
  each record. Preserve completed data after graceful and interrupted shutdown;
  incomplete inventory cannot authorize destructive cache reset or over-admission.
- Mutate recency/reservation/identity/publication guards separately, observe behavioral
  failures and restore each guard before the final pass.

**Files:** `tests/disk_cache.rs`, `tests/disk_cache_recovery.rs`,
`tests/support/disk.rs`.

**Examples:** gated ownership tests in `tests/cache_loans.rs`; real subprocess
fixtures in `tests/fuse_deadline_review.rs`.

**Verification:**

```bash
timeout --kill-after=10s 900s cargo +1.99.1 test --locked --test disk_cache --test disk_cache_recovery
```

### Stage 6: Integrate L1 demotion, L2 reads and lifecycle

**What to implement:**

- Add an explicit capacity-eviction result to `ReadCache`; hand eligible victims
  to `DiskCache::try_demote` after releasing the cache lock. Skip loans,
  invalidation/replacement victims and unadmitted work.
- Extend payload budget partitioning with bounded spill/engine capacity inside
  the unchanged total. Preserve FIFO required admission and independent fsync
  progress; do not retain a flush loan in an asynchronous disk job.
- In `read_inner`/`load_window`, try eligible L2 blocks under the existing fill
  owner, admit output/scratch before allocation, recheck snapshot identity after
  waits and fetch only missing ranges. Consult L2 from `refill_after_hit` too.
- Reuse logical L1/read-path units for disk records and split larger validated
  backings with explicit charges. Keep checksum barriers, first-reply ownership,
  cancellation and exact EOF semantics. Touch existing L2 recency on L1/L2 use
  without issuing disk metadata writes or populating L2 on an ordinary L1 hit.
- Bypass dirty/sparse/pending-namespace states; preserve write/truncate/rename/
  unlink generation barriers. Join cache lifecycle through construction and
  `Filesystem::shutdown`, including paths where writer flushing fails.
- Export L1/L2 hit bytes, misses, demotions/drops, capacity evictions, corruption, disk
  read/write latency, pending bytes/jobs, allocated/index memory and recovery
  state through the existing metrics registry. Do not label by object path.

**Files:** `src/cache.rs`, `src/budget.rs`, `src/read.rs`, `src/filesystem.rs`,
`src/main.rs`, `src/metrics.rs`, `src/metrics/`, `tests/disk_cache_reads.rs`,
`tests/metrics.rs`. Inspect `src/inode.rs` and `src/namespace.rs` for each affected
publication invariant; add only necessary identity-state changes.

**Documentation/examples:** `docs/architecture.md`, `docs/locking.md`,
`tests/read_windows.rs`, `tests/read_refill.rs`, `tests/read_reply.rs`.

**Verification:**

```bash
timeout --kill-after=10s 1200s cargo +1.99.1 test --locked --test disk_cache_reads --test read_windows --test read_refill --test read_reply --test cache_loans --test metrics
```

### Stage 7: Verify cross-tier correctness and resource progress

**What to implement:**

- Through `Filesystem` and real SDK/HTTP boundaries, verify L1 eviction followed
  by an L2 byte-correct hit with no extra payload GET. Verify excluded files miss
  normally, L2 hits promote to L1, partial ranges and EOF never expose holes.
- Cover changed ETag at the same size, same-ETag metadata changes, deleted/
  recreated names, rename across the regex boundary, write/truncate/regrow, failed fsync
  and concurrent old-generation fills. Restart must not reuse inode identities.
- Gate slow disk and S3 separately; test same-range coalescing, independent-range
  progress, waiter/leader cancellation, saturated spill admission, loan revocation
  and unselected-reader/writer progress. Optional fill errors cannot revoke a reply.
- Verify known-stale records are never served, unaffected old records survive
  without payload TTL, and evicting one record never mutates the source S3 object.
- Verify disabled-mode compatibility and metrics semantics; restore all injected
  faults. Run full affected suites, denied-warning lint and public-source audit.

**Files:** `tests/disk_cache_reads.rs`, `tests/disk_cache_recovery.rs`,
`tests/backend_contract.rs`, `tests/cache_loans.rs`,
`tests/write_review_regressions.rs`, `tests/namespace_review_regressions.rs`,
`tests/fuse_deadline_review.rs`, `tests/metrics.rs`.

**Examples:** the existing public-interface suites above and `tests/read_views.rs`.

**Verification:**

```bash
timeout --kill-after=10s 1800s cargo +1.99.1 test --locked --all-targets --no-fail-fast
timeout --kill-after=10s 1800s cargo +1.99.1 test --locked --all-targets --all-features --no-fail-fast
timeout --kill-after=10s 1800s cargo +1.99.1 clippy --locked --all-targets --all-features -- -D warnings
timeout --kill-after=10s 60s cargo +1.99.1 fmt --all -- --check
timeout --kill-after=10s 120s python3 scripts/audit_public_sources.py
timeout --kill-after=10s 180s python3 -m unittest discover -s tests -p 'test_*.py'
timeout --kill-after=10s 180s python3 -m unittest discover -s bench -p 'test_*.py'
```

### Stage 8: Qualify mounted workloads and deliver the reviewed change

**What to implement:**

- Add opt-in native FUSE fixtures for package full/Range/EOF reads through nginx,
  rsync, RAM-pressure revisit, disk eviction, restart and local-disk failure.
  Use independent byte hashes and disposable namespaces/backends.
- Extend existing benchmark accounting with disk I/O, write amplification, cache
  index/queue memory and recovery time. Compare frozen RAM-only Rust and candidate
  with identical total payload budgets; retain Go 0.43.9 reference measurements.
- Run at least eight independent mount pairs and corresponding same-executable
  A/A controls for declared targets. Fix dataset generations, storage device,
  available resources, traffic, concurrency and cache-state protocol. Report
  L1-hot, L2-hot/page-cache-hot, L2-hot/page-cache-cold and both-cold separately.
- Include mixed hot/cold traffic and a working set larger than RAM, SSD/HDD
  applicability, sequential/random/concurrent reads, metadata and writer fsync.
  Report CPU, throughput, latency, steady/peak RSS, allocator, total cgroup and
  file/kernel memory, disk occupancy/IOPS and dropped cache admissions. Additional
  disk resources are explicit, not evidence of a free memory/performance win.
- Preserve raw failures, paired intervals, non-inferiority results and missing
  metrics. Distinguish feature qualification from full production acceptance.
- Update architecture/configuration/verification docs and the public-source
  dependency audit. After implementation approval and checks, commit only task
  changes, push the task branch and open a draft PR after checking target and
  auto-publish/merge settings. Audit the relationship to the existing unmerged
  base before choosing the PR target; do not merge or deploy.

**Files:** `tests/disk_cache_mount.rs`, `scripts/integration.py`,
`bench/disk_cache.py`, `bench/test_disk_cache.py`, `bench/accounting.py`,
`docs/architecture.md`, `docs/testing.md`, `docs/dependency-decisions.md`,
`docs/change-history.md`, `README.md`.

**Documentation/examples:** `docs/testing.md`, `docs/performance-methodology.md`,
`tests/fuse_directory.rs`, `bench/compare.py`, `bench/paired.py`,
`bench/sealed_s3_fixture.py`. Extend the existing harness rather than writing a
second unrelated fixture or statistics framework.

**Verification:**

```bash
timeout --kill-after=10s 1800s cargo +1.99.1 build --locked --release
timeout --kill-after=10s 300s unshare --mount --propagation private cargo +1.99.1 test --locked --test disk_cache_mount -- --ignored --test-threads=1
timeout --kill-after=30s 1200s python3 scripts/integration.py --go-reference "$GEESEFS_GO_REFERENCE"
timeout --kill-after=10s 180s python3 -m unittest discover -s bench -p 'test_*.py'
```

Define the new `bench/disk_cache.py` CLI in this stage with explicit baseline and
candidate binaries, workload manifest, output directory, pair/control counts and
an aggregate deadline. Record its exact measurement commands in private receipts;
do not present existing benchmark flags as supporting an unimplemented L2 matrix.
