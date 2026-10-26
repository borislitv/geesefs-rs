# Linux/S3 compatibility inventory

**Overall compatibility: UNACCEPTED.** This is incremental implementation, not
a complete replacement or a read-only filesystem offered as a substitute.
No mounted or performance acceptance has been established.

Inventory source: public GeeseFS `5c0884c9e8799bbab93e2d4102fdc15e9c079e0d`,
`core/cfg/flags.go` NewApp and PopulateFlags, `conf_s3.go`, `main.go`,
`core/handles.go`, `core/utils.go`, `core/ycs3ext/{types,list_decode,errors}.go`.
CLI defaults are NewApp defaults, not DefaultFlags (the latter has a different
stat TTL). Unknown/unsupported options must fail, never be accepted and ignored.

## Complete reference option inventory

The following lists every NewApp option; this is an inventory, not a claim of
implementation. See the executable's help for the strictly smaller parsed set.
Parsing does not establish complete mount/backend semantics. All options not in
help are rejected. Foreground read/write mounts work for the verified subset below;
the remaining options and optional paths still prevent complete acceptance.

| Group | Reference names |
| --- | --- |
| Filesystem | o, cache, dir-mode, file-mode, cache-file-mode, uid, gid, setuid, setgid, refresh-dirs, ignore-setting-attrs-for-root-dir-erros |
| S3 endpoint / credentials | endpoint, project-id, iam, iam-header, iam-flavor, iam-url, region, requester-pays, profile, shared-config, subdomain, no-detect, no-verify-ssl |
| Object configuration | storage-class, cold-min-size, use-content-type, sse, sse-kms, sse-c, no-checksum, list-type, no-expire-multipart, multipart-age, multipart-copy-threshold, acl |
| SDK retry | sdk-max-retries, sdk-min-retry-delay, sdk-max-retry-delay, sdk-min-throttle-delay, sdk-max-throttle-delay |
| Cache / directory | memory-limit, use-enomem, read-only-direct-io, entry-limit, gc-interval, cheap, no-preload-dir, no-implicit-dir, no-dir-object, stat-cache-ttl, negative-lookup-cache-size, max-disk-cache-fd |
| Scheduling / reads | max-flushers, max-parallel-parts, max-parallel-copy, read-ahead, small-read-count, small-read-cutoff, read-ahead-small, large-read-cutoff, read-ahead-large, read-ahead-parallel, read-merge |
| Writes | single-part, part-sizes, enable-patch, drop-patch-conflicts, prefer-patch-uploads, max-merge-copy, ignore-fsync, fsync-on-close |
| Attributes | enable-perms, enable-specials, no-specials, enable-mtime, emulate-hardlinks-as-symlinks, disable-xattr, uid-attr, gid-attr, mode-attr, rdev-attr, mtime-attr, symlink-attr, refresh-attr |
| Retry / timeouts | http-timeout, retry-interval, read-retry-interval, read-retry-mul, read-retry-max-interval, read-retry-attempts |
| Diagnostics | debug, debug_fuse, debug_s3, pprof, f, log-file, print-stats, debug_grpc |
| Cluster (outside target) | cluster, grpc-reflection, cluster-me, cluster-peer |
| Built-in | help (h), version (v) |
| Windows only (outside target) | refresh-filename, flush-filename |

macOS/Windows, Azure/GCS, cluster/distributed ownership and Go pprof are not
claimed. IAM/signature negotiation, anonymous autodetection and encryption now
have backend implementations and loopback contract tests, not actual-service
acceptance. PATCH writer generation/conflict/retry integration now has deterministic
tests; actual-service acceptance remains absent.
These and all-reference CLI parity remain required optional Linux/S3 paths;
see [backend options and exact limitations](backend-options.md).
FIFO creation was exercised on both mounted implementations. Both returned EIO
for the optional hardlink-as-symlink probe on this Linux kernel; this is recorded
as a failure, not hardlink acceptance. Root directory fsync was exercised on both.

## Reference contracts and acceptance status

