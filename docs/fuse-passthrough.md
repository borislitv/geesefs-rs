# FUSE passthrough: applicability to an S3 filesystem

## Result

The selected public `fuser` 0.18.0 already exposes safe passthrough APIs and a
passthrough example. A disposable capability-negotiation probe using that exact
crate on the measurement kernel `6.8.0-142-generic` reports
`passthrough_supported=false`, FUSE ABI 7.39. The result is an actual INIT
handshake, not an assumption based only on the kernel release string. The probe
then intentionally returns EINVAL and leaves no diagnostic mount behind.

The operator is willing to update the test kernel if needed. Keep passthrough
as a candidate for an already materialized local file cache; do not reject it
solely because the present kernel lacks the feature. No kernel update was
requested or performed at this checkpoint.

## Kernel and library contract

The [upstream kernel documentation](https://github.com/torvalds/linux/blob/master/Documentation/filesystems/fuse/fuse-passthrough.rst)
describes registering an underlying file descriptor with
`FUSE_DEV_IOC_BACKING_OPEN`, returning its backing ID with `FOPEN_PASSTHROUGH`
on OPEN/CREATE, and releasing the registration with BACKING_CLOSE. Supported
data operations can bypass userspace and access the backing file directly.
Upstream v6.9 contains `fs/fuse/passthrough.c`; v6.8 does not. Capability
negotiation, `CONFIG_FUSE_PASSTHROUGH`, permitted stack depth and the documented
CAP_SYS_ADMIN requirement matter in addition to the release number.

Inspected `fuser`'s installed public 0.18.0 source and example:

- `KernelConfig::add_capabilities(InitFlags::FUSE_PASSTHROUGH)`;
- `KernelConfig::set_max_stack_depth`;
- `ReplyOpen::open_backing` and `opened_passthrough`;
- `BackingId` registration lifetime and reuse for the same inode.

Use those APIs if a later prototype is justified. There is no reason to add an
application-owned ioctl/FUSE protocol implementation or local unsafe code.

## Fit and next experiment

An S3 HTTP stream is not a readable, seekable backing file at matching file
offsets. Passthrough therefore does not directly replace a cold conditional GET
or repair Moka/index overhead in the present bounded RAM-fragment cache.
Serving a sparse, incompletely filled backing file would return holes instead of
calling the daemon to fetch the missing S3 bytes. Do not enable it for such data.

For a ready local generation, bypassing repeated FUSE read requests could reduce
copies, daemon CPU and transitions. Evaluate it separately with:

1. A disposable node kernel that advertises the negotiated capability, with the
   existing default data path retained for unsupported kernels.
2. A fully validated local backing file keyed by immutable object generation;
   bounded file, FD, disk and page-cache ownership, including open-but-evicted
   files held by the kernel.
3. Explicit invalidation and open-handle behavior across remote replacement,
   rename, truncate and local writes. Data writes bypassing the daemon must not
   silently bypass dirty tracking, multipart commit or fsync durability.
4. Cold population cost and repeated-read CPU/p50/p95/p99/RSS plus whole-cgroup
   memory/file-cache/disk costs, against the ordinary path and both frozen Go
   references on the same updated kernel and cache budgets.

This is a research result, not a working S3 passthrough implementation or measured
speed claim. Do not update production nodes or served mounts for this experiment.
