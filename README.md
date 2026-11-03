# geesefs-rs

Experimental Linux/S3 GeeseFS replacement for readers and writers.
**Not accepted for production:** compatibility and comparative performance remain
incomplete. Three separately authorized read-only readers are covered by the
[scoped serving trials](docs/pr-context.md#scoped-serving-trial--2026-11-02),
without general deployment acceptance.

The goal is preserved filesystem/S3 behavior with lower memory use and faster
normal, sustained operation than **Go 0.43.9**, the primary reference. Earlier
unmerged Go optimization results remain historical evidence. A language change,
passing unit tests or a favorable workload does not establish acceptance.

Compare correctness-equivalent behavior: do not reproduce demonstrated stale
metadata or weaker durability to win. Go-equivalent bounded negative caching is
accepted; immediate external-create visibility beyond Go is not an extra gate.

## Development

Use Rust 1.96.0+ with rustfmt/clippy and Python 3.12+ with
`requirements-fixture.txt`. Dependencies are public crates; this package forbids
unsafe code. Builds, mounted tests and benchmarks use isolated disposable fixtures.

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
python3 -m unittest discover -s bench -p 'test_*.py'
```

The foreground binary integrates fuser, SDK-backed S3, bounded payload ownership,
paged namespace operations and generation-aware sparse/truncate/fsync writes.
See [architecture](docs/architecture.md) for delivered mechanisms and
[compatibility](docs/compatibility.md) for gaps.

## Current checkpoint — 2026-11-02

- Explicit `-o allow_other` now admits other UIDs through the supported fuser
  session policy while retaining kernel permissions and the owner-only default.
  A public [three-case native regression](docs/testing.md#cross-uid-mount-access--2026-11-02)
  verifies independent readable bytes, default denial and mode-0600 denial.
  This closes the earlier frozen-binary access failure, not full target acceptance.
- The [listing diagnosis](docs/performance-listing-diagnosis.md) attributes five
  absent-name checks to 15 S3 requests per warm default-Rust listing. Eight blocks
  measure **18.493 ms** default Rust, **3.095 ms** with explicit negative64 and
  **4.190 ms** Go 0.43.9. Compare with the accepted explicit configuration and the
  same `--stat-cache-ttl=1m`; 64 is capacity per directory, not seconds. The cache
  default remains zero. Five seconds was only a separate expiry probe.
- [Read-shape controls](docs/performance-read-shape.md) complete 40 isolated mounts
  with genuine A/A controls and independent nginx/golist/rsync bytes. A separate
  bounded native-S3 attempt stops after five complete blocks on a dependency
  timeout, below the eight-pair gate. Short-window memory is not a plateau.
- [Classic dispatch controls](docs/performance-classic-dispatch.md) retain the
  reproduced rename-notification deadlock correction. Inline first polling is
  rejected for receive-thread liveness; extra receive loops retain allocation/rate
  objections. Other short scheduling/reuse trials remain absent and inconclusive.
- [Actual LXC checks](docs/performance-target-preflight.md) cover disposable
  nginx/rsync contracts. Target profile/service-user compatibility, sustained
  large-namespace memory, broad quality and representative comparative gates
  remain open.

## Measurements and remaining work

Use the [comparison contract](docs/performance.md) and
[paired statistical protocol](docs/performance-methodology.md): equivalent work,
at least eight independent mounts, same-executable/configuration A/A controls,
paired intervals and predeclared non-inferiority margins. Publish tails, CPU/work,
daemon/client RSS and allocator/cgroup/file/kernel memory separately. Missing or
biased observations stay inconclusive; sampled late means do not prove a plateau.

Frozen checkpoints retain favorable and adverse results and executable identities:

- [Integrated metadata/handle measurements](docs/performance-integrated.md).
- [Directory batching](docs/performance-directory-remainders.md),
  [metadata observation age](docs/performance-observation-age.md) and
  [reference permission freshness](docs/metadata-expiry-diagnosis.md).
- [Owned uploads](docs/performance-owned-uploads.md) and
  [historical cache-policy/mixed measurements](docs/performance-cache-policy.md).

[Next work](docs/next-work.md) lists actionable gaps. FUSE io_uring is an
[upstream watch](docs/fuse-io-uring-assessment.md); passthrough and disk-backed
caching remain deferred.

## Project context

- [Project brief](docs/project-brief.md) and [agent rules](AGENTS.md).
- [Change history and reasons](docs/change-history.md), [PR context](docs/pr-context.md).
- [Backend/mount options](docs/backend-options.md) and [dependency decisions](docs/dependency-decisions.md).
- [Testing](docs/testing.md), [sealed read fixture](docs/immutable-read-dataset.md)
  and [distribution notices](docs/distribution.md).

Detailed results, generated graphs and execution journals belong in ignored
`benchmark-results/private/`, not source diffs. Tracked documentation keeps the
contract, confirmed mechanisms, verification scope and remaining gaps.
