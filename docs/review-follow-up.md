# Core/backend inspection follow-up

This records independently checked behavior, not mounted or performance acceptance.
Public Go reference revision: `5c0884c9e8799bbab93e2d4102fdc15e9c079e0d`.

## Candidate disposition

| # | Disposition | Evidence and regression |
| --- | --- | --- |
| 1 | Confirmed and fixed | A delayed HEAD replaced a newer successfully flushed inode: size 3 observed instead of 5. Lookup now checks inode ID, local version and remote generation as well as namespace epoch before replacing clean state; negative publication uses the same observation. `stale_lookup_head_cannot_replace_a_new_successfully_flushed_generation` failed before the fix. |
| 2 | Confirmed and fixed | Expired clean cached names survived an empty authoritative LIST. Directory selection now retains fresh cache entries and dirty local children/ancestors, not expired clean ghosts. The regression first failed on the ghost entry, then caught loss of an implicit parent containing a dirty child during the fix. |
| 3 | Probe scenario already guarded; final publication hardened | The existing epoch check already rejected a probe spanning parent rename/delete: the deterministic regression passed before changes. Creation additionally validates parent ID/name binding, current key, deleted/rename state and namespace reservations under the publication lock. No deterministic failure of the narrower thread-preemption window is claimed. |
| 4 | Confirmed and fixed | A child was created across the rmdir empty-LIST barrier. A bounded prefix lease now covers the emptiness probe and DELETE; mutators check the reservation at their actual local mutation. `rmdir_reserves_the_directory_before_its_remote_empty_probe` failed first. |
| 5 | Confirmed and fixed | Two unlink callers waited behind a paused fsync; the second deleted a recreated name. After acquiring the flush gate, unlink checks deleted state, captured key and name-to-inode binding before reserving/deleting. The second-unlink regression failed first. |
| 6 | Confirmed and fixed | Cancellation before/after applied DELETE left the dirty inode hidden and unlink retry returned ENOENT. Ambiguous deletes now retain a bounded original-name record and dirty ownership, block conflicting creation/moves, and reconcile through unlink retry or `reconcile_pending_deletes`. Flush-all/shutdown reconcile before flushing dirty data. Modeled authorization/validation rejection rolls back the pending record instead. Cancellation tests verify retry, bytes, replacement safety and absence of resurrection on shutdown. |
| 7 | Confirmed and fixed | Successful PUT followed by failed verification HEAD left conditional reads using an obsolete ETag (ESTALE). The known committed generation/size are recorded before that await without retiring dirty extents. Single-part, multipart and cancelled-verification tests check readable bytes and successful retry. The existing foreign-same-size HEAD regression still rejects false durability acknowledgement. |
| 8 | Confirmed and fixed | Object HEAD/LIST timestamps were discarded. ObjectHead carries LastModified and inode initialization uses it after any enabled explicit mtime attribute. Restoring the old now fallback failed the fixed timestamp assertion. Loopback HTTP also verifies the SDK timestamp conversion. |
| 9 | Confirmed false type hint; fixed with an explicit compatibility distinction | Public Go `makeDirEntry` emits DT_File for every non-directory, so this omission also exists in the reference. Safe fuser 0.18 has no DT_UNKNOWN variant. Standard-LIST unknown types are therefore resolved through bounded selected-page lookups before emitting concrete types; authoritative extended metadata avoids extra HEADs. The regression failed with RegularFile hints instead of NamedPipe/Symlink and now checks pagination, HEAD bounds and the no-extra-HEAD extended path. This adds metadata work for standard LIST, not an unmeasured performance win. |
| 10 | Confirmed and fixed | Root setattr dirtied a synthetic bucket-root inode that could only flush an invalid empty key. Root attribute/xattr mutation now returns EOPNOTSUPP without dirtying it, matching default Go root-attribute rejection. No ignore-root-attribute-errors flag parity is claimed. Root refresh is a no-op. |
| 11 | Confirmed wire omission and fixed | Requester-pays was absent on PUT/copy/parts and DELETE. Go has the same GET/HEAD/POST whitelist; this fix implements the S3 write/delete requirement rather than claiming Go already sends those headers. Real SDK requests now assert the header is present and signed on each phase. |
| 12 | Confirmed and fixed | Metadata REPLACE discarded HTTP object properties and MIME guessing replaced the source Content-Type. HEAD carries the properties; self-COPY and multipart initiation preserve Content-Type/Encoding/Disposition/Language, Cache-Control, Expires and website redirect. Loopback tests cover both copy forms and independently verify that new uploads still receive guessed MIME. |
| 13 | Caller identity confirmed; setgid-parent inheritance refuted | Public Go CreateFile/MkDir/CreateSymlink pass request UID/GID to SetAttributes, which applies them only with enable-perms. It does not derive userspace ownership from the setgid parent. `create_with_identity` exposes that contract for FUSE wiring; tests verify fresh-reader persistence, disabled-permission mount defaults, and no invented group inheritance. Substituting mount identity failed the regression. |

