# Remaining work — 2026-11-03

Overall status is **UNACCEPTED**. Completed mechanisms belong in
[architecture](architecture.md) and [change history](change-history.md); frozen
measurements retain their own source and statistical scope. This page lists
actionable gaps rather than repeating their execution history.

## 1. Sustained read-only acceptance

Use Go 0.43.9 as the primary reference and the
[paired protocol](performance-methodology.md): at least eight independent mount
pairs, actual same-binary/configuration A/A controls, predeclared practical
non-inferiority margins and independent bytes/generations. Compare under equal
resources and the accepted negative-cache policy: 64 misses per directory with
one-minute stat TTL. Immediate external-create visibility beyond Go is not a gate.

The [controlled listing result](performance-listing-diagnosis.md) explains the
default-Rust loss through repeated absent-name probes. The
[loaded read-shape matrix](performance-read-shape.md) is a transformed local
workload; its real-S3 follow-up stops after five complete blocks on a dependency
timeout. Neither replaces sustained target-environment acceptance. Keep cumulative
native traffic budgets and original failed/partial attempts intact.

Profile random/concurrent reads, metadata and cache residency before another
runtime change. Remaining setup, scheduler/admission and reply costs have no
proven single SDK defect. Preserve checksums, If-Match, exact metadata, ownership
and durability. Higher equal CPU allocation is a sensitivity control, not a
source-code speedup. See [current accounting and scopes](performance.md).

Attribute the [scoped serving trial's](pr-context.md#scoped-serving-trial--2026-11-02)
`open`/`pread` `EAGAIN` and HTTP 500 before claiming equivalent correctness. Separate
admission pressure, metadata races and backend failures; retain failed responses
alongside successful-request timings and distinguish post-header read failures.
Include later `pread()` EIO and independently reproduced native backend read
failure. Compare vhost/method/URI and generations before attributing 404 changes;
current native/served HEAD parity is not historical full-payload acceptance.

The [HDD functional checkpoint](target-disk-cache-functional.md) adds a completed
hourly replay with five common-mount body truncations, despite successful explicit
byte/recovery controls. Native buffered-read/sendfile regressions now reproduce
immediate `spawn_traced` rejection as EIO and verify bounded receiver-thread
backpressure with unchanged work credits and a shared deadline. Qualify that
correction under authorized sustained target traffic; local fixtures do not close
the recorded serving failures. Investigate the three READDIRPLUS assertions that
also fail on the pristine baseline under the current development kernel; see
[the verification scope](testing.md#fuse-admission-backpressure--2026-11-12).
Preserve failures from other mounts/proxies separately, without changing runtime
limits or claiming whole-system acceptance.

Finish the separately authorized full configured L2 capacity/recycling trial with
concurrent complete `du -sh`, retaining traversal/read errors and daemon/client/
cgroup/kernel memory independently. Preallocation and cumulative writes alone do
not establish a full live cache or complete physical-pool turnover. Deploy the
[region observations and broader mirror policy](disk-cache-operations.md) only in
a separately recorded phase after the frozen workload; preserve inventory and
verify recovery/independent bytes. Streaming zstd is an inspection topic, not an
enabled codec or accepted memory/performance improvement.

## 2. Target access and large-namespace memory

Explicit `allow_other` wiring and the three local cross-UID cases are complete;
the owner-only default and kernel mode checks remain. Verify the actual target
service users, mount policies, profile and error paths separately. `dev`, `suid`
and Go's `--pprof` are still unsupported. The
[LXC preflight](performance-target-preflight.md) uses disposable fixtures and does
not establish full served-configuration compatibility.

Establish a realistic long-lived namespace/memory envelope. The observed loaded
Go reader footprint is anonymous; its absent profiling listener prevents live
heap-owner attribution. Payload budgets, soft Go limits and small short-window
RSS do not bound whole-process memory. Rust entry pressure can return `ENOMEM`
when every candidate remains referenced. Do not shrink caches, drop metadata or
force reclamation to manufacture a win.

For the serving trial, attribute namespace/cache occupancy, eviction and allocator
retention alongside anonymous memory over time. TTL refresh does not itself evict
the namespace. Slowing growth alone neither proves nor excludes a leak. The operator
separately authorized a larger 48-GiB total payload budget and 30-million-entry cap
for the three addressed readers; that choice does not establish a sustained envelope.

The [optional Prometheus listener](metrics.md) now provides cache/read, namespace
pressure and allocator/RSS observations and is deployed in the addressed trial.
Collect sustained series at the new limits; the preceding binaries had no counters,
so a retrospective unchanged-budget reuse comparison is unavailable.
Measure observer overhead separately and keep cgroup pressure and user-visible
errors/tails alongside cache reuse. This feature does not supply a heap profile.

## 3. Compatibility and writer behavior

- Complete actual-service IAM, SSE/signing/PATCH, imported multipart layouts and
  required Linux/CLI contracts. [Loopback coverage](backend-options.md) is not
  actual-service acceptance.
- Required Linux LINK remains unsupported without side effects. Returning
  `EOPNOTSUPP` safely is not positive hardlink compatibility.
- Preserve writer correctness while read-only optimization has priority. Attribute
  avoidable request/body work without dropping create validation, post-commit HEAD
  or retry/body ownership. [Owned-upload measurements](performance-owned-uploads.md)
  retain their short-control scope and remaining losses.
- Preserve observed Go open-handle fsync-recovery failures. Recovery after close
  is a different contract; increasing retries until it passes changes the probe.

## 4. Verification and statistical gaps

- Verify the changed public generic comparison runner end to end. Private native
  runners do not establish its orchestration coverage.
- Continue whole-file CRAP and broader mutation work. The new access wiring passes
  scoped CRAP and a meaningful omitted-ACL fault; zero automatically selected
  mutation sites are not a mutation score. Two percentile-rank mutations remain
  undetected. [Exact verification scopes](testing.md).
- Repeat observer-on/off and offered-rate controls where needed. Report daemon and
  client CPU/RSS, live allocator, cgroup/file/kernel memory and adverse interval
  counters independently; missing observations stay missing. A temporal late mean
  is not a memory plateau or zero observer overhead.

## 5. Deferred or suspended experiments

Lookup overlap and cached-observation reuse remain absent and statistically
inconclusive; the old strict four-pair predicate is withdrawn. Inline first polling
is rejected for receive-thread liveness. Additional loops have independent
allocation/rate objections. Do not reinstate those trials without new evidence.

[FUSE io_uring](fuse-io-uring-assessment.md) waits for a fitting released upstream
API, target-kernel/confinement support and a material measured transport share.
Passthrough and writeback remain deferred. The original disk-cache deferral was
superseded by separately authorized persistent-tier development and the withdrawn
HDD trial; its results do not close acceptance. A merge may retain the experimental
checkpoint; it does not accept production replacement.
