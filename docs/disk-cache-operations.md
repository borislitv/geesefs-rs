# Persistent payload cache observations and selection

L2 is populated from eligible validated source fills, in records of at most
128 KiB. There is no retained application RAM payload tier. Active reads and
replies retain charged immutable owners; independent reuse goes through Foyer
and the backing-file page cache. Bounded read-ahead does not proactively download
every matching file or require completing a large object. Without an eligible
funded L2, reads fetch only their requested bytes. Population remains best-effort
within the existing byte/job budgets; inspect copy/enqueue drops, hits and writes.

## Mirror payload selection

`--disk-cache-regex` matches the UTF-8 mount-relative **path**, with a leading `/`
and without the S3 mount prefix. The resolved file must still be regular, clean,
generation-valid and in a supported remote namespace. Regex selection is not a
size threshold, and it does not bypass these checks.

The [mirror payload pattern](mirror-payload.regex) selects observed artifact
formats case-insensitively and only at the end of the filename:

- Packages/installers: deb/udeb/ddeb, rpm, apk, xbps, hpkg, ipk, ipa, flatpak,
  AppImage, run, exe, msi/msix, cab, bin, hpi, vsix, war and xzm.
- Images: iso, img, raw, qcow/qcow2, vhd/vhdx, vmdk, vdi, dmg, squashfs; these and
  tar, pkg, zip and diff also support gz, bz2, xz, zst, lz4 and lzma suffixes.
- Other archives/objects: tgz, tbz/tbz2, txz, tzst, 7z, rar, ova, whl, jar, far,
  crate, OSTree filez, debug-symbol pdb and Git pack.
- Narrow path exceptions: versioned `dl.k8s.io/v*/bin/<os>/<arch>/kubectl`
  executables and `manjaro/<channel>/<repo>/<arch>/*.files` repository databases.

The 2026-11-17 expansion follows a bounded one-hour access-log inspection:
previously omitted package formats, binaries and repeatedly downloaded large
file databases. Request bytes and repeated URLs are not measured L2 savings;
ranges can be disjoint and future generations still require validation. The
database exception preserves normal metadata TTL and ETag/size checks.

For example, load the checked-in one-line pattern from the repository root:

```bash
IFS= read -r disk_cache_regex < docs/mirror-payload.regex
geesefs-rs -f example-bucket /mnt/mirror \
  --disk-cache-dir /var/cache/geesefs-rs/mirror \
  --disk-cache-max-size '2 TiB' \
  --disk-cache-regex "$disk_cache_regex"
```

Paths and capacity are illustrative. This is an explicit operator policy, not a
new global default. The generic regex option keeps its original semantics.
`Packages.gz`, general repository XML/JSON, checksums/signatures, torrents and partial download
suffixes are not selected. Files with unlisted extensions remain outside this
policy regardless of size. A directory named `something.iso` does not select its
`README` child. CLI tests exercise the actual pattern, including uppercase names,
compressed images, mount-relative paths and sidecar exclusions.

Change the launch policy at an ordinary planned restart and record the new
configuration separately from an ongoing measurement. Preserve the existing cache
inventory; newly selected paths are populated by subsequent eligible source fills.

## Capacity and native index allowances

See [the Prometheus contract](metrics.md#persistent-payload-tier) for data-region
capacity/free/used observations. Region fullness includes partially used writer
regions and obsolete records. Allocated file size and cumulative written bytes
are not occupancy measurements. Immutable keys do not use a tombstone log.

The released native HashMap has no application-enforced slot cap or public entry
count. Funding uses a nominal full-128-KiB-record count: a page-aligned 1.5-decimal-TB
device (`1499999997952` bytes) plans 11,444,091 entries; a 2-TiB device plans
16,777,216. Smaller records can exceed those counts. The fixed allowance is
deducted from the data lane within `--memory-limit`; insufficient funding rejects
configuration rather than silently lowering an index cap. Measure native index
and recovery memory separately from the planned allowance.

## Cache geometry and cold transition

Version 0.1.22 sizes regions from the configured capacity in bytes. Below
256 MiB it preserves the 1 MiB layout. Otherwise compute `ceil(capacity / 150000)`
and choose the smallest supported region at least that large, capped at 64 MiB.
The cap keeps reclamation granularity bounded even if there are more than
150,000 regions. Selection does not depend on current free space or occupancy.

| Configured capacity | Native region size | Backing filename |
| --- | --- | --- |
| Below 256 MiB | 1 MiB | `payload-upstream-v1.cache` |
| 256 MiB through `150000 × 16 MiB` (inclusive) | 16 MiB | `payload-upstream-v1.cache` |
| Above `150000 × 16 MiB` through `150000 × 32 MiB` (inclusive) | 32 MiB | `payload-upstream-r32-v1.cache` |
| Above `150000 × 32 MiB` | 64 MiB | `payload-upstream-r64-v1.cache` |

Both 2 TiB + 1 decimal TB (`3199023255552` bytes) and exactly 3 TiB use 32 MiB regions.
The flags `--disk-cache-dir`, `--disk-cache-max-size` and `--disk-cache-regex`
are unchanged. Capacity is still page-aligned (a multiple of 4096 bytes) and
limited to 8 MiB–16 TiB. Payload records are still at most 128 KiB. The engine
remains exact crates.io Foyer 0.22.6 with hot-record reinsertion enabled. Only whole regions are usable:
any partial region at the end is an unused internal tail in the allocated file.
Larger regions make reclamation coarser: each victim covers more capacity and
can affect more records at once.

The legacy, 32 MiB and 64 MiB native layouts are mutually incompatible, and the
former fork's `payload-v1.cache` is incompatible with all of them. Changing capacity
does not resize existing media. Reuse requires the matching layout and capacity.
For an explicitly authorized cold transition, stop the current cache owner,
confirm no process still holds the cache, and verify the directory and media
belong to the expected service account before moving any inventory. Start with
an empty private cache directory on a local filesystem, outside the mount and
other applications' managed cache trees. Preserve the old directory separately
if rollback is needed. Do not rename one layout's file to the other's basename.
Startup rejects incompatible inventory without modifying it; there is no
automatic reset on startup or after a cache error.
