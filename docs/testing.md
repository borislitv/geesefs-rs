# Testing and current verification boundary

**Experimental foreground read/write filesystem; not accepted as a replacement.**
Use only disposable fixtures, never existing mounts or production credentials.

## Repeatable library checks

Install Rust 1.96.0 or newer with rustfmt/clippy and the Python dependencies from
`requirements-fixture.txt` in the selected environment. The verified toolchain is
Rust 1.99.1. Dependencies resolve from public crates.io with Cargo.lock; process
CPU accounting uses the public pinned psutil package.

The [sealed native read diagnostic](immutable-read-dataset.md) adds synthetic
immutable data, independently checked listings and native generation checks,
plus explicit metadata-roundtrip behavior tests. It preserves the failing plain
standard-listing Go workload and does not replace the complete acceptance matrix.

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
```

The 157 Rust integration tests cover actual loopback SDK HTTP requests and signed
extension transport, metadata/schema contracts, configuration, inode/namespace
state, dirty generations and concurrency. The performance-gate test invokes 23
Python evidence/summary tests. Five additional Python tests cover provenance and
the fixture oracle. No Rust tests are ignored. These are not a
claim of complete reference-test parity or production durability acceptance.

The latest full checks ran in an isolated Kubernetes build Pod without S3
credentials: formatting, all-target/all-feature tests and warning-denying Clippy,
23 benchmark Python cases, five source/oracle Python cases, and the 336-package
public-source audit passed. Directory/preload guard mutations and CPU
system-time/unit/identity mutations failed their expected assertions and were
restored. A live process/cgroup smoke check also observed positive CPU deltas.

The ub-mode follow-up checks adjacent, non-overlapping read windows through
`Filesystem::read` in both arrival orders. Both requests must reach the gated
backend before either completes, then return independently expected bytes.
`mutator` originally left both strict-boundary mutations alive; the new test
kills both. Separate runs retained the actual behavior assertion failures, not
compilation errors or timeouts of the mutation runner. The restored full suite
passes. See [the quality checkpoint](quality-results-20261027.json) and
[execution log](../_bmad-output/ub-mode-20261027.md) for the deliberately limited
mutation scope and the still-open complexity/coverage findings.

The [metadata coherence follow-up](metadata-coherence.md) adds ten public behavior
tests. Unrelated remote-name removal no longer invalidates successful lookup or
getattr; old responses cannot erase move/delete ownership or orphan dirty/newly
durable children. Twelve independent guard faults failed their actual assertions,
two selected tool mutants are killed, and the restored full checks and native
release build pass in the secret-free Kubernetes Pod. This is not a mounted
reproduction of all six historical EAGAIN failures or a measured performance win.

CPU windows retain user/system/total seconds for daemon and client independently,
plus cgroup usage and throttling counters. Reports distinguish wall time, occupied
cores and CPU seconds per GiB/operation. Missing historical CPU receipts stay
missing; successful older data checks cannot retroactively supply those counters.

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

- Formatting, warning-denying clippy and all 130 Rust tests (including the twelve
  Python evidence/summary cases): passed after scheduling, streaming, adaptive
  read-window and negative-cache changes.
  `cargo build` passed in the prior cancellation tranche; current full-target
  tests also build the CLI used by subprocess tests.
- Public source audit: 336 packages, zero non-public/path sources and no missing
  license declarations. This is not a complete binary license-notice audit.
- The initial benchmark preflight failed without delegated controllers. A dedicated
  user unit subsequently supplied owned CPU+memory delegation and ran a real paired
  smoke test. The subsequent full attempt timed out at 42/280; a post-adaptive-fix
  sequential diagnostic also timed out at five completed runs and retained the
  severe slowdown. All results and accounting limits are preserved in
  [performance](performance.md); neither attempt passes acceptance.

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
suite then had 123 passing cases, including the parent's lifecycle test.
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

The final adaptive-window follow-up adds three public read tests, bringing the
Rust suite to 126: contiguous 128 KiB requests including cache hits, promoted-window
cancellation/retry/owner budgets and true small seeks/count-one history. Original
code and partial repairs failed the independently expected range assertions;
a restored full-history-condition mutation failed the seek case. These fixtures
prove the corrected heuristic contract, not mounted speed: the subsequent bounded
measurement still showed the severe regression. The summary test also rejected
a deliberate mutation that counted failed runs as correct; the fault was restored.

The actual-FUSE overlap follow-up adds four public read regressions, bringing the
suite to 130: coalescing under large-window pressure with a cancelled follower,
publication after another inode replaces the cache, concurrent disjoint windows
and cancelled-owner follower recovery with exact owner charging. Original-code
and deliberate mutation failures were observed for all four scenarios; all faults
were restored. See [mechanism and invariants](read-fill-pressure.md).

Committed source 572195abfd37db14dc13b23b33708ddfdf4d662c then completed all 80
selected paired sequential/eviction results and one concurrent pair, all byte-correct.
The throughput collapse is resolved in those measurements, with lower mean RSS
and cgroup memory. P95 remains worse; full compatibility/performance gates remain
false. [Performance](performance.md) retains complete groups and both favorable
and unfavorable accounting/latency results rather than replacing earlier receipts.