## Public reference evidence

- [Go lookup/creation and dirent callbacks](https://github.com/yandex-cloud/geesefs/blob/5c0884c9e8799bbab93e2d4102fdc15e9c079e0d/core/goofys_fuse.go):
  `makeDirEntry` lines 381-397; caller ownership at 626, 678 and 720.
- [Go inode metadata](https://github.com/yandex-cloud/geesefs/blob/5c0884c9e8799bbab93e2d4102fdc15e9c079e0d/core/handles.go):
  LastModified at 223-225 and permission metadata handling.
- [Go SetAttributes](https://github.com/yandex-cloud/geesefs/blob/5c0884c9e8799bbab93e2d4102fdc15e9c079e0d/core/file.go):
  root rejection at 1880-1887; enable-perms UID/GID at 1936-1962.
- [Go S3 options](https://github.com/yandex-cloud/geesefs/blob/5c0884c9e8799bbab93e2d4102fdc15e9c079e0d/core/backend_s3.go):
  requester-pays whitelist at 297-302, COPY/REPLACE metadata behavior.
- [Safe fuser FileType](https://docs.rs/fuser/0.18.0/fuser/enum.FileType.html):
  concrete types only, no unknown variant. No unsafe FUSE encoding was added.

## Parent integration API

```rust
Filesystem::refresh_inode_if_expired(ino: u64) -> Result<Arc<Inode>>
Filesystem::create_with_identity(
    parent: u64, name: &str, mode: u32, target: Option<&[u8]>,
    rdev: u32, identity: (u32, u32),
) -> Result<Arc<Inode>>
Filesystem::reconcile_pending_deletes() -> Result<()>
```

All three functions are async. Getattr/fstat should await refresh and then use
the returned inode's attr. The refresh preserves inode identity, opens and lookup
references, skips dirty/pending-rename state, checks clean remote generations,
returns ordinary remote errors without replacing state, and never HEADs synthetic
root. Implicit directories use LIST; dirty descendants keep their parent visible.
Creation callbacks supply request UID/GID; caller mode remains the caller's mode,
without new userspace setgid derivation. The old create API remains available.

Pending DELETEs intentionally return EAGAIN from competing namespace operations
until reconciliation; dirty ownership is retained, not acknowledged durable.
Unlink retry and flush-all/shutdown perform reconciliation. This cannot promise
recovery from process loss or preserve externally overwritten/deleted remote data.

ObjectHead now also has `last_modified` and `properties` fields and implements
Default for existing fixture constructors. No new dependency or unsafe code was
introduced. FUSE/main, scripts, benchmark tooling and CI are parent-owned and
were not edited by this work.

## Verification boundary

The new cases live in `tests/review_regressions.rs` (14 cases) and
`tests/backend_options.rs` (26 total cases). Final formatting check, all-target/
all-feature warning-denying Clippy and all 119 Rust tests passed, none ignored.
The standard-LIST regression also caught type resolution conflicting with the
rmdir reservation; the internal emptiness probe now omits type resolution and
correctly returns ENOTEMPTY without publishing any falsely typed dirents.
The evidence-gate Rust case includes the Python gate tests. No mounted,
actual-service, SSH, production, VCS or benchmark run was performed here.
Compatibility and comparative performance remain unaccepted.
