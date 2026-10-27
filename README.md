# geesefs-rs

Experimental Rust replacement for GeeseFS. BMAD was used for planning,
implementation delegation and independent review; continued use is optional.

## Acceptance contract

The implementation must preserve the required Linux/S3 reader and writer
behavior while using less memory and running faster than an identified Go
reference under equivalent workloads and budgets. Rust alone is not evidence
of either improvement. Full compatibility and performance acceptance remain
pending until independently verified.

Read [the project brief](docs/project-brief.md) and [agent rules](AGENTS.md).
The mandatory dependency policy is **reuse first**: evaluate maintained Rust
crates before custom infrastructure, including using an existing FUSE crate.
The package forbids unsafe code for all targets and CI rejects non-public Cargo
dependency origins. No internal Arcadia libraries are copied, translated or linked.

## Development setup

Build with Rust 1.96.0 or newer, rustfmt/clippy and the native compiler required
by the public allocator/TLS dependencies. Python 3.12+ runs the evidence checks;
mounted comparison additionally needs Linux FUSE, rsync, Go and the test requirements.
The initial environment uses Rust 1.99.1.

Install the public Python benchmark dependencies in your selected virtual
environment before the evidence/CPU tests: `pip install -r requirements-bench.txt`.

Optional BMAD setup uses `uv` (initial version 0.13.5), Node.js and GitHub CLI.
BMAD skills are project-local; `skills-lock.json` records their source hashes.
Install the selected official skills:

```bash
npx --yes skills add bmad-code-org/BMAD-METHOD --skill bmad bmod-core-tools bmod-method bmad-build-auto bmad-build bmad-prd bmad-architecture bmad-code-review bmad-review bmad-project-context bmad-deep-recon bmad-agent-dev bmad-agent-architect bmad-agent-analyst bmad-agent-pm bmad-agent-po --agent opencode -y
```

Read the installed `.agents/skills/bmad/SKILL.md` and follow its setup procedure.
After setup, read `.agents/skills/bmad-build-auto/SKILL.md` and invoke its
renderer exactly as instructed from this project. Do not run workflow sources
directly or claim the unattended loop is running when it has halted.

The input is `docs/project-brief.md`; generated plans and actual run results
belong under `_bmad-output/`. Commit initial configuration before starting a
build iteration: BMAD requires a clean task branch.

The requested [ub-mode workflow](https://github.com/unclebob/ub-mode) is applied
to continued development. Its first [quality checkpoint](docs/quality-results-20261027.json)
and [execution log](_bmad-output/ub-mode-20261027.md) retain complexity findings,
duplicate candidates and a verified improvement to adjacent-read boundary tests.
See the [glossary](docs/glossary.md) for ownership and measurement terminology.

## Status

**Experimental read/write mounts work; do not replace a served mount.** The
foreground binary integrates fuser, SDK-backed S3, bounded payload ownership,
paged namespace operations and versioned sparse/truncate/fsync writes. Actual
disposable Rust/Go mounts exercised rsync, HTTP full/Range and writer paths.
The compatibility runner still fails: actual-service optional paths, hardlink
emulation and a Go failure-recovery check are unresolved. Actual FUSE tracing and
clean-buffer repairs resolved the severe historical sequential slowdown. The
latest selected local matrix completes all 80 results with correct bytes and
higher group mean throughput, but some cold tail metrics still regress.
An independent real-object Kubernetes diagnostic also completed all 18 phases
correctly; Rust fresh throughput was **198–204 MB/s versus Go 225–262 MB/s**.
Lower daemon RSS does not establish lower total cgroup memory when file cache
fills the limit. [Bounded metadata reuse](docs/metadata-page-reuse.md) has library
regressions; its whole-filesystem speed benefit remains unestablished. New test
windows include separate daemon/client/cgroup user/system CPU cost, efficiency
and throttling. All favorable and unfavorable results remain in
[performance](docs/performance.md). Overall acceptance is
**UNACCEPTED**. Owned cpu+memory cgroup execution works without changing the
hosting service's controls.

The [50-phase CPU-accounted diagnostic](docs/k8s-cpu-results-20261027.md)
completed, but 214 objects disappeared during the run. It retained 5,522 file
failures and 146 listing mismatches; its lower observed CPU costs do not prove
a win on equivalent data. Six EAGAIN failures remain separately unresolved.
The [metadata coherence follow-up](docs/metadata-coherence.md) reproduces and
repairs unrelated-name EAGAIN plus response-publication ownership races with ten
new behavior tests. Attribution of the six historical failures and a valid
post-fix performance comparison are still open.

Backend IAM/profile authentication, signature discovery/SigV2, encryption and
request options now have loopback HTTP/TLS coverage. See [backend options](docs/backend-options.md)
for supported semantics and remaining limits; PATCH now has writer
generation/conflict/recovery integration and deterministic tests. Configured
multipart tiers, streamed large parts, bounded parallel copy/upload, effective
read-ahead windows and bounded negative lookup caching are also covered by
deterministic tests. Owner-budget reporting does not establish an RSS bound.
These additions
require Rust 1.96.0 or newer and do not
establish actual-service or production acceptance.

```bash
cargo test --locked --all-targets --all-features
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

See [compatibility](docs/compatibility.md), [dependency decisions](docs/dependency-decisions.md),
[testing](docs/testing.md) and [performance](docs/performance.md) for reproduction
commands and exact failures. Full compatibility, writer/fsync durability and
comparative performance acceptance remain incomplete. CI success covers
deterministic library/contract/evidence-gate checks, not production acceptance.
