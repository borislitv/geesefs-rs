# Backend authentication and request options

These options have deterministic loopback HTTP/TLS contract coverage, not
production-service or complete mounted compatibility acceptance. Public upstream
reference and dependency provenance are in [dependency decisions](dependency-decisions.md).

| Area | Implemented behavior |
| --- | --- |
| Credentials | SDK credential providers; explicit `--profile` takes precedence over environment credentials; `--shared-config` selects custom profile files. Provider loading and requests have finite deadlines. |
| IAM | `--iam`, `--iam-header`, `--iam-flavor=gcp\|imdsv1`, `--iam-url`; token schema/expiry validation, single-flight refresh with wall-clock expiry, bounded responses, redacted errors, no redirects/proxy or fallback to AWS signatures. |
| Signatures | `--signature-version=auto\|4\|2\|anonymous`; SigV2 uses public `s3s-sigv2`, not a handwritten cryptographic signer. Extension LIST and PATCH use the same selected credentials/signature policy. |
| Discovery | Startup read-only bucket probe, region-header validation and configured-client rebuild; automatic anonymous selection only without an explicit profile/environment key; once-only SigV2 fallback on the reference probe statuses. Explicit auth, IAM and `--no-detect` do not silently downgrade. |
| TLS | Certificate verification by default. `--no-verify-ssl` explicitly disables S3 certificate verification and emits a warning; it does not weaken IAM metadata TLS. Both SDK and extension transport retain bounded streaming and raw stored-gzip bytes. |
| Object options | Requester-pays; SSE-S3, SSE-KMS (including empty key selection), SSE-C with validated 256-bit base64 key and MD5; source SSE-C on copy; ACL and storage-class headers are applied before signing. |
| MIME | `--use-content-type` follows Go's 16 defaults, first readable Unix `globs2` database or ordered `mime.types` overrides, exact-case then lower-case lookup and last-extension selection. `mime` parses syntax; charset parameters are omitted. Input is capped at 8 MiB/64 KiB per line. |
| Checksums | `--no-checksum` requests an unsigned payload and disables optional SDK checksums; mandatory protocol checksums are not suppressed. |
| Time/retries | Positive `--http-timeout` up to 24 hours; `--sdk-max-retries` (0–100) configures ordinary SDK and extension LIST attempts. Extension retries have a bounded aggregate deadline. PATCH is deliberately never blindly retried. |
| Startup cleanup | Prefix-confined, paginated stale multipart expiry using `--multipart-age` (default 48 hours); `--no-expire-multipart` really disables it. Invalid/repeated markers fail rather than looping. Startup reports cleanup errors. |
| Multipart clean copy | `--multipart-copy-threshold` (default 128 MiB), HEAD generation validation, conditional bounded copy parts, metadata/storage policy, completion ETag validation; failure or cancellation aborts a known owned upload. Shutdown drains cleanup admission. |

`credential_process` is intentionally unavailable: the evaluated public provider
executes a shell without a finite child-process timeout/kill-on-drop contract.
An explicitly selected profile depending on it fails; it is not executed or
accepted as a no-op. SSO/role/external-provider completeness is not established
by the dedicated static-profile test.

Cold-storage selection now knows PUT, clean-copy and multipart final snapshot
sizes. `Store::begin_upload(key, metadata, size)` selects STANDARD below the
configured STANDARD_IA cutoff, and STANDARD_IA at the exact boundary. The writer
passes final size, not current part length or a preferred-upload prefix length.
Reference SDK per-delay tuning flags and other missing reference CLI options
remain rejected rather than silently ignored.

## PATCH writer integration

The backend now exposes:

```rust
Store::patch(key, offset, body: Block, etag, append_part_size: Option<u64>)
    -> Result<PatchResult>
```

It owns the charged payload through one conditional request, validates offset
overflow and headers, sends `Content-Range: bytes start-end/*` and If-Match, and
returns the new ETag/LastModified only after complete bounded XML validation.
Only modeled NotImplemented maps to ENOSYS; access/conflict/server failures are
not hidden. A timeout or lost response may follow a remotely applied mutation:
the caller must not blindly repeat an append with a new generation.

`--enable-patch` now activates the writer. Nonempty small/simple and multipart
objects can be patched; empty objects and truncation/shrink-grow histories use
the ordinary upload path. Dirty ranges and growing zero tails are split into
bounded requests at configured part boundaries. Successful intermediate PATCHes
publish the new remote baseline without retiring local dirty generations;
only a completed snapshot retires its own extents. Metadata changes use a
conditional metadata-replacement COPY, with multipart copy for large objects.
PUT, complete-upload and COPY now return their known committed ETag; a later
same-size foreign HEAD cannot silently acknowledge our generation.

