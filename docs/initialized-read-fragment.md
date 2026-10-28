# Initialize streamed fragments through maintained buffer APIs

Status: **unaccepted for production**. The preceding
[borrowed-reply repeat](borrowed-read-reply.md) removed an allocation but did not
improve whole-filesystem speed/RSS. Its adverse results remain retained.

## Narrow change

The streaming adapter previously allocated a zero-filled fragment and immediately
overwrote every byte through `read_exact`. Tokio 1.54.1 already supplies the
needed mechanism: `copy_buf` consumes the SDK reader's AsyncBufRead buffer with
no extra intermediate allocation, `take` bounds consumption to the admitted
fragment length, and `AsyncWrite for Vec<u8>` extends initialized bytes without
pre-zeroing the unused capacity. This is supported library behavior, not a local
unsafe/uninitialized-buffer implementation.

`Reservation::read_exact` preallocates only the reserved capacity, invokes those
APIs and checks the copied count before moving the Vec into the existing charged
Block. Short reads/errors drop the private partial allocation and reservation;
cancellation drops both through the same future owner. The fragment limit keeps
the vector within the admitted size and does not consume the following fragment.
No global pool, detached producer, new dependency or budget/concurrency tuning is
introduced. Only `src/budget.rs` and `src/backend/s3.rs` change among compiled
inputs relative to the exact borrowed-reply release.

The existing final EOF check still runs before the last fragment is delivered.
Advertised whole-body checksums still take the complete-validation path and do
not publish early. Their existing materialization path remains unchanged.

## Verification before measurement

- All 181 Rust cases, 14 source/oracle Python cases and 23 benchmark Python cases
  pass in isolated secret-free Kubernetes. Formatting, all-target/all-feature
  warning-denying Clippy, the 336-package public-source audit and native release
  pass after explicit package cleanup.
- Existing real SDK/HTTP tests cover prefix timing, unaligned/EOF bytes, short/
  excess bodies, checksum barriers, exact charges, cancellation and late errors.
  Original writer/generation/FIFO ownership tests remain unchanged. The real
  matched performance failure is the optimization's red; no artificial private
  initialization counter or flaky timing assertion is added.
- Eleven independent faults fail the relevant public behavioral checks, including
  suppressed IO errors and consuming past the admitted fragment. Restored suites
  pass. The initial copied-count guard mutant survives because the real SDK
  rejects truncated HTTP first; that observation is retained. The revised fault
  suppresses the reachable SDK IO error instead of inventing an impossible mock.
- Crapper reports the new admitted fill at CC 3 and read-inner at CC 53. Dryer
  retains three previous candidates. Coverage/CRAP remain unavailable. Mutator
  scans 18 budget sites and kills the one selected count-comparison mutation;
  17 sites are unselected, not covered by that score.
- Independent receipt reconciliation verifies test counts, fault/tool logs,
  11 restored source hashes, 25 compiled inputs and exact executable bytes. An
  initial proof expected CC 2 and failed; the measured CC 3 was retained and the
  corrected receipt verification passes without changing production source.

See [the quality receipt](initialized-read-fragment-quality-20261028.json).
Reproduction uses the commands in
[the previous checkpoint](borrowed-read-reply.md#verification), on the exact
source revision recorded with the forthcoming matched results.

Baseline source: `906eb45cef9a015c8725bda189b3431d7c6b7d7b`, executable SHA256
`b96c9427be58f5c6ad3f22920417aea697ab01cdc83b0a64d0e60cedfbe8a55f`.
Candidate executable SHA256:
`20d450570792eb21a6090e1399d429472fac8d1648ae1fcc52bb95dda27c40e7`.
Candidate 25-input tree SHA256:
`42fd17aa65700cde56239094b8df7c4275a44a2d26263e6b5daaffaedf142472`.
No comparative speed, CPU, tail or RSS improvement is claimed before the exact
matched repeat with both Go references and all adverse metrics retained.
