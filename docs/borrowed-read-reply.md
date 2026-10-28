# Borrowed single-fragment read replies

Status: **unaccepted for production**. This checkpoint removes a redundant
application allocation/copy. Its matched repeat does **not** demonstrate a
whole-filesystem improvement; adverse means and individual repetitions remain
retained below.

## Ownership contract

`Filesystem::read_reply_data` accepts a synchronous callback receiving
`Result<&[u8]>`. The FUSE adapter calls the existing `fuser::ReplyData::data`
within that callback. The callback cannot retain a `Block` or clone the backing
GET owner. This reuses the existing immutable `Block::slice`, Tokio admission
and fuser APIs; no buffer pool, protocol stack or stream replacement is added.

Miss admission still reserves the complete window and requested output before
backend work. Physical output allocation is deferred. When one delivered
fragment covers the complete requested reply, the borrowed callback receives a
slice of that fragment and the unnecessary output reservation is released.
The fragment retains its complete backing/GET charge. A reply spanning multiple
fragments still composes into admitted output before publication.

Owned `read` and `read_reply` miss responses retain independent output storage.
An initial attempt to return shared GET views to escaping owners failed the
existing concurrent write/fsync progress scenario: small old/new generation
replies pinned large GET owners, preventing admission of a following window.
That attempt was rejected and the original owned charge/progress assertions
were restored. Cache-hit view behavior is unchanged.

Generation checks, atomic aggregate admission, FIFO pressure reclamation,
checksum validation, bounded delivery, exactly-once publication, cancellation
and late body-error visibility remain required. This change does not adopt SDK
transport `Bytes` directly: charging retained transport backing and preserving
checksum/EOF semantics require separate evidence.

## Verification

All builds and checks ran in an isolated secret-free Kubernetes build Pod:

```sh
cargo clean --package geesefs-rs
cargo fmt --all -- --check
cargo test --locked --offline --all-targets --all-features
cargo clippy --locked --offline --all-targets --all-features -- -D warnings
python3 -m unittest discover -v -s tests -p 'test_*.py'
python3 -m unittest discover -v -s bench -p 'test_*.py'
python3 scripts/audit_public_sources.py
cargo build --release --locked --offline
```

- 181 Rust cases, 14 source/oracle Python cases and 23 benchmark Python cases
  pass; no Rust cases fail or are ignored. Formatting, warning-denying Clippy
  and the 336-package public-source audit pass.
- Before enabling reuse, the deliberately copied borrowed callback fails the
  retained-payload assertion: 2,228,224 bytes rather than the complete
  2,097,152-byte backing charge. The tests cover aligned/unaligned/EOF replies,
  real SDK body-prefix publication, cross-fragment composition, late body
  failure, cancellation and validation/empty replies.
- Ten independent faults fail their intended behavioral assertions, including
  copied borrowed output, wrong slice offset and escaping a large GET owner.
  Restored affected suites pass. Original writer/generation/pressure assertions
  are retained.
- Crapper reports read-inner CC 53, compared with 49 before this change; the
  borrowed callback adapter is CC 3. Dryer retains three previous duplication
  candidates at threshold 0.8. Coverage and CRAP are unavailable, not passing.
- Mutator scans 381 filesystem sites. The five selected mutations of the
  borrowing predicate are killed; 376 sites are unselected. This is not a
  whole-package mutation score.
- An independent receipt check reconciles the test counts, assertion/tool log
  hashes, 11 restored source hashes, all 25 compiled inputs and executable
  bytes. See [the quality receipt](borrowed-read-reply-quality-20261028.json).

The frozen cache-lookup baseline is
`3b94a76abc422f6e944c9e596fa971cf19b03858`, executable SHA256
`0e5a852d7fe4812eedb05cb11c74da67bdbe428764dbddd7f988daa91ddca07b`.
The new release executable SHA256 is
`b96c9427be58f5c6ad3f22920417aea697ab01cdc83b0a64d0e60cedfbe8a55f`;
its 25-input tree SHA256 is
`394be766410ccce70a3e29b80159ea8e896c95e913e81fe58cf606f9da53fff5`.
Only `src/budget.rs`, `src/filesystem.rs` and `src/fuse.rs` differ among compiled
inputs.

