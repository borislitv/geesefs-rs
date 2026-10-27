# Filesystem and measurement terms

- **Object generation:** a remote object's ETag and size, checked by conditional
  reads. An ETag alone is not assumed to be a full payload checksum.
- **Dirty version:** the inode-local version of pending writes or metadata.
  A read-cache generation combines that version with the remote ETag so data from
  different versions cannot share cached payloads.
- **Namespace epoch:** the local mutation counter protecting directory snapshots
  and negative namespace inferences from concurrent create, unlink and rename.
  Successful single-name metadata publication instead checks the affected binding
  and ownership. The epoch is distinct from payload version.
- **Read window:** a half-open byte interval `[start, end)` fetched for a caller
  and possible subsequent reads. Adjacent windows do not overlap.
- **Fill owner:** the generation-scoped owner of an in-flight read window. Its
  guard wakes overlapping followers on completion, failure or cancellation.
- **Payload budget:** the bounded ownership charge shared by cached, queued,
  in-flight and output payloads; retained cache alone is not total memory use.
- **CPU seconds:** differences between cumulative CPU counters at two boundaries.
  Process user/system CPU, whole-cgroup CPU and elapsed wall time are separate
  measurements. Adding process totals to cgroup totals would double-count work.
- **Fresh / warm revisit:** respectively a new mount/process and a second full
  dataset traversal. Neither label establishes controlled cold kernel or remote
  caches; see [the performance methodology](performance.md).
- **Sealed fixture:** disposable synthetic S3 data verified in full after removing
  its write identity and restarting the native server read-only. Its generations
  and independent payload/listing oracles are checked before and after workloads.
- **Explicit metadata roundtrip:** an additional workload reading stored xattrs,
  forcing attribute revalidation and checking ordinary stat plus payload bytes;
  it does not erase a failure of the original plain-list/stat workload.