| Contract | Reference requirement | Status |
| --- | --- | --- |
| fstab | Exactly `program bucket mount -o comma-options`: dash-prefixed tokens become CLI flags, other tokens stay mount options | Normalizer/parser tests; foreground mount dispatch implemented |
| metadata | Lowercase escaped names before path-unescape; malformed key/value escapes omit that pair; plus is literal; empty decoded map is known empty | Adapter tests include Unicode printing categories, invalid UTF-8 RuneError and per-byte escaping |
| defaults | endpoint https://storage.yandexcloud.net; region us-east-1; memory 1000 MiB; entry 100000; modes 0755/0644; symlink key --symlink-target | Parsed subset tested |
| attributes | Configurable keys uid/gid/mode/rdev/mtime; integer Unix seconds, POSIX mode conversion, cloud/user xattrs; symlink target in metadata | Implemented subset; mounted mode/mtime/symlink/user xattr/FIFO checks on Rust and Go; full cloud/attribute matrix outstanding |
| list | V1 marker; V2/ext continuation; ext-v1 Metadata Name/Value; bounded page; malformed pages never installed | SDK V1/V2 and signed ext-v1 transport implemented; loopback signed query, tokens, compression/integrity tests |
| fallback | Only modeled InvalidArgument / NotImplemented permit ext-v1 fallback; access/serialization/network remain errors | Exact classification and actual transport fallback tests |
| auth/options | IAM, profile files, discovery/SigV2, encryption, requester-pays, MIME, TLS, multipart expiry/copy | 26 backend-option HTTP/TLS tests, profile subprocess test and MIME policy tests; actual-service coverage and full CLI parity outstanding |
| PATCH | Conditional owned mutation and updated generation metadata | Enable/prefer/drop-conflict writer policies, configured part boundaries, partial progress, metadata replacement, cancellation and explicit-conflict recovery tests; actual-service/imported-layout acceptance outstanding |
| scheduling/cache | Configurable single/multipart tiers, bounded parallel work, effective read windows and cache policy | Tier/object bounds, virtual 60 GiB layout, streamed 12 MiB real SDK PUT/checksum, parallel copy/parts and 100 MiB read-window tests; effective owner-budget report available; no mounted/performance acceptance for this tranche |
| read | Conditional unaligned Range; exact bytes; preserve stored gzip; EOF/short-body errors; release bodies | HTTP contracts and actual mounted full-byte/Range/EOF/mmap checks |
| namespace | Confined implicit/explicit dirs, paged traversal, live stable inode IDs, forget, stale LIST tombstones | Implemented; deterministic paging, stale LIST, implicit directory and replaced-inode eviction tests |
| write | Sparse zero holes, truncate/random overwrite/read-after-write, versioned snapshots, immutable flush generations, fsync completion barrier | Implemented subset; deterministic generation tests and mounted fresh S3 byte checks; full durability matrix unaccepted |
| rename/delete | Copy then delete, source condition, rename before close, delete during upload, no resurrection | Implemented; cached descendant locking, destination ETag refresh, paged partial-directory retry and in-flight commit tests; mounted file rename |
| pressure | Pinned and queued allocations charged, reserved flush progress, bounded admission | Owner charging, reserved flush lane, full dirty-budget progress and cancelled multipart cleanup tests; not a whole-process RSS bound |
| FUSE | Exactly-once owned replies, bounded scheduling, xattrs/fallocate/fsyncdir/mmap/RO/shutdown | fuser callbacks implemented; actual mount/unmount, mmap, fsyncdir exercised; full callback/failure matrix outstanding |
| clients | rsync full/delta, nginx full/Range, concurrent writers, injected upload/copy/completion failures | Actual rsync and aiohttp full/Range checks on Rust/Go; Rust mounted fsync recovery passes; Go open-file recovery remains EAGAIN through 75-second deadline; no literal nginx or all-xfstests claim |
| performance | Both verified Go baselines, unchanged budgets, cold/warm, five interleaved repetitions, all accounting and tails | Not measured; gate must fail closed |

No atomic S3 rename or writable-unlinked POSIX behavior is promised by the
reference. A failure of required optional behavior leaves overall compatibility
unaccepted even when ordinary operations work.
