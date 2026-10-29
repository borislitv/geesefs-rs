# Fixed-binary warm-read profiles

These are instrumented diagnostic captures, separate from the unprofiled
[resource matrix](../steady-state-tail-diagnosis.md). Filesystem/client resources
are 8 CPU / 16 GiB; the synthetic S3 backend has 16 CPU / 12 GiB. CPU and memory
limits are identical for Rust and both Go references. The two-worker and
eight-worker captures are separate runs of the same frozen executables.

| Runtime workers | Implementation | CPU | Daemon off-CPU | Client off-CPU |
| --- | --- | --- | --- | --- |
| 2 | Rust | [SVG](workers2-rust-cpu.svg) | [SVG](workers2-rust-daemon-offcpu.svg) | [SVG](workers2-rust-client-offcpu.svg) |
| 2 | Go experimental | [SVG](workers2-go-experimental-cpu.svg) | [SVG](workers2-go-experimental-daemon-offcpu.svg) | [SVG](workers2-go-experimental-client-offcpu.svg) |
| 2 | Go packaged | [SVG](workers2-go-packaged-cpu.svg) | [SVG](workers2-go-packaged-daemon-offcpu.svg) | [SVG](workers2-go-packaged-client-offcpu.svg) |
| 8 | Rust | [SVG](workers8-rust-cpu.svg) | [SVG](workers8-rust-daemon-offcpu.svg) | [SVG](workers8-rust-client-offcpu.svg) |
| 8 | Go experimental | [SVG](workers8-go-experimental-cpu.svg) | [SVG](workers8-go-experimental-daemon-offcpu.svg) | [SVG](workers8-go-experimental-client-offcpu.svg) |
| 8 | Go packaged | [SVG](workers8-go-packaged-cpu.svg) | [SVG](workers8-go-packaged-daemon-offcpu.svg) | [SVG](workers8-go-packaged-client-offcpu.svg) |

## Interpretation

- CPU widths count 99 Hz `cpu-clock` samples from the filesystem daemon. They do
  not include all benchmark-client or backend CPU. Callchain widths are inclusive;
  the numeric receipt separately classifies exclusive leaf costs.
- Off-CPU widths count aggregate thread nanoseconds. They include idle/background
  threads and can exceed elapsed experiment time. They are not request-latency
  flamegraphs and do not establish which async operation blocked a specific read.
- Separate scheduler/syscall/FUSE evidence in the report joins individual reads
  and request stages. CFS overlap is not treated as a causal wall-time fraction.
- Unknown frames remain visible. libstd/dependency inlining and incomplete root
  unwinds limit attribution; zero lost samples does not imply complete callchains.
- Open a downloaded SVG in a browser for search/zoom where the hosting preview
  disables embedded SVG interaction.

## Provenance

Generated with Brendan Gregg's public
[FlameGraph](https://github.com/brendangregg/FlameGraph/tree/41fee1f99f9276008b7cd112fca19dc3ea84ac32)
tools `stackcollapse-perf.pl` and `flamegraph.pl` at that exact revision.
Their upstream source and licensing remain authoritative; no modified tool source
is vendored here. The published SVGs only normalize trailing whitespace/newlines.
The [numeric receipt](../steady-state-headroom-results-20261029.json) records tool
revision/blob/content hashes, raw and published SVG hashes, executable provenance,
measurement scopes, pressure observations and sample dispersion.
