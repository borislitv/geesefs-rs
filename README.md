# geesefs-rs

Experimental Linux/S3 GeeseFS replacement for readers and writers.
**Not accepted for production:** compatibility and comparative performance remain
incomplete. Do not replace a served mount with this implementation.

The goal is preserved filesystem/S3 behavior with lower memory use and faster
normal, sustained operation than both identified Go references. A language change,
passing unit tests or one favorable workload does not satisfy that goal.

## Development

Use Rust 1.96.0 or newer with rustfmt/clippy and Python 3.12+ with dependencies
from `requirements-fixture.txt`. Builds, tests and mounted benchmarks run in
isolated Kubernetes fixtures. Dependencies are public crates; this package
forbids unsafe code. Reuse maintained library APIs before custom infrastructure.

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
```

The foreground binary integrates fuser, SDK-backed S3, bounded payload ownership,
paged namespace operations and generation-aware sparse/truncate/fsync writes.
The latest runtime correction resets adaptive read history when a closed stream
reopens, while retaining cached payload and active-reader history. Warm repeated
payload disappeared in the selected comparison; mean p99 still loses to both Go
references. Initial-read client work remains under investigation.

## Project context

- [Project brief](docs/project-brief.md) and [agent rules](AGENTS.md).
- [Change history and reasons](docs/change-history.md).
- [Compatibility inventory](docs/compatibility.md) and [backend options](docs/backend-options.md).
- [Dependency decisions](docs/dependency-decisions.md).
- [Testing](docs/testing.md), [performance methodology](docs/performance.md) and
  [sealed read fixture](docs/immutable-read-dataset.md).

Detailed measurements, generated graphs and execution logs belong in ignored
`benchmark-results/private/`, not in source diffs. Keep a concise explanation of
each confirmed mechanism, correction, tradeoff and remaining gap in the history.
