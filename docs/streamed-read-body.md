# Incremental conditional-GET publication

## Contract and implementation

The [requested-range callback](requested-read-reply.md) removes the unrelated
parallel-window barrier, but the matched repeat still loses to Go. Public Go
`sendRead` separately publishes completed 128 KiB body buffers before a configured
GET finishes. The previous Rust backend returned only complete GET blocks.

The SDK backend now delivers complete immutable 128 KiB fragments through a
bounded Tokio channel to the existing admitted window owner. Conditional GET
ranges, If-Match, request concurrency, payload/cache budgets and default allocator
are unchanged. There is no duplicate small GET, detached producer or new HTTP/S3
implementation. The SDK ByteStream/Tokio AsyncRead adapter retains body framing,
errors and cancellation; the selected public dependencies remain in use.

Each fragment retains a shared reservation for its **complete admitted GET**.
Producer, queued fragments, cache and last borrower cannot independently release
that charge. Each physical fragment also retains its local allocation reservation.
This is bounded application-payload accounting, not a claim that SDK buffers,
metadata, allocator overhead or process RSS fit the configured payload limit.

The window owner drains queued fragments before observing producer completion.
Requested bytes can be published once complete while the same bounded FUSE task
continues owning the body. A subsequent body failure remains an observable tail
error; a requested dependency failure before publication remains a reply error.
Cancellation drops the channel and aborts the existing owned child task set.

Exact Content-Range and Content-Length are validated before delivery. Incomplete
fragments are never published, and the final fragment requires complete body EOF.
If the response advertises a whole-body checksum, delivery retains the previous
complete-body SDK validation barrier instead of exposing a prefix that could
subsequently fail that check. Whole-range reads and writer materialization keep
their complete-validation API.

One-block cache hits still return immutable charged views. An unaligned request
spanning adjacent fragments uses its already admitted output reservation to join
only bytes from the same inode/generation. Gaps remain misses. Admission and
pressure reclamation still happen before borrowing cache allocations, preserving
the existing FIFO progress regression.

## Behavioral evidence

Eight additional cases in `tests/backend_contract.rs` exercise actual SDK HTTP
and Filesystem/S3 wiring:

- Complete 128 KiB bytes arrive while the same conditional GET's tail is gated.
- Queued data retains full admission after its producer has finished.
- Invalid ranges/checksums expose no fragment; advertised valid checksums wait
  for complete SDK validation.
- A following unaligned cached span succeeds after its server has exited, and a
  newer dirty write cannot reuse the old generation's fragments.
- Short bodies before/after a complete prefix reject without publishing partial
  data or retaining unpublished allocation charge.
- Closing delivery cancels the producer while a retained prefix remains valid
  and charged until its last owner drops.
- Filesystem tail failure and cancellation preserve successful prefix bytes,
  observable errors and exactly the remaining cache/reply ownership.

The delegation-only backend first failed prefix timing and transferred-charge
assertions. The complete-range filesystem path then failed the real HTTP prefix
assertion. After connecting incremental delivery, the following unaligned cached
span failed until the cache could compose fragments. Raw red/intermediate/green
receipts are retained, including an initially unnamed timeout diagnostic.

Fixture corrections remain separate from production regressions: tiny_http's
automatic chunked framing initially omitted the required Content-Length; default
extended listing issued LIST instead of the fixture's expected HEAD; and a
deliberately short keep-alive response initially stalled instead of ending at
EOF. The final fixture uses explicit lengths, standard listing and the SDK's
test-only `Connection: close` header. No byte/range/checksum assertion was removed.

The isolated secret-free Kubernetes checks pass with 178 Rust cases, 14 Python
source/oracle cases, 23 benchmark Python cases, formatting, warning-denying
all-target/all-feature Clippy and the 336-package public-source audit. Existing
writer, generation, FIFO-pressure and concurrent-window cases remain enabled.

Seven independent faults fail their intended behavior assertions: deferred body
publication, missing complete-GET ownership, omitted cached-span composition,
premature checksum publication, ignored short reads, hidden late body errors and
lost fragment offsets. The restored 56-case affected suite passes. One selected
tool equality mutation in cached-fragment completion is killed; the other scanned
sites are not claimed as executed. Mutation workers and final verification clean
the owned package to avoid the previously reproduced stale-mtime artifact trap.

The [whitelisted quality receipt](streamed-read-body-quality-20261027.json)
binds source/fault/tool hashes to those checks. After behavioral green, the new
CC-18 body function was split into admission/checksum selection, delivery and EOF
validation, reported as CC 7, 9 and 3. Cached-fragment composition is CC 6; the
larger filesystem read-inner remains CC 49. Three prior duplication candidates
remain. Coverage and CRAP have not been measured; a selected mutation score does
not establish whole-package quality or filesystem acceptance.

## Measurement boundary

No speed, CPU, RSS or whole-memory improvement follows from the gated tests.
Freeze and hash the candidate, retain the exact requested-reply prior and both
public Go executables, then repeat the unchanged sealed native dataset with five
interleaved orders, full bytes/metadata and native generations/listings before and
after. Publish CPU with peak/final-fifth RSS, allocator, client/cgroup/backend scope
and adverse repetitions. Compatibility and overall performance remain
**UNACCEPTED**.
