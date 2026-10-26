# geesefs-rs

Rust replacement for GeeseFS, developed through BMAD's `bmad-build-auto` loop.

## Acceptance contract

The implementation must preserve the required Linux/S3 reader and writer
behavior while using less memory and running faster than an identified Go
reference under equivalent workloads and budgets. Rust alone is not evidence
of either improvement. Full compatibility and performance acceptance remain
pending until independently verified.

Read [the project brief](docs/project-brief.md) and [agent rules](AGENTS.md).
The mandatory dependency policy is **reuse first**: evaluate maintained Rust
crates before custom infrastructure, including using an existing FUSE crate.

## Development setup

Install `uv`, a current stable Rust toolchain with rustfmt/clippy, Node.js and
the GitHub CLI. The initial environment uses Rust 1.99.1 and uv 0.13.5.
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

Development bootstrap is established. The Rust filesystem implementation,
compatibility suite and comparative performance acceptance are not complete.