## Matched native comparison

Source `906eb45cef9a015c8725bda189b3431d7c6b7d7b` completed repetitions 50..54:
five rotated/reversed implementation orders, fresh and complete-dataset revisit
for the new executable, frozen cache prior and both unchanged Go references.
All 40 phases finish in 695.748 seconds with 120,000 full-byte/metadata checks,
zero file errors, unchanged native conditional hashes before/after and 97 exact
listings. All 460 indexed artifacts, 280 CPU windows and both Rust compiled-input
commits are independently verified. The complete
[numeric receipt](borrowed-read-reply-results-20261028.json) includes CPU splits,
allocator/client/cgroup/file/kernel/backend memory, p50/p95/p99 and dispersion.

The same sealed SeaweedFS 4.51 dataset has 3,000 files / 8,298,727,528 bytes.
Filesystem quota is 2 CPU / 4 GiB; backend quota is a separate 4 CPU / 6 GiB.
All implementations use a 512 MiB payload budget, 100,000 entries and four
128 KiB readers. Budgets, allocator defaults and metadata workload are unchanged.
Fresh means a new process/mount, not controlled cold host/backend caches.

Five-phase means; decimal MB. RSS is peak / mean of final fifth, not a stationary
plateau. Whole CPU includes daemon, clients and controller, including listing
and metadata work; payload throughput is measured after listing.

| Implementation/state | Read MB/s | Daemon RSS peak / steady MB | Whole cgroup CPU s | Read p99 ms | Listing s |
| --- | ---: | ---: | ---: | ---: | ---: |
| Go experimental fresh | 628.968 | 1289.231 / 1221.133 | 28.566 | 10.977 | 0.460 |
| Go experimental warm | 605.055 | 1292.544 / 1216.754 | 28.637 | 11.219 | 0.034 |
| Go packaged fresh | 624.295 | 1267.116 / 1206.325 | 28.754 | 10.974 | 0.461 |
| Go packaged warm | 596.628 | 1298.592 / 1218.926 | 28.971 | 11.365 | 0.037 |
| Rust borrowed fresh | 529.945 | 563.953 / 434.934 | 33.768 | 13.796 | 1.523 |
| Rust borrowed warm | 529.613 | 562.068 / 455.229 | 32.356 | 13.818 | 0.023 |
| Rust cache prior fresh | 534.541 | 560.189 / 433.569 | 33.364 | 13.079 | 1.478 |
| Rust cache prior warm | 532.183 | 560.906 / 453.227 | 32.416 | 13.912 | 0.024 |

Against cache prior, mean throughput decreases 0.86% fresh / 0.48% warm; whole CPU
increases 1.21% fresh and decreases only 0.19% warm. P99 worsens 5.48% fresh and
improves 0.68% warm. Peak RSS rises 0.67% / 0.21%; final-fifth RSS rises 0.31% /
0.44%. Throughput/CPU favor only two fresh and three warm pairs; p99 favors two
pairs in each state. Peak RSS favors one pair and final-fifth RSS two in each
state. These mixed directional observations are not significance or speed claims.

Both Go references still win throughput, whole CPU, p95/p99 and cold listing.
Lower Rust daemon RSS is not a whole-cgroup memory win under file-cache pressure.
Go heap time series and backend request metrics remain missing, not zero.
This repeat retains the prior failing plain standard-list/stat Go workload
separately; it does not replace that failure with a changed success criterion.

The useful local allocation change is insufficient to close the measured gap.
Next isolate the adapter's zero-initialization before fully overwriting each
fragment, using supported Tokio/standard buffer APIs rather than another buffer
implementation. The current repeat is a performance failure to retain, not a
reason to tune the comparison or infer a Rust/SDK/network ceiling. Full reader/
writer compatibility and representative workload acceptance remain incomplete.
