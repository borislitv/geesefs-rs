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
  All 85 frozen source/build/test inputs and the binary still match after package
  installation. Do not restore the rejected experimental runtime/test changes.
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

## CPU-scope correction and authorized reboot

The operator intended four CPUs for the io_uring experiment, not four CPUs for the
whole VM. The agent incorrectly implemented that scope with global `nr_cpus=4`.
The guest has since booted 7.3.1 with only four possible/online CPUs. This was an
agent configuration error, not an operator-requested guest resource reduction.

The global limit is now removed from the next-boot entry. Four-CPU placement is
implemented separately for the standalone lab process tree. The operator explicitly
authorized correcting the configuration **and rebooting**. This pre-reboot
checkpoint must not be mistaken for proof that all CPUs have already returned;
read the automatic postboot receipt below. Production mounts/data/credentials
remain outside this task.

### Verified host and package plan

- Disposable KVM guest: Ubuntu 26.04 LTS `resolute`, amd64, currently running
  `7.3.1-070301-generic`; GRUB/BIOS and existing dracut `110-11ubuntu0.1`.
  Preserve the installed 7.0 kernels and existing initramfs tooling.
- The original guest has 256 possible/configured CPUs and 64 online CPUs. The
  mistaken boot currently exposes only `0-3`. Removing `nr_cpus=4` requires a new
  boot; CPUs outside the possible mask cannot simply be onlined at runtime.
  The corrected entry retains `fuse.enable_uring=1` without a global CPU limit.
