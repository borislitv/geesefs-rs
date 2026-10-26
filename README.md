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

## Status

**Experimental read/write mounts work; do not replace a served mount.** The
foreground binary integrates fuser, SDK-backed S3, bounded payload ownership,
paged namespace operations and versioned sparse/truncate/fsync writes. Actual
disposable Rust/Go mounts exercised rsync, HTTP full/Range and writer paths.
The compatibility runner still fails: actual-service optional paths, hardlink
emulation and a Go failure-recovery check are unresolved. An initial diagnostic
pair measured lower memory but substantially slower cold standard-LIST traversal
for Rust; it is not acceptance. Bounded metadata concurrency was subsequently
implemented and requires fresh measurement. Owned cpu+memory cgroup execution
has now been verified without changing the hosting service's controls.

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
