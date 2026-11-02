# Read-only target preflight — 2026-11-02

**UNACCEPTED.** Read-only serving is the operator's first performance target;
writer correctness remains required. This checkpoint establishes environment and
compatibility facts, not an optimization or a representative performance win.
Host identities, operational traces and execution receipts remain private.

## Existing reader and available workload evidence

The authorized target is LXC on `5.15.179-15.2mds`, running GeeseFS 0.43.9. Its
shared reader is read-only with an 8 GiB buffer budget, two million metadata
entries, a 60-second stat TTL, `GOMAXPROCS=32` and `GOMEMLIMIT=16GiB`. The latter
is a Go soft target, not a process-RSS bound. The namespace exposes CPUs 0–55;
namespace-visible cgroup maxima do not establish the host-ancestor ceilings.

This reader is currently idle. One bounded nginx sample has 5,331 requests,
including 5,303 health checks and no requests in the shared-prefix cohort. All
2,000 retained 30-second GeeseFS samples spanning about 16.7 hours report zero
payload-read rate, with only 7,133 live inodes. The 67.7 MiB proc RSS snapshot is
therefore not evidence of loaded memory use, a plateau or a Rust/Go comparison.

Two bounded historical decoded log prefixes from October 10 and 11 contain
70,119 and 66,900 parsed requests; five and four lines respectively do not match
the HTTP parser. Whole-vhost `206` fractions are 11.27% and 9.71%, but only 9,488
requests in total belong to the later shared-prefix cohort, including 263 `206`
responses. These windows predate the current common-reader layout and routing.
Do not assign their whole-vhost mix or elapsed HTTP time to the current FUSE mount.

The log records request duration and bytes sent, but not exact Range offsets,
object generations, FUSE operations or S3 durations. Bytes sent include headers
and error/conditional responses. They cannot reconstruct a checksum-verified
filesystem replay, a cache working set or a daemon-latency distribution. Current
loaded-neighbor logs still require a live SSH authentication agent.

## Actual-kernel interoperability result

Current jemalloc release SHA256:

```text
8cd8d48f8a1546d54cf46e6e52252eaca10e6b4dac6317e37a65a5542a388c62
```

The runtime is the `4a06749` errno/fallocate refactor plus the retained rename
correction. No read fast path was added. Private receipts bind the executable,
source and transferred fixture code. The release passes available rw/ro client
and failure contracts on the development VM, then the following on the actual LXC kernel:

| Check | Buffered RO | Opt-in direct-I/O RO |
| --- | --- | --- |
| Full payload SHA256; four-client unaligned reads and EOF | Pass | Pass |
| Complete 32-entry implicit directory; mutation returns `EROFS` | Pass | Pass |
| Installed nginx 1.24.0 HEAD/full/Range/suffix/short-EOF/416 | Pass | Pass |
| Installed rsync 3.2.7 full copy and checksum-driven delta repair | Pass | Pass |
| Shared read-only mmap | Pass | `ENODEV` |
| Ordinary unmount and normal daemon exit | Pass | Pass |

The 16 MiB position-dependent payload is generated independently of filesystem
code. A one-byte backend corruption with the original expected digest produces
the expected SHA256 assertion failure; the unchanged fixture passes again.

These are two short mounts in an owned private mount namespace, with a two-CPU
quota and 1 GiB cap and 64 MiB/100,000-entry application budgets. Only disposable
test keys and a temporary SSH forward to a local fixture backend are used.
Private processes, mounts, forwarding and temporary files are cleaned up on both
success and expected failure. Readback preserves the served mount identities,
reader PID, executable hash, argv and configuration.

This validates the tested client binaries, kernel and confinement subset. It
does not validate golist, the served configuration, production S3, extended
metadata/auth options, sustained memory or comparative speed. Buffered and
direct-I/O policies must remain separately labelled, especially for mmap users.

## Next measurement boundary

Obtain a current loaded read trace with route attribution: filesystem-backed
payload, direct backend/origin responses and listing-service responses are
different actors. Establish byte/generation oracles, concurrency, working-set
size, actual FUSE activity and cache policy before paired measurements. Separate
resident reads, cache misses and metadata expiry; preserve freshness and budgets.
Use the [eight-mount A/A protocol](performance-methodology.md), keeping live
performance claims open until representative sustained evidence exists.
