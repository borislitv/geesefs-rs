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

## Matched native comparison

Frozen source `20a9dcd2f125e30f251e02510fe3f44501a290f6` completed forty phases
in 777.999 seconds against exact prior `d17a5a6` and both Go references. All
120,000 complete file hashes/metadata checks pass. Native conditional hashes for
3,000 objects and all 97 exact listings remain unchanged before/after. All 280 CPU
windows and raw memory summaries were recomputed, all 460 indexed artifacts were
checked after collection, and both Rust compiled-input sets match their commits.
The candidate differs from its prior in the six intended build/source files.

Same 8,298,727,528 synthetic bytes, 2 CPU/4 GiB filesystem quota, separate
4 CPU/6 GiB native backend, 512 MiB payload budget, 100,000 entries and four
128 KiB readers. Default allocator and explicit xattr/forced-stat/full-payload
work are unchanged. Fresh means a new mount/process; warm revisits the complete
dataset in that mount. Host/backend caches were not controlled or globally dropped.

Five-phase means, decimal MB; RSS is daemon peak / mean final fifth, **not a
stationary steady state**. CPU is the whole filesystem cgroup, including client
and controller work. The [numeric receipt](streamed-read-body-results-20261027.json)
also retains p50/p95, dispersion, individual CPU roles, allocator, client,
cgroup/file/kernel and separate native-backend memory/CPU.

| Implementation/state | Read MB/s | Daemon RSS peak / steady MB | Whole cgroup CPU s | Read p99 ms | Listing s |
| --- | ---: | ---: | ---: | ---: | ---: |
| Go experimental fresh | 526.487 | 1250.247 / 1214.540 | 34.561 | 13.189 | 0.489 |
| Go experimental warm | 523.264 | 1313.059 / 1218.911 | 33.412 | 13.089 | 0.031 |
| Go packaged fresh | 521.847 | 1271.393 / 1218.118 | 34.596 | 13.214 | 0.481 |
| Go packaged warm | 498.129 | 1301.252 / 1215.338 | 34.978 | 13.898 | 0.033 |
| Rust body-prefix fresh | 486.629 | 564.181 / 429.476 | 36.724 | 15.208 | 1.590 |
| Rust body-prefix warm | 470.705 | 563.541 / 454.082 | 36.732 | 16.269 | 0.021 |
| Rust prior fresh | 507.835 | 540.324 / 360.174 | 35.152 | 21.337 | 1.571 |
| Rust prior warm | 498.257 | 645.332 / 357.497 | 34.441 | 20.618 | 0.021 |

The behavior improvement is a measured tradeoff, **not an overall speed fix**.
Versus the matched prior, p99 improves about 29% fresh and 21% warm, but throughput
falls about 4%/6%, whole CPU increases and final-fifth RSS rises about 19%/27%.
P50 also increases, and p95 is 2.370/2.371 ms versus prior 1.529/1.505 ms.
Warm peak RSS falls, while fresh peak RSS increases. Do not hide those regressions
behind p99 or lower process RSS than Go. Go still wins throughput, CPU and p99;
cold listing remains slower. File cache fills most of the cgroup limit, so process
RSS does not establish a whole-memory win.

The prior premise that publication granularity alone explains the imbalance is
not sufficient: there are now body production, per-fragment cache/notification
work and allocator-retention actors to distinguish. Separate owned-PID profiles
are diagnostic only; their instrumented timings are not additional performance
samples or evidence of a Rust/SDK/network ceiling. Go heap time series, backend
request metrics, the full representative matrix and compatibility remain open.
Overall acceptance remains **UNACCEPTED**.

## Separate owned-profile follow-up

Three additional instrumented mounts use exact Go experimental, prior Rust and
body-prefix executables on a newly sealed copy of the same manifest. All 9,000
file hashes/metadata checks pass, followed by native verification of all 3,000
objects and 97 listings. Only owned daemon PIDs are sampled at 149 Hz; normal
unmount and daemon cleanup complete. Raw sample/report hashes and narrow leaf
shares are retained in the [profile receipt](streamed-read-body-profiles-20261027.json).
These are single diagnostics, **not additional speed-comparison repetitions**.

The body-prefix profile exposes cached-range bookkeeping: BTree range search is
2.73% of reported leaf samples, Moka bucket lookup 2.46%, policy maintenance 1.03%,
recency update 1.03% and range covering 0.83%. The prior's bucket lookup is 0.14%;
the other selected entries are absent at the 0.1% report threshold, not zero.
Kernel payload copying and anonymous page work remain visible in all profiles.
These samples identify a concrete next application-level candidate, not a claim
that the SDK or language imposes a ceiling or that all CPU differences are explained.

Code inspection matches that candidate: a request spanning smaller fragments first
asks for one whole covering block. That unsuccessful lookup walks older cached
fragments and records their Moka accesses before the following byte-wise coverage
loop can compose the actual reply. Isolate this repeated lookup work without
weakening generation, gap, admission or complete-byte contracts, then measure the
result against the frozen body-prefix executable.

Failed setup remains explicit. The first diagnostic rejected a stop exit status;
its extra sample file was not collected before the finite Pod expired, although
all matched evidence was already preserved. A task-volume-only read-only recovery
could not mount the expected directory. A fresh attempt exposed dangling uploaded
tool symlinks and stopped before sampling; corrected upload resolves the public
tool binaries. The successful repeat records perf's requested SIGINT exit -2 and
independently validates readable nonempty reports, sample hashes and complete
client bytes. Neither failed attempt is counted as a performance or profile pass.
