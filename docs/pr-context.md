# PR context and continuation plan — 2026-11-02

This is the resumable checkpoint for [PR2](https://github.com/borislitv/geesefs-rs/pull/2).
Update this file when completing a step; keep detailed execution logs and private
receipts in ignored evidence, not in this document. The authoritative requirements
remain [project brief](project-brief.md) and [AGENTS.md](../AGENTS.md).

## Goal and delivery state

The [2026-11-02 statistical review](performance-methodology.md) supersedes every
strict four-pair rejection below. Lookup overlap and observation
reuse are suspended/inconclusive; implementations remain absent. A real A/A arm,
eight independent mount pairs and paired CIs replace strict wins in every sample.
Extra loops retain independent buffer/rate objections. Actual LXC access is now
authorized and an isolated read-only nginx/rsync fixture passes on the actual
kernel. The initial reader inventory is idle. A subsequent loaded-neighbor
capture now completes four-source attribution and a 40-mount transformed replay;
the native-S3 attempt stops on an upstream timeout with five complete A/A blocks.
See [read-shape results and gaps](performance-read-shape.md). Historical failed
test mounts that blocked host `df` are identified and removed; future native
fixtures require private mount namespaces and actual task/mount cleanup checks.
Private host/mount identifiers stay outside public source.

- Deliver a Linux/S3 Rust replacement for both readers and writers, reusing an
  existing FUSE implementation and public dependencies. FUSE passthrough and a
  disk-backed cache remain deferred. Local unsafe code is forbidden.
- Overall status: **UNACCEPTED**. Keep the PR open and draft, targeting `main`,
  without auto-merge. Library CI is not native-filesystem or production acceptance.
- Branch: `perf/prove-read-bottlenecks-20261030`. Plan B below is now measured.
  The retained runtime removes redundant post-rename entry notifications and
  splits backend errno/fallocate validation without changing their behavior.
  Native regressions cover the deadlock, receive-thread isolation and xattr errors.
  No performance fast path, new dependency or CLI is retained. Runtime commit:
  `4a06749`; current verified jemalloc release SHA256
  `8cd8d48f8a1546d54cf46e6e52252eaca10e6b4dac6317e37a65a5542a388c62`.
  The earlier rename-only runtime `0e44bd8` and release `9949ad9f...` remain
  historical evidence, not the current build identity.
- Historical observation-age baseline: `1b13033c3f161b9502c7a23870b645ca871ab2e3`,
  release SHA256 `df24b6bdd42920b7989db2ed7c44855b9fc2bb1cf53292ee60ce023eb42138a8`.
  Its 85 frozen inputs matched after installation/reboot/native C tests. Rejected
  lookup, observation-reuse, receive-loop and inline-read trial changes stay discarded.
- That baseline's verification scope was 260 Rust cases per allocator, 75 benchmark
  Python cases, ten native directory contracts and three matching native variants.
  These historical results do not replace [current-source verification](testing.md)
  or establish an io_uring integration.

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
  has unresolved tail/RSS effects. [Cached-observation reuse](performance-cached-observations.md)
  also lacks calibrated joint evidence. Both runtime/test trials remain absent.
- [Expiry diagnosis](metadata-expiry-diagnosis.md) identifies Rust's 1,024 HEADs
  and three LISTs versus Go's two HEADs and four LISTs, then demonstrates the
  permission-freshness difference. SDK/kernel CPU categories are not additive
  wall-time or proof that all kernel cost is FUSE. No supported prepared-SDK API
  or justified minimal runtime correction was established.
- The first performance priority is read-only serving: random/concurrent-read tails
  and CPU, metadata and cache residency. Remaining work also includes writer
  request/setup attribution, long-lived memory ownership, actual-service and
  remaining Linux/S3 contracts, observer/source-bound controls and whole-file
  quality. Adapter CRAP passes; Python orchestration still fails. Targeted mutation
  results do not close broader survivors. Positive LINK, full CLI/imported multipart layouts and
  actual-service IAM/SSE/signing/PATCH are not claimed complete.

## CPU-scope correction and authorized reboot

The operator intended four CPUs for the io_uring experiment, not four CPUs for the
whole VM. The agent incorrectly implemented that scope with global `nr_cpus=4`.
The guest initially booted 7.3.1 with only four possible/online CPUs. This was an
agent configuration error, not an operator-requested guest resource reduction.

The global limit was removed and the explicitly authorized corrective reboot has
completed. The postboot receipt and independent live checks confirm **256 possible
CPUs, 64 online CPUs and unrestricted caller affinity `0-63`** on 7.3.1, without
`nr_cpus` or `maxcpus`. FUSE io_uring is enabled. A separate lab child stays on
`0-3` even when attempting to widen its affinity. Production mounts/data/credentials
remain outside this task.

### Verified host and package plan

- Disposable KVM guest: Ubuntu 26.04 LTS `resolute`, amd64, currently running
  `7.3.1-070301-generic`; GRUB/BIOS and existing dracut `110-11ubuntu0.1`.
  Preserve the installed 7.0 kernels and existing initramfs tooling.
- The restored guest has 256 possible/configured CPUs and 64 online CPUs. The
  mistaken boot exposed only `0-3`; restoring CPUs outside that possible mask
  required the corrective reboot. The selected entry retains `fuse.enable_uring=1`
  without a global CPU limit.
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

Existing perf 7.0.14 now also records and decodes a bounded child-only CPU smoke
test on **7.3.1: 129 samples**, following the earlier six-sample 7.0 check. This
establishes tool compatibility, not an application profile or performance result.
Native example mounts are verified below; the Rust transport remains unchanged.

### Verified boot configuration and laboratory launcher

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

The lab prefix also has `bin/check-boot` and `bin/run-fuse-lab`. The read-only
postboot check now succeeds as root:

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
`CAP_IPC_LOCK` can bypass memlock: the limit is not a root-process memory bound.
The native test harness separately requested a 4 GiB owned-daemon cgroup ceiling,
zero swap and 768 tasks; it did not change hosting or ancestor cgroups.

The cpuset covers the entire standalone C lab process tree. It is not a delivered
io-thread-only affinity implementation for Rust/Tokio; the Rust transport remains
unchanged. Inspected libfuse creates one worker/queue per configured possible CPU
and attempts to bind each qid to its CPU. A cpuset prevents those attempts from
escaping the selected four CPUs, but does not reduce worker/queue count or payload
allocation. At 256 possible CPUs and depth 8 its nominal 1 MiB payload capacity is
2 GiB, not the earlier four-queue 32 MiB estimate. The native tests confirm about
2,064 MiB daemon RSS in both basic and pool modes. Do not hide this cost by shrinking
the VM or describe four-CPU placement as a four-worker transport.

`geesefs-cpu-restore-check.service` completed successfully and was then disabled.
Its receipt records changed boot identity, absence of CPU-cap parameters, restored
unpinned guest counts and a separate child whose affinity stays `0-3` even after
an attempted widening:
`benchmark-results/private/vm-20261031-integrated/cpu-scope-correction-v1/postboot.json`.
The immutable receipt remains available; no background verification loop is needed.

### Native transport and notification results

The unmodified upstream `passthrough_ll` ran classic (`ENABLE=0, BUFPOOL=0`), basic
io_uring (`1,0`) and pool (`1,1`) controls on the restored topology. Each used depth 8,
`FUSE_INIT_STATUS=1`, debug output, an io_uring setup/register syscall trace and
`-o source=<disposable-source>,cache=auto,timeout=0`. All 14 checks passed per mode:
directory contents, full bytes, ranges/EOF, mode, mmap read, concurrent readers on
CPUs 0/3/31/63, create/write/fsync/reopen/backing bytes, chmod, truncate/zero fill,
rename/unlink, symlinks, directory mutation and ENOENT. These are local backing-file
contracts, not remote S3 generation, failure-recovery or Rust compatibility proofs.

Negotiation reports `io_uring=on`; requests appear as `cqe unique` rather than
legacy `dev unique`. Each uring control successfully creates all 256 rings and
retains qid workers 0 through 255 within the four-CPU cpuset. The 252 attempted
per-qid bindings outside CPU0-3 log `EINVAL`; no thread escapes. Pool controls
negotiate `FUSE_CAP_IO_URING_BUFPOOL` without per-entry-payload fallback.

| Diagnostic control | Daemon threads | Fixed pools | RSS / peak RSS, MiB | Owned daemon-scope peak, MiB |
| --- | ---: | ---: | ---: | ---: |
| Classic, memlock 64 MiB | 6 | n/a | 3.01 / 3.57 | 5.68 |
| Basic uring, memlock 64 MiB | 258 | n/a | 2,064.13 / 2,064.13 | 2,081.65 |
| Pool, `CAP_IPC_LOCK`, memlock 64 MiB | 258 | 256/256 | 2,064.13 / 2,064.13 | 2,081.84 |
| Pool, no `CAP_IPC_LOCK`, memlock 64 MiB | 258 | 8/256 | 2,064.13 / 2,064.13 | 2,081.65 |
| Pool, no `CAP_IPC_LOCK`, memlock 3 GiB | 258 | 256/256 | 2,064.12 / 2,064.12 | 2,081.92 |

The privileged control registers 2 GiB despite its 64 MiB memlock. Dropping only
the child's `CAP_IPC_LOCK` causes 248 fixed registrations to fail with ENOMEM;
filesystem checks still pass using unregistered pools. A strict fully-registered
gate correctly rejects that control. Raising only that child's memlock to 3 GiB
restores 256 registrations without the capability. This is a registration
sensitivity test, not an application-budget change or performance optimization.
These instrumented short-run footprints include the daemon/strace/timeout scope
but exclude the client and separately created backing fixture. They are not
whole-workload memory comparisons, long-lived plateaus or acceptance timings.

The upstream `notify_inval_inode` also passes in all three modes: a persistent
open fd sees changed cached bytes after `NOTIFY_INVAL_INODE` despite 500,000-second
name/attribute TTLs. In the pooled negative control, `--no-notify` leaves the same
bytes cached and the positive freshness assertion fails after four seconds;
re-enabling notifications passes again. This covers the legacy notification path
while regular requests use uring. Every mount unmounts normally, every daemon
exits zero, the original FUSE connection set is preserved, and no owned-scope OOM
or memory-limit event is observed.

The first classic attempt used `cache=never`, which makes this example return
`direct_io`; its unnegotiated shared-mmap attempt correctly returned ENODEV. That
failed fixture assumption remains recorded. All comparable positive controls use
the explicitly declared `cache=auto` configuration; no upstream/application source
was changed. This setup is functional but its 256-worker/2 GiB backing is not yet
a bounded low-memory transport suitable for the Rust acceptance experiment.

### Review of the io_uring decisions and tests

Sources checked for this review: Linux 7.3 `fs/fuse/dev_uring.c` (pinned copy in
`kernel73-research-v1`), libfuse `fe66006` `lib/fuse_uring.c`, libfuse issue 1281
and PRs 1177/1351/1522/1593, fuser issue 380 with the Skory design note and
fork, installed fuser 0.18.0 `session.rs`/`mnt/mount_options.rs`, and the 2023
LSFMM FUSE/io_uring report. Verdicts below separate what the tests prove from what
they do not.

What holds up:

- The transport proof is real, not inferred: `io_uring=on` in INIT, 159 `cqe unique`
  versus 5 `dev unique` requests in the basic log, 256 successful `io_uring_setup`
  calls and zero `io_uring=off:` fallbacks. Classic controls report `disabled` and
  create no rings.
- Negative controls exist and fail for the right reason: `cache=never`/ENODEV,
  `--no-notify` stale bytes after 4 s, and the no-`CAP_IPC_LOCK` run where 248 fixed
  registrations return ENOMEM while the filesystem keeps working.
- Teardown is clean in all eleven runs (normal unmount, exit 0, baseline FUSE
  connection set unchanged, no cgroup OOM/max events).

What does not hold up or is over-stated:

- The original `nr_cpus=4` rationale was wrong twice. The kernel routes by
  `task_cpu()` and marks the ring ready only when every `qid < nr_queues =
  num_possible_cpus()` has a registered queue (`fuse_uring_task_to_queue`,
  `is_ring_ready`); a request from a CPU without a queue fails with EINVAL. So
  queue count is a VM-definition property (256 possible vs 64 online is a
  hypervisor hot-plug setting), not a guest-kernel or cgroup knob.
- The 2,064 MiB RSS is a libfuse implementation choice, not a kernel requirement:
  `fuse_create_ring` allocates and populates `depth x payload_chunk` per queue.
  The Skory fuser design maps one `MAP_NORESERVE` region per ring and only pages
  touched by a request become resident; queues are shared across `n_threads`
  rings. The earlier text implying pools might shrink the backing was unfounded;
  pools only change ownership/registration, which the tests now show directly.
- The four-CPU "placement" result is a cpuset holding 256 threads on four CPUs,
  with 252 failed `sched_setaffinity` attempts. Any later timing on that layout
  would measure thread contention, not transport cost.
- The "readers on CPU 0/3/31/63" check proves bytes, not routing: no per-qid
  request counter exists. Kernel tracepoints or per-queue counters are needed
  before claiming cross-CPU queue coverage.
- The harness changed during the series (three `harness_sha256` values; the
  `IORING_REGISTER_BUFFERS|IORING_REGISTER_USE_REGISTERED_RING` flag parser was
  fixed mid-way). Receipts stay immutable; only `native-uring-verified-v1` is
  authoritative for registration counts. The on-disk harness is not byte-identical
  to any run.
- `strace -f` on a 258-thread daemon and 90 s/2 s run lengths make the recorded
  CPU and init times unusable as performance evidence; they were never meant as
  such, and the tables above are labelled accordingly.
- The 64 MiB child memlock is not a bound for a root daemon with `CAP_IPC_LOCK`.
  If the Rust daemon is ever deployed as root, memlock-based pool budgeting is
  ineffective and a cgroup memory ceiling is the only real limit.

Relevance to the acceptance gaps: published numbers for this transport (Skory
bench, in-memory filesystem, Linux 7.0, single NUMA node) are +10-25% on small
frequent requests (4 KiB reads/writes, stat, pread) and parity on 1 MiB streams.
Our random-read loss (1.73/1.79 vs 1.91/1.97 MiB/s) is about 450 4 KiB requests
per second against S3 round trips; the transport share there is negligible. The
transport can plausibly matter only for cache-hit metadata (22,398/20,239 ops/s)
and cached 4 KiB reads. The io_uring track is therefore a candidate for one gap,
not the acceptance path, and must be gated by a measured transport share.

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
- [x] Confirm the authorized corrective reboot and full CPU restoration from the
  automatic postboot receipt and independent live checks; verify perf record/decode.
- [x] Prove native classic/basic/pool initialization, bytes/mutation/mmap contracts,
  fixed-registration sensitivity, notifications and clean owned-resource teardown.

For each checkpoint delivery, append the private action/reason log, commit/push
the documentation, verify the existing draft PR and its footer link, and report
actual scope and remaining gaps in Tracker in Russian. Do not convert a
reboot-required marker or package/build readiness into runtime acceptance.

## Upstream-first review of all plans (2026-11-01)

Operator constraints added for this review: avoid forks of shared libraries and
prefer released upstream libraries even at a modest performance cost. The
[ticket](https://st.yandex-team.ru/MIRROR-188719) still contains an older Kubernetes
deployment plan. The operator subsequently clarified the primary target as **LXC
with nginx, rsync and golist**; containerd is not a blocker. A sustained real-data
run remains outstanding, with the primary deployment environment now LXC.
Plan A had started (gate A1 was being located); it is stopped and nothing from
it was changed. Upstream state was checked on 2026-11-01:

| Source | State | Consequence |
| --- | --- | --- |
| fuser [#380](https://github.com/cberner/fuser/issues/380) | Open; transport exists only as the Skory fork PR stack; upstream accepts PRs only from collaborators. Latest crate 0.18.0, no ring transport. | Fork excluded by the new constraint. |
| fuse-backend-rs [#247](https://github.com/cloud-hypervisor/fuse-backend-rs/pull/247) | Upstream (cloud-hypervisor, Apache-2.0 AND BSD-3-Clause) merged an experimental opt-in `fusedev-uring` transport on 2026-09-01; fixes #276/#280/#285 merged; 7.3 pools [#281](https://github.com/cloud-hypervisor/fuse-backend-rs/pull/281), zero-copy #282 and worker-failure #284 open. Released 0.14.0 (2026-02-26) predates it. | The only non-fork Rust path, but unreleased and a different filesystem API than fuser: adopting it replaces fuser and rewrites `src/fuse.rs`; notification support must be verified. |
| fractal-fuse 0.6.1, fuser-iouring 0.1.2 | Released, small; compio runtime with eager `MAP_POPULATE` payloads; prototype missing fsync/flush/release. | Not suitable as a fuser replacement. |
| libfuse | Released 3.18.3 predates bufpool PR1640; no maintained Rust binding to the ring loop. | C lab only; no Rust integration path. |
| Linux 7.3 | `nr_queues = num_possible_cpus()`, all qids required; patches for fewer queues could not be checked (lore behind bot protection). | Queue memory scales with node possible CPUs. |
| containerd [#9320](https://github.com/containerd/containerd/pull/9320), moby [#46762](https://github.com/moby/moby/pull/46762) | RuntimeDefault/default seccomp profiles block `io_uring_*` since 2023-11. | Relevant only to a future deployment using those runtimes, not the LXC target. LXC host kernel/FUSE and its actual confinement policy remain separate checks. |
| fuser 0.18.0 | `Config::n_threads`/`clone_fd` supported; callbacks take `&self`; `ReplyOpen::open_backing` passthrough and writeback capability exposed through safe APIs. | Plans B and D need no new dependency. |

Rechecked on 2026-11-02: crates.io still reports fuser 0.18.0 and fuse-backend-rs
0.14.0 as latest stable; fuser #380 remains open and #247 remains merged upstream.
There is still no fitting released non-fork Rust ring transport to adopt.

### Verdicts

- **A, FUSE-over-io_uring: stop integration; move to an upstream watch list.**
  No released non-fork Rust transport fits the current fuser stack. The best
  upstream candidate would replace the FUSE crate. Deployment needs verified
  LXC host-kernel/FUSE support and actual container policy. The benefit
  ceiling is cache-hit small requests (published +10-25% on in-memory
  filesystems), not the S3-bound random-read and writer gaps. Queue memory scales
  with node possible CPUs. Re-entry requires all of: a released crate with the
  transport and notifications (a future fuser or fuse-backend-rs release containing
  it; no future version number is assumed), verified LXC host/container policy, and a
  measured transport share >= 10% of daemon CPU on cache-hit workloads after B.
  The Skory fork is rejected under the new constraint. The native C results stay
  as evidence; no further lab work is planned.
- **B, classic transport through supported fuser APIs: completed below.**
  Additional receive loops retain allocation/rate objections; synchronous
  first-poll reads are rejected: a native contract shows a contended read stalling
  the sole receive thread. Retain the independent rename deadlock correction.
  Negotiated limits and the isolated cache-hit transport share are now recorded.
- **C, S3 request path: primary acceptance work.** Bounded prefetch for random
  reads, necessary versus avoidable writer RPCs, and owners of late memory growth.
  These are the measured gaps; they use the released SDK/Tokio and fit the
  no-fork rule as recorded in dependency decisions.
- **D, passthrough/writeback: remains deferred.** Supported by fuser 0.18.0 and
  upstream kernels, but changes disk-cache/coherence/durability contracts the
  operator deferred, and passthrough needs `CAP_SYS_ADMIN`. A VM with
  possible == online CPUs is no longer needed because A has no lab follow-up.
- **E, target-environment verification.** A sustained correctness-checked LXC
  run with nginx/rsync/golist, varied file sizes and listings, plus actual-service
  IAM/SSE checks. Use authorized read-only real data and disposable writes;
  VM and loopback results do not replace this gate. The older Kubernetes-only
  wording of ticket item 4 is superseded by the operator's deployment clarification.

### Order and gates

1. Keep the rename fix; calibrate identical-binary controls and reassess archived
   trials under the paired CI protocol, without reinstating runtime changes.
2. C attribution and corrections under the revised noise-aware rules.
3. E once authorized; record LXC host kernel, CPUs, confinement and FUSE settings,
   which also answers A's deployment precondition.
4. A only on all re-entry triggers. Upstream releases are re-checked at each
   checkpoint instead of building a local transport.

Resume the independent [remaining priorities](next-work.md) within C and E.
Neither a boot, standalone C mount nor a microbenchmark closes the complete
Rust/S3 acceptance gate or authorizes deployment.

### Completed Plan B checkpoint (2026-11-02)

[Attribution, complete checkpoint tables and limits](performance-classic-dispatch.md)
distinguish profiling from four-repetition unprofiled controls. Classic 1/2/4 loops
complete 864 mixed and 144 isolated phases; first-poll source pairs complete 96
isolated and 576 mixed phases. Every series verifies final payload generations,
bytes, topology, cleanup and adverse counters. The old joint-win predicate is
withdrawn as inference. Cached-read latency/CPU improve with first polling, but
joint effects need actual A/A calibration. No cache/TTL/durability or SDK policy
is weakened.

The rename regression hangs before removal of post-reply `inval_entry`, including
at one loop; kernel stacks and versioned Linux/libfuse sources confirm the parent
directory/notifier lock cycle. Keep VFS rename dentry handling and last-close inode
notification, without a new notification queue. Trial source/binary archives and
all original failures remain private; they are not the delivered runtime.

Captured INIT is 7.46→7.40, max_write 1 MiB, max_readahead 128 KiB, max_pages 256,
max_background 16. Application max_requests remains 64. The cached probe observes
26.99% inclusive transport CPU, but A still lacks a fitting released upstream
transport and verified LXC policy. Continue C's measured S3/setup and memory-owner
work, then E's actual LXC/nginx/rsync/golist and service contracts. Do not repeat B
or resume fork integration without new evidence and the stated upstream triggers.

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
`native-uring-verified-v1/report.json` derives strict verdicts and footprint rows
from the immutable `native-*-v1`/`native-notify-pool-v2` receipts. The bounded
`native_uring_smoke.py` and `verify_native_uring.py` use unmodified installed examples;
`postboot-perf-*-v1` records the 129-sample record/decode smoke. Initial trace-count
extraction missed the ORed registered-ring flag; the derived verification corrects
that parser against the unchanged syscall logs instead of overwriting receipts.
Raw sources, commands, failures and hashes remain there; they are not public PR attachments.
Use new output labels/directories: earlier receipts are immutable and helpers
often use exclusive creation. Do not rerun an old label over an existing result.

Before resuming, read this file, AGENTS.md and the project brief; inspect the live
branch/status and PR draft/auto-merge state. Use explicit finite timeouts and
noninteractive external operations. Check supported upstream APIs before another
workaround. Credentials and private operational identifiers stay out of commits.
