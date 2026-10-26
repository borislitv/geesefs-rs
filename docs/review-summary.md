# Implementation review and verification follow-up

Four independent inspection lenses reviewed the initial complete implementation:
blind defects, edge cases, verification gaps and intent alignment. Findings were
checked against callers, public Go contracts and regressions before correction.
The first input had defects; this is not a clean-review claim about that input.

Code review complete: **0 decision-needed, 21 patch groups, 0 deferred,
1 rejected raw finding**. All patch groups below were addressed. The core details
and precise tests are in [review follow-up](review-follow-up.md).

| Group | Correction |
| --- | --- |
| F1 | Construct the shutdown deadline inside Tokio; synchronous fuser destroy now flushes without a reactor panic. |
| F2 | Compare inode identity/version/remote generation before installing a delayed HEAD. |
| F3 | Retain the known committed generation before fallible verification, without retiring dirty state early. |
| F4 | Retain and reconcile bounded pending DELETE ownership after cancellation or ambiguous results. |
| F5 | Reserve the removed directory prefix across the emptiness probe and deletion. |
| F6 | Recheck name-to-inode binding after an old unlink waiter acquires its gate. |
| F7 | Refresh expired clean getattr/fstat state through the core, preserving live identity and dirty data. |
| F8 | Drop expired clean directory ghosts while retaining dirty children and implicit ancestors. |
| F9 | Pass FUSE request UID/GID to creation when permission emulation is enabled. |
| F10 | Preserve S3 LastModified as the stable fallback for object timestamps. |
| F11 | Preserve HTTP object properties across ordinary and multipart metadata replacement. |
| F12 | Sign requester-pays headers on write, copy, part and delete operations. |
| F13 | Replace the periodic integrity fixture with position-distinguishing chunks. |
| F14 | Resolve standard-LIST unknown concrete types with bounded, globally admitted concurrent work. |
| F15 | Reject unsupported synthetic-root mutations without producing dirty empty-key objects. |
| F16 | Forward and record actual benchmark entry limits; reject wrong applied/effective budgets. |
| F17 | Warm inside the measured client cgroup; distinguish exact lifetime cgroup peak from sampled active RSS. |
| F18 | Propagate the overall benchmark deadline and recheck monotonic expiry between runs. |
| F19 | Reap owned test daemons even when ordinary unmount fails; use bounded waits. |
| F20 | Add an isolated correctness-failure evidence-gate case, independent of NaN/duplicate failures. |
| F21 | Exercise failed synchronous writes through actual mounted O_SYNC descriptors. |

## Refutations and evidence limits

- The reported create probe spanning a parent rename/delete was already rejected
  by the namespace epoch guard; its deterministic test passed before changes.
  Locked parent publication was additionally hardened. No failing reproduction
  of a narrower preemption window is claimed.
- Public Go applies request ownership but does not derive userspace setgid-parent
  inheritance. That suggested policy was not introduced.
- Go also emits regular-file hints without extended metadata and has the old
  requester-pays whitelist. These corrections improve the Rust behavior; they
  are not claims that Go already implements them.
- A mistargeted initial O_SYNC fixture did not inject the intended failure and
  was rejected. The corrected controlled `/fault` probe passes for both mounts.
  No defect in the existing write-flags branch is claimed; an unnecessary
  experimental open-handle map was removed.

## Failure sensitivity

The synchronous destroy test failed on the actual reactor panic before the fix.
Removing correctness rejection makes the isolated gate test fail while the old
combined test remains green. Restoring the old periodic data produces one digest
instead of six independently required positions. Core regressions and restored
mutations are documented with their exact boundaries in the linked follow-up and
[bounded metadata/pressure notes](bounded-metadata-pressure.md).

## Acceptance remains separate

Inspection fixes and green tests do not establish full mounted compatibility or
memory/throughput improvement. Required actual-service and complete CLI cases,
reference recovery failures, heap accounting and paired performance outcomes
remain explicit in [compatibility](compatibility.md) and
[performance](performance.md). No production acceptance is granted by this review.
