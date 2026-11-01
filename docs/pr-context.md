# PR context and continuation plan — 2026-11-01

This is the resumable checkpoint for [PR2](https://github.com/borislitv/geesefs-rs/pull/2).
Update this file when completing a step; keep detailed execution logs and private
receipts in ignored evidence, not in this document. The authoritative requirements
remain [project brief](project-brief.md) and [AGENTS.md](../AGENTS.md).

## Goal and delivery state

- Deliver a Linux/S3 Rust replacement for both readers and writers, reusing an
  existing FUSE implementation and public dependencies. FUSE passthrough and a
  disk-backed cache remain deferred. Local unsafe code is forbidden.
- Overall status: **UNACCEPTED**. Keep the PR open and draft, targeting `main`,
  without auto-merge. Library CI is not native-filesystem or production acceptance.
- Branch: `perf/prove-read-bottlenecks-20261030`. The preceding research checkpoint
  is `a724dbb617b4d99a0475823d8cd49915c69b99b9`; subsequent checkpoint edits are
  documentation and test-environment preparation, not a new application runtime.
- Retained runtime: `1b13033c3f161b9502c7a23870b645ca871ab2e3`.
  Release SHA256: `df24b6bdd42920b7989db2ed7c44855b9fc2bb1cf53292ee60ce023eb42138a8`.
  All 85 frozen source/build/test inputs matched committed blobs at the preceding
  checkpoint. Do not restore the rejected experimental runtime/test changes.
- The retained verification scope is 260 Rust cases per allocator, 75 benchmark
  Python cases, ten native directory contracts and three matching native variants.
  These results belong to that runtime, not to an untested io_uring integration.

## Established findings and remaining priorities

The retained corrections cover reclaimable-cache admission wakeups, scoped
directory invalidation, bounded LIST continuation, exact metadata observation
age, per-open history, owned upload/reply lifetimes and last-close notification.
See [architecture](architecture.md) and [change history](change-history.md).

The operator's 2026-11-01 clarification requires Go-relative superiority only for
**correctness-equivalent** behavior. Six equal-TTL probes demonstrate stale Go
permission metadata at unchanged content ETag, including after a successful HEAD.
Do not reproduce that defect or remove necessary durability/validation work to
win. Keep raw measurements and distinguish required checks from avoidable cost.

- [Current mixed measurements](performance-observation-age.md): 1,440 checked
  phases and eight final byte/generation checks across two rotated repetitions.
  Rust random reads are 1.73/1.79 MiB/s versus experimental Go 1.91/1.97; writer
  rates are 23.39/26.13 MiB/s. Late daemon RSS 157.26/159.03 MiB is not a reproducible
  process-memory win. Sampled peaks or late means do not establish a plateau.
- [Lookup-probe overlap](performance-lookup-probes.md) preserves RPC counts but
  fails the joint tail/RSS gate. [Cached-observation reuse](performance-cached-observations.md)
  also fails a joint measured gate. Both runtime/test trials were discarded.
- [Expiry diagnosis](metadata-expiry-diagnosis.md) identifies Rust's 1,024 HEADs
  and three LISTs versus Go's two HEADs and four LISTs, then demonstrates the
  permission-freshness difference. SDK/kernel CPU categories are not additive
  wall-time or proof that all kernel cost is FUSE. No supported prepared-SDK API
  or justified minimal runtime correction was established.
- Independent priorities remain random/concurrent-read tails and CPU, writer
  request/setup attribution, long-lived memory ownership, actual-service and
  remaining Linux/S3 contracts, observer/source-bound controls and whole-file
  quality. Whole-file CRAP still fails; targeted mutation results do not close
  broader survivors. Positive LINK, full CLI/imported multipart layouts and
  actual-service IAM/SSE/signing/PATCH are not claimed complete.

## Immediate task: install experiment prerequisites without rebooting

The operator authorized installing the required packages, **without a reboot**.
At this checkpoint installation has not started. Do not reboot, schedule a
reboot, kexec, replace served mounts or modify production data/credentials.

### Verified host and package plan

- Disposable KVM guest: Ubuntu 26.04 LTS `resolute`, amd64, running
  `7.0.0-38-generic`; GRUB/BIOS and existing dracut `110-11ubuntu0.1`.
  Preserve the installed 7.0 kernels and existing initramfs tooling.
- Possible CPUs are `0-255`, configured CPUs 256, online CPUs 64.
  `fuse.enable_uring=N`, `kernel.io_uring_disabled=0`. No boot parameter/sysctl
  was changed. Current FUSE and io_uring options are compiled in.
- Normal generic/HWE APT candidates are still 7.0, not the required 7.3 pool UAPI.
  [Canonical mainline 7.3.1 amd64](https://kernel.ubuntu.com/mainline/v7.3.1/amd64/)
  provides matching image/modules version `7.3.1-070301.202610301300`.
- Install only `linux-image-unsigned-7.3.1-070301-generic`,
  `linux-modules-7.3.1-070301-generic` and `liburing-dev=2.14-1`.
  `liburing2=2.14-1` is already installed. The verified solver plan is three new
  packages, zero upgrades and zero removals; repeat it immediately before install.
- Do not use the four-package headers wildcard. Mainline generic headers require
  unavailable `libssl4>=4.0.0`; no external module is being built and the userspace
  build succeeds without those headers. Do not mix distributions to satisfy it.
- Signed CHECKSUMS were verified with primary fingerprint
  `60AA7B6F30434AE68E569963E50C6A0917C622B0`. Downloaded package SHA256 values:

| Archive | SHA256 |
| --- | --- |
| `linux-image-unsigned-7.3.1-070301-generic_7.3.1-070301.202610301300_amd64.deb` | `93c7c059e51cd81d7aa817f0cb0d2a7e94c5b04642a8e20de8b4ab4628b6a357` |
| `linux-modules-7.3.1-070301-generic_7.3.1-070301.202610301300_amd64.deb` | `7f9a44175036fd63b01ec317ccace29e184261a794d39046902439458fb18c2f` |
| `liburing-dev_2.14-1_amd64.deb` | `738e18a2d43fe51b9b0b5015fc87d0ffadc9e7ac6834edb6c3d06407a7427454` |

Mainline kernels are unsupported diagnostic builds. Its published amd64 self-test
build failed for missing build dependencies/targets; neither a passed runtime test
nor a FUSE runtime failure is established. Follow the
[current Canonical instructions](https://canonical-kernel-docs.readthedocs-hosted.com/how-to/testing-verification/test-with-mainline-kernel/).

### Userspace already proved locally

Pinned libfuse source `fe66006fbbe2ba32e8e1252df610a948b30c3f43` identifies as
3.19.0-rc0 and includes the pool merge. Archive SHA256 is
`995e04d9d439e12af1301041f15f0e1eb001a6bb78dacdf85a0f77602d5aebbb`.
A private build against extracted Ubuntu liburing 2.14 packages successfully
builds `example/hello_ll` and runs `--help`, using:

```text
meson setup <build> <source> --buildtype=release --prefix=<isolated-prefix> \
  -Dutils=false -Dtests=false -Dexamples=true -Denable-io-uring=true -Duseroot=false
ninja -C <build> -j2 example/hello_ll
```

Check `HAVE_URING` in generated private `fuse_config.h`, not the version-only
public `libfuse_config.h`. The initial wrong-header verifier failure is retained.
Nothing has been installed system-wide. Keep system libfuse3 3.18.2-1 and helpers;
use a separate versioned prefix for the experimental userspace. Existing perf
7.0.14 must pass a recording smoke test on the eventual kernel before reuse.

### Execution checklist

- [ ] Capture running kernel, boot identity/cmdline, installed package and boot
  state; repeat signature/hash verification, package health and the exact solver
  plan. Preflight currently has clean `dpkg --audit` / `apt-get check` and enough
  root/boot space. Historical failed benchmark units are not package failures.
- [ ] Install the three verified packages with finite deadlines, noninteractive
  APT, process-local `NEEDRESTART_SUSPEND=1` and the verified shutdown inhibitor.
  Do not perform a broad upgrade/autoremove, change running sysctls or reboot.
- [ ] Build/install the pinned userspace into an isolated versioned prefix with
  system liburing-dev; verify HAVE_URING, linkage, example help and profiling tools.
- [ ] Verify package health, matching vmlinuz/config/System.map/initrd/modules.dep
  and generated GRUB entries, retaining a normal-kernel recovery entry. Confirm
  running kernel and boot identity are unchanged. A reboot-required marker is not
  evidence that a reboot occurred or authorization to perform one.
- [ ] Update this checkpoint, append the private action/reason log, commit/push
  the documentation and update the existing draft PR. Report actual installation
  scope and remaining boot/integration gaps in Tracker in Russian.

## Subsequent work after an authorized test boot

1. Obtain separate reboot authorization. For a controlled boot use the reviewed
   `fuse.enable_uring=1 nr_cpus=4` or 2 configuration; verify `uname`, possible CPU
   mask, configured count, FUSE parameter, `/dev/fuse`, io_uring policy and memlock.
   `maxcpus`, quota, affinity or fewer workers do not shrink possible-CPU queues.
2. Prove actual initialization/no fallback in the standalone pool smoke test.
   Explicit controls are `FUSE_URING_ENABLE=1`, `FUSE_URING_BUFPOOL=1`,
   `FUSE_URING_QUEUE_DEPTH=4` or 8. Legacy controls must set both ENABLE and
   BUFPOOL to 0 because BUFPOOL enables transport. Inspected libfuse still
   allocates/populates depth times payload per qid: pools do not automatically
   deliver a smaller backing or client-page zero-copy.
3. Select a supported reused Rust transport/adapter using the
   [versioned assessment](fuse-io-uring-assessment.md) and
   [dependency decisions](dependency-decisions.md). Current fuser 0.18.0 is
   read/writev; a new kernel or libfuse install does not change our Rust transport.
   The unmerged Skory stack is the closest API candidate, not an accepted library
   release. Do not implement a new FUSE protocol or local unsafe/FFI shim.
4. First prove full bytes, fresh modes/generations, fsync failure/recovery,
   cancellation/unmount, mmap, delivered FORGET/PLUS references, notifications
   and reply/write/buffer ownership through completion. Cover every required qid
   while investigating 1/2 transport workers and shallow depths 4/8.
5. Compare one-loop and bounded multi-loop classic fuser controls with the reused
   transport on the same kernel and verified 2/4 possible CPUs. Hold Tokio workers,
   budgets, TTL, signing/checksums, negative-cache policy and post-commit HEAD fixed.
   Rotate repeated cache-hit metadata/4 KiB reads, expiry, S3 misses and writer/fsync
   runs. Separate basic transport, pools and zero-copy; report pooled tails,
   CPU/work and process/allocator/cgroup/kernel/pinned memory with pressure deltas.
6. Resume the independent [remaining priorities](next-work.md). Neither a boot,
   standalone C mount nor a microbenchmark closes the complete Rust/S3 acceptance
   gate or authorizes deployment.

## Evidence and resumption

Ignored evidence root: `benchmark-results/private/vm-20261031-integrated/`.
Relevant groups are `candidate-v8`, `kernel73-research-v1`, `kernel73-reuse-v1`,
`ubuntu-uring-prerequisites-v1`, `ubuntu-uring-kernel-plan-v1` and
`ubuntu-uring-userspace-v1`; `actions.md` records actions/reasons. Raw sources,
commands, failures and hashes remain there; they are not public PR attachments.
Use new output labels/directories: earlier receipts are immutable and helpers
often use exclusive creation. Do not rerun an old label over an existing result.

Before resuming, read this file, AGENTS.md and the project brief; inspect the live
branch/status and PR draft/auto-merge state. Use explicit finite timeouts and
noninteractive external operations. Check supported upstream APIs before another
workaround. Credentials and private operational identifiers stay out of commits.
