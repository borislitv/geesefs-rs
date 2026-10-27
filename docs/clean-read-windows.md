# Single-buffer clean reads and aligned windows

## Diagnosis

The next bounded actual-mount timing trace separated SDK GET headers from body
collection. For a 256 MiB sequential read on the disposable standard-listing
backend, the preceding Rust implementation issued 26 GETs and the experimental
Go baseline 17. The backend typically spent about 100 ms preparing each Range
response; extra requests and refill boundaries therefore matter. These short
instrumented runs diagnose scheduling, not repeated comparative acceptance.

The clean read path also allocated a zeroed materialization buffer, then another
complete GET buffer, and copied between them even when there were no dirty bytes
or holes to overlay. It reserved both allocations up front, constraining a large
window to 24.5 MiB inside the unchanged 50 MiB shared data lane. With 20 MiB
parallel chunks, that repeatedly added a 4.5 MiB Range request.

## Change and invariant

A fully visible remote range with no overlapping dirty extent now returns the
backend's single charged allocation directly. The backend still validates its
conditional ETag, Content-Range, exact length, body completion and SDK checksums.
Dirty/sparse materialization retains its overlay path and double-buffer reserve.
No partial or unvalidated body is returned to the caller.

The effective clean-window bound is the data lane minus maximum reply space;
the separate overlay-window bound remains half that capacity. Budget-limited
large windows end on configured parallel-chunk boundaries when the caller's
range fits. EOF retains its exact tail. Neither configured cache/payload limits,
requested read-ahead nor parallelism are reduced. The same configured 100 MiB
window still uses five 20 MiB GETs when the unchanged memory limit can hold it.

Admission may wait while writers change the inode. Revalidate the acquired
snapshot before using the one-buffer allowance. If it now requires an overlay,
drop that snapshot and reservation, reduce to the explicitly reported overlay
bound and reacquire double-buffer admission. Never pin dirty extents while
waiting for their own permits.

## Regression evidence

- A nonconstant 64 MiB file, 128 KiB sequential reads and 64 MiB total memory
  requires the explicit backend ranges 0–20, 20–40, 40–60 and 60–64 MiB.
  Original code failed at the second range (4.5 MiB rather than 20 MiB).
- The same test asserts one 20 MiB allocation at each clean GET boundary.
  Restoring the redundant copy failed with 40 MiB instead of 20 MiB.
- While clean-read admission waits on an externally held reply, truncate a file
  to 2 MiB and regrow it to 8 MiB. The read must preserve base bytes, return zeros
  in the hole and keep scratch plus GET output within the 5 MiB data lane.
  Removing the post-admission recheck failed the allocation-bound assertion.
- Updated the pressure/ownership fixtures for the new allocation shape, retaining
  their actual race. Restored detached publication fails four versus three GETs;
  restoring cache borrowing before reply admission fails the FIFO progress deadline.

All mutations were restored. All 132 Rust tests, format and warning-denying
Clippy passed, including existing conditional/body-integrity, sparse, writer and
cancellation cases. Temporary timing instrumentation is removed. Fresh frozen-
source paired measurement is required; no new speed or latency acceptance is
inferred from these tests or scratch timings.
