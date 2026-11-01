# FUSE io_uring assessment, including small-core configurations — 2026-11-01

Decision: worth a bounded experiment, especially for cache-hit metadata/small I/O
and concurrent request dispatch. It is not an established speedup for this S3
filesystem. The operator explicitly permits a small CPU allocation; large-core
defaults are not a reason to reject the design. No kernel, transport or runtime
change is made by this assessment.

Go-relative comparisons now require equivalent correctness, per the
[operator clarification](project-brief.md#acceptance-gates). The
[permission-expiry gap](metadata-expiry-diagnosis.md) is not an optimization target.
Random-read tails, unnecessary setup/probes, ownership/memory and uncovered
compatibility remain actionable independently of that gap.

## What Linux 7.3 changes

[Kernel.org](https://www.kernel.org/) identifies 7.3 as released on 2026-10-18 and
7.3.1 on 2026-10-30. FUSE-over-io_uring itself already exists in
[Linux 6.14](https://github.com/torvalds/linux/blob/v6.14/Documentation/filesystems/fuse-io-uring.rst),
including the current host's kernel generation. The important 7.3 additions are:

- [Buffer pools](https://github.com/torvalds/linux/commit/b45aaabc628bc7356e21bd2eb0c2ae9bdfa13894):
  decouple registered request entries from maximum-sized payload buffers. Kernel
  slots draw a buffer only while a request requiring a payload is active.
- [Registered pools](https://github.com/torvalds/linux/commit/96caf2496e15b3b12e1e4f3ac592648291333ecb):
  pin the pool's backing pages once instead of repeated per-request mapping work.
- [Per-open zero-copy](https://github.com/torvalds/linux/commit/43f8343858eb942d7f7c49964b31c54dcc314890):
  `FOPEN_IO_URING_ZERO_COPY` exposes page-backed READ/WRITE data through registered
  buffer indices and fixed-buffer operations. It requires `CAP_SYS_ADMIN`, a
  zero-copy queue and a buffer pool.

The [7.3 design](https://docs.kernel.org/filesystems/fuse/fuse-io-uring.html) and
[UAPI](https://docs.kernel.org/filesystems/fuse/uapi/fuse-uapi-io-uring.html) describe
per-CPU queues and combined commit/fetch commands. They also require handling
remaining control traffic through `/dev/fuse`; an io_uring-only happy-path loop is
insufficient. Notification support matters to our retained last-close invalidation.

FUSE transport can reduce device syscalls, handoff/queue contention and copies.
It does not remove S3 HEAD/LIST, SDK signing/decoding, remote latency, admission
rules or the durability barrier. Basic transport support is separate from 7.3
page-backed zero-copy: our AWS SDK/HTTP/TLS Bytes streams do not automatically
become fixed-buffer I/O into the client's pages. End-to-end zero-copy needs a
supported integration and independent checksum, generation and lifetime proofs.

## Our actual userspace support

Installed `fuser 0.18.0`, source `9c957f74efe715112049298cdf1d601781829c8d`, uses
blocking `read` and `writev` in `src/channel.rs`. The `FUSE_OVER_IO_URING` constant
is present, but there is no ring transport. Its capability setter checks only the
kernel's advertised bits; a constant is not implementation support. Later
[upstream PR719](https://github.com/cberner/fuser/pull/719) explicitly refuses the
unsupported capability. Do not conflate upstream master with the installed crate.
Our application requests READDIRPLUS, not io_uring, and `main::mount_options`
leaves the event-loop count at its default of one; callbacks already dispatch
bounded work to Tokio. Updating the kernel or linking libfuse does not replace
fuser's device loop.

Reuse candidates inspected before any custom transport:

| Candidate | Evidence and integration consequence |
| --- | --- |
| [fuser issue 380 / Skory stack](https://github.com/cberner/fuser/issues/380) | MIT, closest API-compatible transport proposal, with a [transport benchmark](https://github.com/Skory/fuser/pull/8) at `0104570c50c45fc2065b64226b67735d7b032b19`. Still a contributor fork/PR stack, not merged upstream support. Its reported benchmark gains are not measurements of our filesystem. |
| [fractal-fuse 0.6.1](https://github.com/fractalbits-labs/fractal-fuse/tree/d01dafc06a2ccd10701e62a19370b8d4c5e901b7) | Apache-2.0, actual io_uring transport; `with_worker_count` folds all qids over fewer compio workers. Different filesystem/runtime API from fuser/Tokio. Inspected transport still uses per-entry `MAP_POPULATE` payload buffers, not the new 7.3 pool commands. README buffer-registration wording does not prove 7.3 client-page zero-copy. |
| [fuser-iouring](https://github.com/sdslabs/fuser-iouring/tree/d136eabee46e243fd62e41d4cb48ed2f5581df22) | Apache-2.0 prototype; the inspected public trait lacks our required fsync/flush/release contracts. Not a drop-in reader/writer replacement. |
| [libfuse](https://github.com/libfuse/libfuse/releases/tag/fuse-3.18.3) | Established C transport; published 3.18.3 predates the [bufpool merge PR1640](https://github.com/libfuse/libfuse/pull/1640), merged 2026-09-29 after superseding unmerged PR1593. Reusing newer upstream is preferable to a new protocol implementation, but this requires a different binding/adapter; merely enabling fuser's libfuse mount feature does not use that event loop. |

Keep local `unsafe_code = "forbid"`; a maintained dependency's safe public API is
the reuse boundary. No candidate has yet passed our full owned-reply, write,
cancel/unmount, mmap, READDIRPLUS reference and notification contracts.

## Small-core design: a useful option, not a verdict

There are three distinct counts: possible CPUs in the kernel, allowed CPUs/quota,
and daemon workers. Linux 7.3
[`fuse_uring_create`](https://github.com/torvalds/linux/blob/v7.3/fs/fuse/dev_uring.c)
sizes queues using
`num_possible_cpus()` and routes by the caller's CPU. Quota/cpuset alone does not
shrink that possible-CPU count. The readiness check requires registration across
the queues; restricting clients does not justify leaving required queues unserved.
Several qids can be driven by one worker, as fractal-fuse demonstrates, so one
worker per possible CPU is not a fundamental requirement. Affinity must also
respect the actual allowed CPU set.

For a VM exposing 2 or 4 possible vCPUs, shallow queues are a credible initial
configuration. Assuming a negotiated maximum payload of 1 MiB, matching the
current I/O limit, old-style payload allocation is
`possible CPUs × entries per queue × 1 MiB`, excluding page-aligned headers,
ring/task/kernel state, workers and the filesystem's existing payload cache.
Verify `max(max_write, max_pages × PAGE_SIZE)` rather than the app limit alone:

| Possible vCPUs | Entries per queue | Nominal payload allocation MiB |
| ---: | ---: | ---: |
| 2 | 4 | 8 |
| 2 | 8 | 16 |
| 4 | 8 | 32 |
| 4 | 256 | 1024 |
| 256 | 8 | 2048 |

These are source-derived capacity estimates, not measured RSS. Inspected
fractal-fuse defaults to depth 256 and eagerly maps/populates each payload; small
worker count alone does not remove that allocation. Reducing actual possible
vCPUs and selecting shallow depth makes a material difference. At 7.3, separate
per-queue pools instead bound payload space by `sum(pool sizes)`, independently
of the number of parked entries. With two 1 MiB payload buffers per queue, 2/4
vCPUs nominally need 4/8 MiB of pool payload space; this is a sizing example, not
a demonstrated sufficient concurrency setting. Pool starvation/backpressure and
metadata progressing alongside data I/O still need testing.

The present VM has kernel `7.0.0-38-generic`, possible CPUs `0-255`,
`fuse.enable_uring=N` and `kernel.io_uring_disabled=0`. It is not a small-possible-
CPU 7.3 test environment. No module parameter, host CPU setting or kernel was
changed, and no 7.3 A/B result is claimed.

## Expected benefit and useful comparison

- **Most plausible benefit:** cache-hit small reads and metadata with many clients,
  where kernel/daemon handoff and the current single receive loop can matter.
  Check the existing fuser `Config::n_threads` / `clone_fd` alternative as a control
  before crediting all improvement to a new transport.
- **Limited by other work:** expired metadata and cache misses still resolve exact
  user metadata and use S3. The current expiry profile has 67.07 ms SDK category,
  51.05 ms kernel and 29.03 ms unresolved out of 207.21 ms sampled period. Kernel
  includes network and scheduling work, not only FUSE; these are not additive
  wall-time components or a predicted io_uring speedup.
- **Writer uncertainty:** batchable FUSE handoff may help small writes, but cannot
  remove our repeated namespace-validation actors or remote fsync work. First
  distinguish necessary checks from avoidable work under the clarified contract.
- **Memory opportunity and cost:** pools avoid allocating a maximum payload for
  every parked entry, but ring/pinned buffers are additional owners. Charge their
  full backing/lifetime and measure RSS/cgroup/kernel memory, not only the 64 MiB
  application cache budget. Fewer cores can trade CPU cost against saturation
  and queue tails; it is an experiment axis, not an assumed improvement.

The appropriate experiment uses a dedicated 7.3.x guest with **verified** 2 and 4
possible vCPUs, then 1/2 transport workers and depths 4/8. Hold application/Tokio
workers and payload budgets constant between transports. On the same kernel and CPU budget,
compare existing fuser one-loop and bounded multi-loop controls against a reused
io_uring transport; separate basic transport, buffer pools and zero-copy. First
prove full bytes, fresh modes, generations, fsync failure/recovery, cancellation,
FORGET/PLUS ownership and notification/shutdown behavior. Then rotate repeated
cache-hit metadata/4 KiB reads, expired metadata, S3 misses and writer/fsync runs,
reporting throughput, pooled tails, CPU/work, all memory and pressure counters.
Select a transport only for a reproducible correctness-equivalent improvement.

Versioned public sources and local-configuration receipts are retained in ignored
`benchmark-results/private/vm-20261031-integrated/kernel73-{research,reuse}-v1`.
This assessment establishes feasibility and constraints, not integration or a
measured performance result.