- Normal generic/HWE APT candidates are still 7.0, not the required 7.3 pool UAPI.
  [Canonical mainline 7.3.1 amd64](https://kernel.ubuntu.com/mainline/v7.3.1/amd64/)
  provides matching image/modules version `7.3.1-070301.202610301300`.
- Installed only `linux-image-unsigned-7.3.1-070301-generic`,
  `linux-modules-7.3.1-070301-generic` and `liburing-dev=2.14-1`.
  Both kernel packages have version `7.3.1-070301.202610301300`.
  `liburing2=2.14-1` was already installed. The fresh solver and actual installation
  both show three new packages, zero upgrades and zero removals.
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

Dracut created `/boot/initrd.img-7.3.1-070301-generic` (44,525,163 bytes). The matching
vmlinuz, config, System.map, modules.dep and initrd contents are verified; package
integrity, `dpkg --audit`, `apt-get check` and `grub-script-check` pass. Both normal
7.0 kernels and their boot entries remain. Standard package hooks initially made
7.3.1 the first normal entry; the corrected configuration selects the dedicated
full-guest-CPU entry below, without a one-shot override. Existing cloud-image console
arguments are preserved. The target kernel builds virtio block/PCI/network, ext4,
devtmpfs and serial-console support in, rather than relying on missing initrd modules.

### Installed isolated userspace

Pinned libfuse source `fe66006fbbe2ba32e8e1252df610a948b30c3f43` identifies as
3.19.0-rc0 and includes the pool merge. Archive SHA256 is
`995e04d9d439e12af1301041f15f0e1eb001a6bb78dacdf85a0f77602d5aebbb`.
The initial private-prefix build was followed by an installation against system
liburing 2.14 at **`/opt/libfuse-3.19.0-rc0-fe66006`**. Its build uses:

```text
meson setup <build> <source> --buildtype=release --prefix=<isolated-prefix> \
  --libdir=lib --bindir=/usr/bin \
  -Dutils=false -Dtests=false -Dexamples=true -Denable-io-uring=true -Duseroot=false
ninja -C <build> -j2 example/hello_ll example/passthrough_ll example/notify_inval_inode
meson install -C <build> --no-rebuild
```

Check `HAVE_URING` in generated private `fuse_config.h`, not the version-only
public `libfuse_config.h`; Meson emits the defined boolean without a numeric 1.
Both checking mistakes remain in private receipts, not reclassified as library
build failures. Every Meson install destination was verified inside the prefix;
`--bindir=/usr/bin` selects the existing mount helper, with utils installation off.

The unmodified upstream `hello_ll`, `passthrough_ll` and `notify_inval_inode`
examples were then compiled against that installed API into its `bin/` directory.
All three help checks and the hello version check pass. Relative
`$ORIGIN/../lib` runpaths and ldd resolve the isolated libfuse and system liburing,
without the ignored build tree. Source archive, licenses, config and provenance
are in `share/libfuse-lab/`. System libfuse3 3.18.2-1 and mount-helper hashes are
unchanged; no global loader path, helper or service was replaced.

Existing perf 7.0.14 records and decodes a bounded child-only CPU smoke test
(six samples) on the **current 7.0 kernel**; tcpdump is available. Repeat the perf
recording smoke test after the future 7.3 boot. No example was mounted, no new
Rust transport was selected and no 7.3 runtime/performance result is claimed.

### Ready next boot and laboratory launcher

The default GRUB entry is **`geesefs-fuse-uring-7.3.1`**, displayed as
`Ubuntu 7.3.1 FUSE io_uring (all guest CPUs)`. It loads the matching 7.3.1 kernel
and initrd with the existing root/console arguments plus exactly:

```text
fuse.enable_uring=1
```

- `/boot/grub/custom.cfg` contains the dedicated entry, using this guest's verified
  generated boot device/root arguments. The standard `41_custom` hook loads it.
- `/etc/default/grub.d/99-geesefs-fuse-uring.cfg` selects that stable entry ID after
  the cloud-image defaults and enables a five-second menu, including recordfail.
- `grub-mkconfig` was rendered and checked before `update-grub`; the resulting live
  config matches the candidate. Both generated and custom syntax checks pass.
  The normal Linux menu block is byte-identical to its saved pre-change version.
  Normal/recovery entries for 7.0.0-38 and 7.0.0-30 keep their original arguments.
  For an ordinary-kernel boot, select the existing 7.0 entry in Advanced options.
- Neither `nr_cpus` nor `maxcpus` is present in the corrected entry. There is no
  one-shot selection. Backups and hashes of the previous configuration are private.

The lab prefix now also has `bin/check-boot` and `bin/run-fuse-lab`. After reboot,
run the read-only check as root:

```sh
timeout 20s /opt/libfuse-3.19.0-rc0-fe66006/bin/check-boot
```

It requires the target kernel, no global CPU-cap parameters, restored guest CPU
counts, enabled FUSE io_uring, allowed generic io_uring, accessible `/dev/fuse`,
mount capability and the pinned userspace. It now rejects the mistaken four-CPU
boot rather than treating that resource reduction as readiness. This is not a
transport mount test.

The existing caller memlock is 8 MiB. `run-fuse-lab classic COMMAND ...` explicitly
sets both FUSE enable/pool flags to 0; `run-fuse-lab uring COMMAND ...` sets both to
1 and refuses to execute the command until `check-boot` succeeds. Both modes select
queue depth 8 and apply **64 MiB soft/hard memlock to the child only**. A dedicated
owned systemd scope sets `AllowedCPUs=0-3`, with matching initial task affinity.
The child cannot widen its CPU placement beyond that cpuset. Other VM processes
retain the full topology; this is not CPU isolation or a guest-wide CPU limit.

The cpuset covers the entire standalone C lab process tree. It is not a delivered
io-thread-only affinity implementation for Rust/Tokio; the Rust transport remains
unchanged. Inspected libfuse creates one worker/queue per configured possible CPU
and attempts to bind each qid to its CPU. A cpuset prevents those attempts from
escaping the selected four CPUs, but does not reduce worker/queue count or payload
allocation. At 256 possible CPUs and depth 8 its nominal 1 MiB payload capacity is
2 GiB, not the earlier four-queue 32 MiB estimate. Reassess bounded transport/pool
ownership before a performance run; do not hide this cost by shrinking the VM.

`geesefs-cpu-restore-check.service` is enabled for the next boot. It records changed
boot identity, absence of CPU-cap parameters, restored unpinned guest counts and a
separate child whose affinity stays `0-3` even after an attempted widening. Receipt:
`benchmark-results/private/vm-20261031-integrated/cpu-scope-correction-v1/postboot.json`.
The one-shot skips later boots once that receipt exists. Until it succeeds, restored
CPU topology must remain unverified in progress reports.

### Execution checklist

- [x] Capture pre-install boot/package state; repeat signed hash/control checks,
  package health, free-space checks and the exact three-package solver plan.
- [x] Install with finite deadlines, noninteractive APT, process-local
  `NEEDRESTART_SUSPEND=1` and shutdown inhibitor, without upgrades/removals/reboot.
- [x] Install and verify the isolated pinned userspace, examples and profiling
  tools, retaining system library/helper hashes and upstream source/licenses.
- [x] Verify package integrity/health, boot artifacts, initrd, GRUB/recovery
  entries and unchanged running kernel, boot identity, possible CPUs and sysctls.
- [x] Correct the erroneous global CPU cap, preserve the recovery menu and scope
  laboratory CPU placement/memlock separately from the VM.
- [ ] Confirm the authorized corrective reboot and full CPU restoration from the
  automatic postboot receipt, then finish delivery verification.

For each checkpoint delivery, append the private action/reason log, commit/push
the documentation, verify the existing draft PR and its footer link, and report
actual scope and remaining gaps in Tracker in Russian. Do not convert a
reboot-required marker or package/build readiness into runtime acceptance.

## Subsequent work after an authorized test boot

1. Execute the now-authorized corrective reboot. Read the postboot receipt and
   verify the original 64 online / 256 possible CPU topology, then run the perf
   recording smoke test. The four-CPU constraint belongs only to the lab cpuset.
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
   transport on the same kernel/full guest topology with local four-CPU placement.
   For the Rust adapter, keep transport affinity separate from Tokio workers.
   Hold application workers,
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
`ubuntu-uring-userspace-v1`. Completed installation receipts are in
`ubuntu-uring-installed-v1`, `ubuntu-uring-userspace-installed-v1` and
`ubuntu-uring-installation-checked-v1`; `actions.md` records actions/reasons.
The bounded helpers are `install_uring_without_reboot.py`,
`install_uring_userspace.py` and `verify_uring_installation.py`. The subsequent
`ubuntu-uring-boot-ready-v1` and `prepare_uring_boot.py` preserve the prior mis-scoped
four-CPU configuration and its checks, not a correct interpretation of CPU pinning.
`cpu-scope-correction-v1` and `correct_cpu_scope.py` supersede it with the corrected
boot/cpuset scope, backups, reboot authorization and automatic postboot verification.
Raw sources, commands, failures and hashes remain there; they are not public PR attachments.
Use new output labels/directories: earlier receipts are immutable and helpers
often use exclusive creation. Do not rerun an old label over an existing result.

Before resuming, read this file, AGENTS.md and the project brief; inspect the live
branch/status and PR draft/auto-merge state. Use explicit finite timeouts and
noninteractive external operations. Check supported upstream APIs before another
workaround. Credentials and private operational identifiers stay out of commits.
