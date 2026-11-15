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

The [mirror payload pattern](mirror-payload.regex) selects packages/installers,
disk images and archives, case-insensitively and only at the end of the filename:

- Packages/installers: deb/udeb, rpm, apk, AppImage, run, exe, msi, cab.
- Images: iso, img, raw, qcow/qcow2, vhd/vhdx, vmdk, vdi, dmg, squashfs; these and
  tar also support gz, bz2, xz, zst, lz4 and lzma compression suffixes.
- Other archives: tgz, tbz/tbz2, txz, tzst, zip, 7z, rar, ova, whl, jar, crate.

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
`Packages.gz`, repository XML, checksums/signatures, torrents and partial download
suffixes are not selected. Files with unlisted extensions remain outside this
policy regardless of size. A directory named `something.iso` does not select its
`README` child. CLI tests exercise the actual pattern, including uppercase names,
compressed images, mount-relative paths and sidecar exclusions.

Change the launch policy at an ordinary planned restart and record the new
configuration separately from an ongoing measurement. Preserve the existing cache
inventory; newly selected paths are populated by subsequent eligible source fills.

## Two independent fullness limits

See [the Prometheus contract](metrics.md#persistent-payload-tier) for index slots
and actual data-region capacity/free/used observations. The index counts payload
records, pending updates and tombstones, not complete files or distinct objects.
Region fullness includes partially used writer regions and obsolete records.
The tombstone log and unusable tail are outside the reported data-region pool.
Allocated file size and cumulative written bytes are not occupancy measurements.

The index automatically scales with configured capacity at one slot per full
128-KiB payload record, limited by the existing memory-derived allowance. There
is no separate 8,388,608-slot ceiling. A page-aligned 1.5-decimal-TB device
(`1499999997952` bytes) receives 11,444,091 slots when memory permits; a 2-TiB
device receives 16,777,216. Fixed index funding is deducted from the data lane,
not added outside `--memory-limit`. A smaller memory budget can still reduce the
index, and partial records lower the useful payload represented by those slots.
Physical regions can continue to be written/recycled after index evictions.

Capacity must be a multiple of 4096 bytes. Existing media cannot be resized by
changing the flag: Foyer's capacity-dependent layout must match the existing
file. Use an explicitly authorized cold replacement or a separately qualified
migration. Keep the cache directory outside another application's managed cache
tree; a cache loader or cleaner may delete files it does not recognize.
