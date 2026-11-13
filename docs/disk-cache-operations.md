# Persistent payload cache observations and selection

L2 is populated from eligible clean L1 capacity victims, in records of at most
128 KiB. It does not proactively download every matching file or retain an entire
large object on its first read. Admission is best-effort within existing byte/job
budgets; inspect demotion drops as well as hits and writes.

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
inventory; newly selected paths are populated by subsequent eligible evictions.

## Two independent fullness limits

See [the Prometheus contract](metrics.md#persistent-payload-tier) for index slots
and actual data-region capacity/free/used observations. The index counts payload
records, pending updates and tombstones, not complete files or distinct objects.
Region fullness includes partially used writer regions and obsolete records.
The tombstone log and unusable tail are outside the reported data-region pool.
Allocated file size and cumulative written bytes are not occupancy measurements.

On the existing bounded policy, an 8,388,608-slot ceiling can represent at most
about 1 TiB of 128-KiB payload records even with a 2-TiB file; partial records lower
that useful payload total. Physical regions can continue to be written/recycled
after index evictions. Neither matching more suffixes nor optional compression
removes this independent index bound.
