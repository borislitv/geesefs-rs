# Read-history lifetime: native comparison

## Contract and source

Candidate `2bf8184` resets the inode's adaptive read history on a zero-to-one open
transition. A new stream no longer inherits a closed stream's promoted window.
Retained payload is preserved, and opening another handle while one remains open
does not reset the active history. This is not full per-handle history isolation.

The public regression fails against `d0d7b99`: after close/reopen, the first GET
is 4 MiB instead of the configured 1 MiB, after correct returned bytes have been
verified. The fix changes only three runtime lines in `src/filesystem.rs`.
The pinned public Go `NewFileHandle` initializes fresh history per open; the
bounded narrower correction and source references are in
[dependency decisions](dependency-decisions.md#read-history-lifetime-across-closed-streams-2026-10-29).

System-feature binary SHA256:
`a0e64688a1f98c240cca7ee40ea938d809257557d6171870da3c5f055f319f23`.
The [numeric report](read-ahead-history-results-20261029.json) includes exact source
and executable identities for the candidate, prior Rust `d0d7b99`, and unchanged
Go experimental `5c0884c` / packaged `80c214e`.

## Matched measurements

96 unprofiled phases: four rotated mounts per executable, each fresh plus five
warm revisits, 288,000 full-file checks. The same sealed 3,000-file /
8,298,727,528-byte / 97-listing fixture passes native verification afterward.
No correctness errors, CPU throttling or memory-event increments are observed.
Limits remain FS/client 8 CPU / 16 GiB, backend 16 CPU / 12 GiB and parent
24 CPU / 28 GiB; eight runtime workers, sixteen backend workers, 512 MiB payload
budget, 100,000 entries and four readers with 128 KiB requests.

Warm arithmetic means (twenty phases, four independent mounts):

| Executable | MB/s | p50 ms | p95 ms | p99 ms | Whole FS/client cgroup CPU s | Daemon peak / final-fifth RSS MB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Rust reset history `2bf8184` | 1664.699 | 0.071760 | 0.736770 | 2.407736 | 24.958796 | 742.739 / 741.882 |
| Prior Rust `d0d7b99` | 1646.457 | 0.071448 | 0.706769 | 2.478021 | 24.804172 | 758.147 / 757.328 |
| Go experimental `5c0884c` | 1239.379 | 0.180008 | 0.808583 | 2.303312 | 26.099643 | 1305.202 / 1221.617 |
| Go packaged `80c214e` | 1230.100 | 0.180983 | 0.836352 | 2.340649 | 26.472807 | 1343.998 / 1231.705 |

Relative to prior Rust, throughput improves 1.11% and p99 2.84%, each in all four
mount pairs. Peak RSS decreases 2.03% on average, with one adverse pair. However,
p95 increases 4.24% and whole CPU increases 0.62%; CPU is worse in all four pairs.
These costs remain visible: the correction is not an across-the-board improvement
over the preceding candidate.

Compared with Go, throughput, p50/p95, whole CPU and daemon RSS are favorable in
this workload, but p99 remains worse: +4.53% versus experimental Go in all four
pairs, +2.87% versus packaged Go in three of four pairs. The broader acceptance
gate therefore remains open.

Warm excess response payload is **zero in every one of the twenty candidate
phases**, versus a prior-Rust mean of 82.413112 MB. The independent direct-range
union likewise has zero candidate duplicate bytes versus 66,158,829 prior bytes;
both Go references have zero. Warm range-response counts are 3899.15 candidate,
3636.90 prior and 3884 for each Go. Cold candidate phases still have mean excess
21.639925 MB; zero warm amplification is not generalized to every workload.

Group percentiles are means of phase percentiles, not pooled percentiles. Warm
revisits are not independent mounts, and final-fifth RSS is not a long-term plateau.
The report preserves dispersion, adverse pairs, cold results, CPU scopes,
allocator sampling and separate client/backend/cgroup/file/kernel memory. Changes
between historical matrices are not credited solely to runtime source changes.

## Initial-read attribution

A separate eight-phase socket/FUSE comparison now includes both Rust versions
and both Go references: 24,000 full checks, 273,752 matched syscalls, no unmatched
syscalls and complete HTTP bodies. Tail-to-network matches are a different metric
and their unmatched counts remain explicit; they are not reported as zero.

To avoid comparing unequal tail-selected populations, all 3,000 initial reads per
executable were independently rejoined by file/inode, FUSE lifetime and initial
GET. All 12,000 joins are unique, with no exclusions. Mean stage times in ms:

| Executable | Before request write | Await response headers | Receive required body | After socket bytes | FUSE service |
| --- | ---: | ---: | ---: | ---: | ---: |
| Prior Rust | 0.301591 | 1.505671 | 0.067738 | 0.150460 | 2.025460 |
| Reset-history Rust | 0.267290 | 1.421068 | 0.064132 | 0.143424 | 1.895914 |
| Go experimental | 0.185047 | 1.424625 | 0.082751 | 0.099466 | 1.791889 |
| Go packaged | 0.183368 | 1.408467 | 0.081877 | 0.101621 | 1.775333 |

The candidate and experimental Go have nearly equal mean response-header waits
in this capture. Rust's observed mean difference is concentrated before request
write (about 82 microseconds) and after required socket bytes (about 44
microseconds), partly offset by its lower body-receive interval. These are
instrumented stage boundaries, not a causal explanation of the global p99 or an
acceptance run. Socket receive completion is not NIC arrival. The next diagnosis
must distinguish FUSE-to-runtime scheduling, admission, SDK request construction,
and body-to-reply work before selecting another runtime change.

The independent initial-read receipt SHA256 is
`33aedb0af42c8eb5823cd93191710a38a3265adbfaac2c415aababf7a5bf1117`.
Its raw inputs belong to the verified capture; the derived private receipt retains
all per-file stages. No server-wide claim is inferred from a client-side clock.

## Verification and status

207 Rust tests, 37 Python tests, rustfmt, all-target/all-feature Clippy with
`-D warnings`, and the public-source audit pass in Kubernetes. Three explicit
faults fail the intended assertions; both selected open-count mutants are killed.
Source restoration and green rerun are verified. Crapper reports `open` CC 6;
coverage/CRAP and broad mutation coverage remain unavailable, and the three prior
duplicate candidates remain open. Mounted verification passes 17 ordinary cases
and retains the known hardlink EIO.

The export verifies 3655 artifact hashes and 25 compiled inputs. An independent
check verifies 9424 source-row/aggregate numeric cells and resource intervals.
The numeric report SHA256 is
`8426196cab1ab1128086a5088f1bde5e18289fac9de534791f40e7afda18411b`.

**UNACCEPTED.** Continue the remaining initial-read attribution, representative
random/hot-set/writer/metadata matrix, long-term memory and compatibility checks.
Cold-start superiority is not required; production is not replaced.
