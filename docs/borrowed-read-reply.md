# Borrowed single-fragment read replies

Status: **unaccepted for production**. This checkpoint removes a redundant
application allocation/copy. Its functional checks do not establish a measured
whole-filesystem performance improvement.

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
inputs. A new matched repeat must retain both Go references and all adverse
metrics before reporting any speed, CPU, latency or RSS improvement.
