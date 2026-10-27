# Metadata coherence follow-up: replacement still UNACCEPTED

## Reproduced defect

A successful `lookup` HEAD or inode attribute refresh was rejected with EAGAIN
when an unrelated cached name disappeared remotely. Resolving that missing name
increments the global namespace epoch. The in-flight operation compared that
global counter even though its own name, inode and object generation were
unchanged. Two deterministic public-API tests failed with errno 11 on the prior
implementation in an isolated, secret-free Kubernetes build Pod.

This is a reproduced mechanism consistent with the failed real-object diagnostic,
not proof that it caused all six recorded EAGAIN failures. That diagnostic remains
invalid for comparative performance because the source objects changed.

## Ownership and publication

- Successful single-name metadata publication validates the current parent key,
  name binding, inode version and ETag, plus move/delete reservations and
  tombstones. An unrelated global epoch change no longer rejects a valid HEAD.
- A removed observed binding cannot be resurrected. A renamed/deleted parent
  cannot receive an old response. Cached references and owned dirty generations
  survive attribute refresh.
- Pending rename/backing-key ownership is rechecked after the network response.
  In particular, a cancelled move must still reject a different destination and
  remain retryable at its original destination. An intermediate implementation
  failed that behavior test before the ownership check was added.
- Dirty-descendant detection lives in one namespace helper and is checked both
  before and after an asynchronous directory probe. A late empty LIST cannot
  orphan a newly dirty child. A negative directory refresh also retains the epoch
  check when a newly created child was already flushed during the probe.
- Negative lookup and directory snapshot checks remain conservative. This change
  does not promise that every possible EAGAIN is eliminated.

No cache budget, TTL, retry policy, supported feature or durability requirement was
reduced. These are coherence repairs, not a measured throughput or memory win.

## Verification

Ten additional tests in `tests/review_regressions.rs` exercise public filesystem
operations, independent expected bytes and channel-controlled backend responses.
They cover new/cached positive lookup, getattr, parent rename/delete, removed
bindings, completed and cancelled deletes, active/cancelled moves, and dirty or
newly durable descendants during empty directory probes. Tests do not sleep to
construct the interleaving. The directory cases also failed on the intermediate
implementation before their publication guards were added.

With the locked public Cargo dependencies already cached:

```bash
timeout 150s cargo test --locked --offline --test review_regressions
timeout 300s cargo test --locked --offline --all-targets --all-features
timeout 300s cargo clippy --locked --offline --all-targets --all-features -- -D warnings
timeout 60s cargo fmt --all -- --check
timeout 60s python3 -m unittest discover -s bench -v
timeout 60s python3 -m unittest discover -s tests -p 'test_*.py' -v
timeout 180s python3 scripts/audit_public_sources.py
timeout 1800s cargo build --release --locked --offline
```

Final Kubernetes results: **157 Rust tests, 23 benchmark Python cases, five
source/oracle cases, rustfmt, warning-denying Clippy, a 336-package public-source
audit and the native release build passed**. Twelve independent guard-removal
faults each failed the intended behavior assertion with test exit 101; the
restored source then passed the full checks. Two selected lookup-guard mutants
from `mutator` are killed. Its initial surviving tombstone mutation prompted an
additional uncached-lookup-versus-unlink test; both results are retained.

`crapper --no-coverage src` and `dryer --threshold 0.8 src` also ran. Coverage and
CRAP are not measured. Lookup CC is 51, attribute-refresh CC is 47, and three
previous duplicate candidates remain. This is not a whole-package quality-gate
pass. The [whitelisted verification receipt](metadata-coherence-results-20261027.json)
binds source, tests and the retained native executable. Raw failed and successful
attempts remain private; no production credentials or objects were used.

The next acceptance work requires an immutable/version-pinned fixture, mounted
reproduction and equivalent repeated Go/Rust measurements with visible RSS, CPU,
cgroup/file/kernel memory, integrity checks and latency tails.