On an explicit conflict, fsync reports the error and refreshes the baseline for
a subsequent caller retry, retaining dirty bytes by default. Unmodified remote
bytes/metadata and remote growth are preserved. `--drop-patch-conflicts` instead
discards only overlapping extents from the failed snapshot, never newer writes.
This is an explicit data-discard policy. Both secondary PATCH flags require
`--enable-patch`. A lost response/cancellation retains the old generation: a
conditional retry can conflict and refresh, but cannot blindly duplicate an
append. Subsequent retries overwrite against the refreshed generation.

`--prefer-patch-uploads` creates a two-part prefix for new/fully truncated large
uploads and appends the tail with PATCH; an explicit unsupported response falls
back to a full upload. Unlike blindly shortening an existing object, this
restriction does not destroy an uncached existing source tail. Imported multipart
layouts remain unverified against the actual PATCH service.

## Configured scheduling and effective budgets

- `--single-part` is in MiB, defaults to 5 and clamps smaller values to 5;
  values above 5120 are rejected. Larger PUTs and upload parts use exact-size
  retryable Smithy bodies, materializing charged chunks of at most 1 MiB rather
  than allocating the configured part size. The SDK retains checksum ownership;
  real HTTP tests independently decode and verify CRC32 trailers and payloads.
- `--part-sizes` defaults to `5:1000,25:1000,125`. Each tier is `MiB[:count]`;
  an omitted/zero final count fills the remaining 10,000 parts. Explicit shorter
  schedules impose their own object bound. Sizes are 5..=5120 MiB. Writes and
  truncates exceeding the schedule fail with EFBIG before mutating local data.
  Tier boundaries, a virtual 60 GiB object and streamed large parts are tested;
  this is not an actual-service terabyte-object acceptance run.
- `--max-parallel-parts` (8) bounds uploaded parts per file;
  `--max-flushers` (16) bounds flush operations and globally admitted upload
  requests; `--max-parallel-copy` (16) bounds copy requests. Multipart backend
  COPY also uses bounded per-file parallelism and ordered completion. Admission
  has deadlines, and errors/cancellation abort and drain sibling jobs before
  owned-upload cleanup releases its permit.
- Read-ahead windows are no longer silently capped at 1 MiB. Requested windows
  are bounded by available configured owner capacity; `--read-ahead-parallel`
  (20480 KiB) controls chunk size and `--max-requests` bounds tasks per window.
  Aggregate output/scratch/reply ownership is reserved before fetching chunks.
  Read cache policy can retain the full shared data lane, not half of it;
  dirty, pinned and temporary owners still compete for that same lane.
- `Filesystem::effective_budgets()` exposes a serializable report of requested
  windows, capacity-limited window/chunk bounds, actual lane usage/capacities,
  cache policy, scheduling limits and object bounds. These are payload-owner
  budgets, not measured RSS or allocator/cgroup limits. FUSE request admission
  supplies the outer request bound; direct library calls are not globally
  limited by the read-window task setting. CLI receipt emission is separate.
- Unknown standard-LIST file types resolve concurrently through the same lookup
  checks, with a filesystem-wide `--max-requests` job bound acquired before each
  future is queued. Errors/cancellation release all borrowed jobs synchronously.
  Effective budgets report the resolver capacity and admitted job count. See the
  [change history](change-history.md).

## Core configuration and rename recovery

The implementation includes clean getattr refresh,
namespace/delete/commit race fixes, caller-ownership API, root attribute rejection,
safe standard-LIST type resolution, requester-pays on write/delete phases and
HTTP property preservation during conditional metadata replacement. These are
deterministic core/wire checks, not a new mounted or actual-service acceptance run.

- `--use-enomem` evicts reclaimable cache, then rejects pressure without flushing
  other dirty files; default admission flushes and waits with its existing deadline.
- `--no-implicit-dir` disables the fallback LIST probe; `--no-dir-object` skips
  marker HEADs and does not create markers. Empty markerless directories and their
  attributes are local until child objects make them discoverable.
- `--stat-cache-ttl` controls clean userspace metadata refresh (default one minute,
  zero forces refresh). Kernel/FUSE reply-cache policy remains a separate review.
- `--negative-lookup-cache-size` defaults to zero (disabled). A positive value
  bounds authoritative misses per directory, using metadata TTL and epoch-safe
  publication. Concurrent creation cannot be hidden by a stale miss; remotely
  deleted clean names can be recreated; dirty local children keep markerless
  parents visible across metadata refresh.
- A 32 GiB byte-budget capacity is admitted without preallocation. Individual
  reservations over u32::MAX are rejected before semaphore-count conversion.
- Pending rename prefixes remain bounded/indexed across errors, blocking child
  flush/create/delete transitions until retry. Repeated drain pages fail before
  repeating copies. A known copied backing key is retained before source DELETE,
  keeping reads and subsequent writes recoverable after a lost DELETE response.

Abort cannot reclaim an upload whose creation response never supplied its ID;
remote abort refusal, process kill and runtime teardown also remain limitations.
No actual IAM/KMS/SSE-C/PATCH service or new mounted acceptance run was performed
for these changes. Overall compatibility and comparative performance remain
**UNACCEPTED**.
