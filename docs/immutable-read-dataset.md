# Sealed native S3 read diagnostic

This diagnostic separates a reproducible, synthetic S3 dataset from mutable
service data. It is an additional workload, not the complete compatibility or
performance acceptance matrix. Production acceptance remains **false**.

## Backend and oracle

`bench/sealed_s3_fixture.py` reuses SeaweedFS 4.51 `weed mini` for S3 and storage,
and boto3 for protocol requests. See the pinned public origin, license and archive
hash in [dependency decisions](dependency-decisions.md).

- Supply a JSON array of exactly 3,000 positive sizes, each at most 128 MiB,
  totaling at most 16 GiB. Only sizes are inputs; names and payloads are synthetic.
- The controller generates deterministic, position-distinct SHAKE-256 payloads,
  independently hashes them, and saves the returned ETags. Metadata rotates four
  modes and distinct mtimes, with explicit uid/gid. One directory has 1,400 files;
  the remaining files occupy 95 other directories.
- After seeding, the controller stops the server, removes the seed identity and
  restarts the same disposable persistent storage with only Read/List permission.
  Public dummy fixture identities are never production credentials.
- PUT, DELETE and multipart initiation by the reader, and PUT by the removed
  seeder, must each return 403. An incorrect If-Match must return 412.
- Every object is read conditionally and checked for complete SHA-256, size and
  metadata. Exact paginated listings are checked against names derived from the
  seed blueprint, not against another possibly incomplete listing response.

The controller publishes `manifest.json`, `listings.json`, `backend.pid` and
`sealed.json` into the shared directory only after these checks. It stays alive
until an explicit `finished` file or its finite timeout. Keep the backend available
for follow-up diagnostics until evidence is collected.

## Runner environment

The current runner targets an owned Linux two-container test Pod with a shared
network and PID namespace. `/work` is private to each container; `/shared` is a
shared volume. The backend listens only on loopback. No application credentials
or service-account token are needed.

- Filesystem container: `/dev/fuse`, ordinary `umount`, `fusermount3`, GNU `stat`
  (`gnustat` is selected when installed), Python 3.12+ and
  `requirements-fixture.txt`; 2 CPU and 4 GiB memory limit.
- Backend container: separately accounted CPU/memory cgroup, the pinned native
  server and boto3; 4 CPU and 6 GiB memory limit; sufficient disposable disk.
- Supply `geesefs-rs`, `geesefs-rs-pre-coherence`, `geesefs-go-experimental` and
  `geesefs-go-packaged` in the filesystem `/work`, with exact source/executable
  SHA-256 values in `provenance.json`. The runner verifies executable hashes.
- Mounts use the same read-only identity, standard V2 listing, 512 MiB payload
  budget, 100,000 entries, enabled permission/mtime metadata, and four 128 KiB
  readers. Startup upload expiry and backend detection are disabled for both.

Run the backend controller and, after sealing, `bench/read_dataset.py` in the
filesystem container. All child processes and the whole experiment have finite
deadlines. Normal owned unmounts are required; a remaining mount aborts the run.
All failed phases remain in the report. The report deliberately never grants
whole-project acceptance.

## Distinct metadata workloads

The original plain workload lists directories, performs ordinary stat and reads
all file bytes. On this standard-listing backend, both exact Go references return
default ownership/mode and object LastModified instead of stored metadata. Their
public CLI documents that enabled permissions/mtime require metadata in listings.
This is retained as a correctness failure; early exits are not throughput wins.

The additional explicit-metadata workload reads and checks all four user xattrs
for every file, calls GNU `stat --cached=never` to revalidate attributes, and then
performs ordinary stat plus complete payload reads. Every implementation executes
the same additional operations and retains its original cache budgets and TTL.
The metadata work's wall time and CPU are reported separately and included in
whole-workload CPU/time. This does not turn the plain workload into a pass and
does not establish extended-listing parity.

## Accounting and verification

Five implementation orders each run a fresh mount/process followed by a complete
warm revisit. Host/backend caches are neither controlled nor globally dropped;
the dataset exceeds the daemon cache. Fresh is not a claim of cold hardware.

Raw artifacts include each file result, exact listing failures, process identity
and CPU boundaries, user/system/cgroup CPU and throttling, 50 ms target RSS/cgroup
memory samples and approximately one-second PSS samples. The sampler reports
backend and controller separately. Peak RSS is an observed per-phase maximum;
steady RSS is the mean of available samples in the final fifth, not a stationary
plateau. A combined daemon/client RSS maximum is computed pointwise, not by
adding unrelated maxima. Shared RSS pages may be counted twice. Cgroup lifetime
peaks are explicitly distinct from sampled phase peaks. GNU stat's waited-child
maximum RSS is separately retained; cgroup memory includes that child.

Rust allocator samples remain raw. Missing Go heap peak/steady time series and
unavailable backend request metrics must be labeled missing, never zero. Final
native full-byte/generation checks and exact listings are mandatory after all
phases. A valid additional read diagnostic is still not a performance-gate pass.

Run the focused oracle tests and failure-sensitivity check in the chosen test
environment:

```bash
timeout 120s python3 -m pip install -r requirements-fixture.txt
timeout 90s python3 -m unittest discover -v -s tests -p 'test_*.py'
timeout 90s python3 -m unittest discover -v -s bench -p 'test_*.py'
timeout 180s python3 bench/check_sealed_oracle_faults.py --output oracle-fault-receipt
```

The targeted checker runs baseline, ten independent guard faults, and restored
tests on temporary source copies. A setup/import error or timeout does not count
as an assertion-level kill. It does not claim whole-program mutation coverage.
