# Testing and current verification boundary

**Experimental foreground read/write filesystem; not accepted as a replacement.**
Use only disposable fixtures, never existing mounts or production credentials.

## Repeatable library checks

Install Rust 1.96.0 or newer with rustfmt/clippy. The locally verified toolchain
is Rust 1.99.1. Dependencies resolve from public crates.io with Cargo.lock.

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
```

The 123 Rust integration tests cover actual loopback SDK HTTP requests and signed
extension transport, metadata/schema contracts, configuration, inode/namespace
state, dirty generations and concurrency. The performance-gate test invokes eight
Python evidence-validation tests. No Rust tests are ignored. These are not a
claim of complete reference-test parity or production durability acceptance.

The storage fake preserves generation conditions, multipart state and byte
content. One-shot channels synchronize upload, multipart-part and LIST races;
tests do not use sleeps to invent an interleaving. Observable checks include
sparse/truncate bytes, fresh readers, failed upload/copy/completion, directory
rename retry, child commits during rename, stale LIST suppression, pinned-owner
charging, full dirty-budget progress and cancelled multipart cleanup. An upload
cleanup task retains its flusher admission token through abort; shutdown waits
for that bounded cleanup. Abort cannot guarantee remote cleanup if the backend
itself refuses abort, the upload ID was never received, or the process is killed.

## Failure sensitivity

Previously observed and restored targeted mutations:

- Removing GET If-Match fails the actual HTTP header assertion.
- Allowing AccessDenied as extension fallback fails exact-error classification.
- Replacing the Go JavaScript MIME default with `application/javascript` fails
  the independently expected `text/javascript` assertion; the mutation was restored.
- The PATCH complete-document regression first failed on a wrong-root success
  response being published. After reusing the bounded full-document validator,
  wrong roots, trailing documents and DTDs are all rejected.
- Raising the IAM response cap to admit an oversized token document fails the
  forbidden-S3-request assertion on a live listener; the cap was restored.

The new multipart cancellation regression was first run on the buggy code: it
failed with one orphaned upload after shutdown instead of zero. After the fix,
it passes and independently checks the complete retried object contents. Other
new tests were run, but no exhaustive mutation or all-regression sensitivity
coverage is claimed.

## Actual disposable mounted verification

Requirements: Linux, readable/writable `/dev/fuse`, `fusermount3`, `rsync`, Go,
Python 3.12+ and the dependencies in `requirements-test.txt`. Build Rust first;
provide a clean Go checkout at the plan's experimental revision.

```bash
python3 -m venv .venv-test
.venv-test/bin/pip install -r requirements-test.txt
cargo build --locked
.venv-test/bin/python scripts/integration.py --go-reference "$GEESEFS_GO_REFERENCE"
```

By default the runner owns a local Moto S3 service, a unique bucket, disposable
credentials and unique mounts. An external backend requires explicit
`GEESEFS_TEST_DISPOSABLE=1` and `GEESEFS_TEST_ACCESS_KEY_ID` /
`GEESEFS_TEST_SECRET_ACCESS_KEY`; ambient AWS credentials are removed. Never
point this runner at production. Cleanup is confined to owned resources. Logs,
partial checks, errors, traceback and provenance go in ignored
`benchmark-results/private/integration/`. The runner exits nonzero until the
complete compatibility matrix passes; `--rust-only` is diagnostic only.

Latest paired run (2026-10-26), using an actual built experimental Go executable:

- Both mounts: full checksum, unaligned Range/EOF, mmap, implicit directories,
  aiohttp full/Range HTTP, rsync full/delta, symlink, mode/mtime, user xattr,
  FIFO, root fsyncdir, sparse/truncate, fresh S3 durability, rename before close
  and concurrent writer/fsync checks completed.
- Rust: injected PUT failure was visible, dirty bytes remained readable, and
  retry after clearing the fault produced the independently expected S3 bytes.
- Go: after the same injected failure, open-file fsync continued returning
  EAGAIN through the 75-second recovery deadline. This remains a failed reference
  recovery check, not a passing comparison or permission to weaken expectations.
- Optional hardlink-as-symlink probe: both mounts returned EIO on this kernel.
  The failure is retained in each check list; no hardlink acceptance is claimed.
- Overall runner exit: **1**, compatibility **UNACCEPTED**. The subsequent
  backend-option tranche adds IAM/encryption/signing/PATCH transport loopback
  coverage, not a new passing mounted or actual-service run. The writer follow-up
  adds PATCH activation tests; actual-service/imported-layout and complete CLI
  coverage remain gaps.

The HTTP client is aiohttp, not a literal nginx deployment. No privileged
xfstests installer was invoked and no all-xfstests pass is claimed. CI checks
deterministic library/contract/gate tests only, not mounted acceptance.

## Local verification record

- Formatting, warning-denying clippy and all 123 Rust tests (including the eleven
  Python gate cases): passed after scheduling, streaming, read-window and
  negative-cache changes.
  `cargo build` passed in the prior cancellation tranche; current full-target
  tests also build the CLI used by subprocess tests.
- Public source audit: 336 packages, zero non-public/path sources and no missing
  license declarations. This is not a complete binary license-notice audit.
- The initial benchmark preflight failed without delegated controllers. A dedicated
  user unit subsequently supplied owned CPU+memory delegation and ran a real paired
  smoke test. Its unfavorable cold-metadata result and accounting limits are
  preserved in [performance](performance.md); this is not a full-matrix pass.

The 26 dedicated backend-option tests cover independently asserted real wire
headers/signatures, IAM cache/expiry, profile precedence, SSE-C copy phases,
bounded retries, prefix-safe cleanup, copy cancellation, PATCH ownership/errors
and full XML validation, plus strict/insecure self-signed TLS streaming. Config
subprocess tests now clear inherited environment, use five-second deadlines and
kill-on-drop. See [backend options](backend-options.md) for unverified paths.

The writer continuation adds 13 generation/conflict/PATCH/rename tests, two
large-budget regressions and four core-policy tests. The 32 GiB configuration
test allocates only small payloads. Regression tests failed before fixes for the
old u32 total-limit rejection, foreign same-size commit HEAD, child resurrection
after failed directory rename, repeated drain pages, lost DELETE response and
PATCH boundary crossing. Targeted mutations also failed for oversized-request
permit truncation, retiring newer writes and ignoring multipart cold-size policy;
all were restored before the final full rerun. These checks run with the parent's
Cargo unsafe-code forbid and retained safe rustix/jemalloc changes.

The scheduling/window continuation adds five schedule/streaming/parallelism tests,
three read-window/cache/cancellation tests, four lookup regressions and two real
SDK wire tests. It independently checks a virtual 60 GiB tiered layout without
allocating that object, a 100 MiB requested window fetched in five 20 MiB chunks,
cache retention beyond half the shared lane, and a 12 MiB streamed PUT with and
without CRC32 trailers. Expected source bytes and CRC32 are checked independently
of the body adapter. Sibling part jobs are aborted/drained on errors and through
owned cancellation cleanup before releasing the flusher token.

The stale-negative concurrent-create, remote-delete/recreate and dirty-markerless
parent tests failed before their fixes. Restoring the old 1 MiB window cap,
half-cache cap and fixed 50 GiB object rejection also produced targeted failures.
For the new wire tests, forcing serial backend copy failed because the second
part did not arrive before the first response; omitting the dirty overlay failed
the expected `end` payload assertion (zeros observed). All faults were restored,
then formatting, warning-denying clippy and the final full 102-test run passed.
No new mounted, real-service or comparative-performance run was performed.

The subsequent core/backend inspection follow-up adds 14 public-interface race,
refresh, timestamp, ownership and cancellation cases plus three wire cases.
See [per-candidate evidence and API handoff](review-follow-up.md), including the
already-guarded creation-probe scenario and explicit differences from Go's
non-directory dirent hint and requester-pays whitelist. Final full-suite count:
119 passing, none ignored. Parent-owned mounted/benchmark runs are separate.

The bounded resolver/pressure follow-up adds two cross-call metadata concurrency
and cleanup tests plus a deterministic cache-hit/FIFO pressure regression. The
current full suite has 123 passing cases, including the parent's lifecycle test.
See [measured baseline versus verified changes](bounded-metadata-pressure.md);
the supplied diagnostic pair is pre-change evidence, not proof of improvement.

## Parent boundary verification

The parent wired expired getattr refresh and request creation identity into FUSE,
and reproduced/fixed shutdown timer construction outside Tokio through the public
fuser destroy callback. Every package target forbids unsafe code. The public-origin
audit checks all 336 Cargo packages; five additional Python tests exercise private
source rejection, the forbid policy and position-distinguishing fixture data.

Fresh paired mounted run: both implementations completed full/Range/mmap, rsync,
metadata, sparse/truncate, rename, concurrent writer and injected O_SYNC failure
checks using position-distinguishing payloads. Rust also recovered from failed
explicit fsync with independently verified remote bytes. Go again remained EAGAIN
through the 75-second recovery deadline. Both hardlink-emulation probes returned
EIO. The runner correctly exited 1; no full mounted acceptance is claimed.

Overall runner deadlines, subprocess kill-on-drop/bounded waits, ordinary owned
unmount cleanup, isolated correctness-gate mutation sensitivity and public source
checks are recorded in [review summary](review-summary.md). The mistargeted first
O_SYNC probe did not inject its fault and was rejected rather than counted as a
regression; the corrected controlled suffix exercises actual failure.
