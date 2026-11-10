#![forbid(unsafe_code)]
#[path = "support/disk.rs"]
mod support;
use budget::Budget;
use disk_cache::{Demotion, DiskCache, DiskKey, PreparedDiskCache, State};
use std::{
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::MetadataExt,
};
use support::{DEADLINE, MIB, block, key, open};
pub use support::{budget, disk_cache, error};

#[tokio::test]
async fn completed_partial_and_short_eof_survive_reopen_without_repreallocation() {
    let dir = support::directory();
    let resources = Budget::new(32 * MIB).unwrap();
    let output = Budget::new(MIB).unwrap();
    let eof = DiskKey::new(b"namespace", b"tail.rpm", b"etag", 131_079, 131_072, 7).unwrap();
    let cache = open(dir.path(), &resources).await;
    assert_eq!(
        cache.try_demote(eof.clone(), block(b"EOFtail")),
        Demotion::Admitted
    );
    cache.drain(DEADLINE).await.unwrap();
    let allocated = std::fs::metadata(dir.path().join("payload-v1.cache"))
        .unwrap()
        .blocks();
    assert!(allocated * 512 >= (16 * MIB) as u64);
    cache.close(DEADLINE).await.unwrap();
    drop(cache);
    let cache = open(dir.path(), &resources).await;
    assert_eq!(
        std::fs::metadata(dir.path().join("payload-v1.cache"))
            .unwrap()
            .blocks(),
        allocated
    );
    assert!(
        cache.stats().io_read_bytes < MIB as u64,
        "startup scans indexes/log, not the payload container"
    );
    assert_eq!(
        cache.read_block(&eof, &output).await.unwrap().as_slice(),
        b"EOFtail"
    );
    assert!(
        cache
            .read_block(
                &DiskKey::new(b"namespace", b"tail.rpm", b"etag", 131_080, 131_072, 8).unwrap(),
                &output
            )
            .await
            .is_none()
    );
    cache.close(DEADLINE).await.unwrap();
}

#[tokio::test]
async fn process_interruption_preserves_completed_unrelated_inventory() {
    for mode in ["graceful", "abrupt"] {
        let dir = support::directory();
        support::child(mode, dir.path()).await;
        let resources = Budget::new(32 * MIB).unwrap();
        let output = Budget::new(MIB).unwrap();
        let cache = open(dir.path(), &resources).await;
        assert_eq!(
            cache
                .read_block(&key(0, 9), &output)
                .await
                .unwrap()
                .as_slice(),
            b"completed",
            "{mode}"
        );
        let pending = cache.read_block(&key(128, 7), &output).await;
        if mode == "graceful" {
            assert_eq!(pending.unwrap().as_slice(), b"pending");
        } else {
            assert!(pending.is_none());
        }
        cache.close(DEADLINE).await.unwrap();
    }
}

#[tokio::test]
async fn second_process_cannot_open_locked_directory_even_before_engine_open() {
    let dir = support::directory();
    let prepared = PreparedDiskCache::prepare(dir.path(), (16 * MIB) as u64).unwrap();
    assert_eq!(prepared.status().state(), State::Prepared);
    support::child("locked", dir.path()).await;
    assert!(
        !dir.path().join("payload-v1.cache").exists(),
        "locking must precede preallocation"
    );
    drop(prepared);
    support::child("unlocked", dir.path()).await;
}

#[tokio::test]
async fn cancelled_recovery_keeps_funding_and_directory_until_owned_startup_closes() {
    use std::sync::atomic::Ordering;
    let dir = support::directory();
    let resources = Budget::new(32 * MIB).unwrap();
    let cache = open(dir.path(), &resources).await;
    assert_eq!(
        cache.try_demote(key(0, 3), block(b"old")),
        Demotion::Admitted
    );
    cache.drain(DEADLINE).await.unwrap();
    cache.close(DEADLINE).await.unwrap();
    drop(cache);
    let prepared = PreparedDiskCache::prepare(dir.path(), (16 * MIB) as u64).unwrap();
    let status = prepared.status();
    assert_eq!(status.startup().startup.count, 0);
    assert_eq!(status.opened_file_allocated_bytes(), None);
    let gate = support::IoGate::new();
    gate.hold_reads.store(true, Ordering::SeqCst);
    let opening = {
        let resources = resources.clone();
        let gate = gate.clone();
        tokio::spawn(async move {
            DiskCache::open_test(
                prepared,
                &resources,
                Some(Box::new(support::GateConfig(gate))),
            )
            .await
        })
    };
    tokio::time::timeout(DEADLINE, gate.entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    assert_eq!(status.state(), State::Recovering);
    assert_eq!(status.startup().startup.count, 0);
    assert_eq!(status.startup().allocation.count, 1);
    assert_eq!(status.startup().recovery.count, 0);
    assert!(status.opened_file_allocated_bytes().unwrap() >= (16 * MIB) as u64);
    opening.abort();
    assert!(matches!(opening.await, Err(error) if error.is_cancelled()));
    assert!(resources.used() > 0);
    assert!(PreparedDiskCache::prepare(dir.path(), (16 * MIB) as u64).is_err());
    gate.hold_reads.store(false, Ordering::SeqCst);
    gate.release.add_permits(1);
    tokio::time::timeout(DEADLINE, async {
        while status.state() != State::Closed || resources.used() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(status.startup().startup.count, 1);
    assert_eq!(status.startup().recovery.count, 1);
    println!("cancelled_startup_observations={:?}", status.startup());
    assert!(
        status.startup().startup.total_duration_ns
            >= status.startup().allocation.total_duration_ns
                + status.startup().recovery.total_duration_ns
    );
    let cache = open(dir.path(), &resources).await;
    assert_eq!(
        cache
            .read_block(&key(0, 3), &Budget::new(3).unwrap())
            .await
            .unwrap()
            .as_slice(),
        b"old"
    );
    cache.close(DEADLINE).await.unwrap();
}

#[tokio::test]
async fn disk_cache_child() {
    let Ok(mode) = std::env::var("GEESEFS_L2_CHILD_MODE") else {
        return;
    };
    let path = std::path::PathBuf::from(std::env::var_os("GEESEFS_L2_CHILD_DIR").unwrap());
    if mode == "locked" {
        assert!(PreparedDiskCache::prepare(&path, (16 * MIB) as u64).is_err());
        return;
    }
    if mode == "unlocked" {
        assert!(PreparedDiskCache::prepare(&path, (16 * MIB) as u64).is_ok());
        return;
    }
    let resources = Budget::new(32 * MIB).unwrap();
    let gate = support::IoGate::new();
    let cache = support::gated(&path, &resources, gate.clone()).await;
    assert_eq!(
        cache.try_demote(key(0, 9), block(b"completed")),
        Demotion::Admitted
    );
    cache.drain(DEADLINE).await.unwrap();
    assert_eq!(
        cache
            .read_block(&key(0, 9), &Budget::new(9).unwrap())
            .await
            .unwrap()
            .as_slice(),
        b"completed"
    );
    gate.publication_only
        .store(true, std::sync::atomic::Ordering::SeqCst);
    gate.hold_writes
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        cache.try_demote(key(128, 7), block(b"pending")),
        Demotion::Admitted
    );
    tokio::time::timeout(DEADLINE, gate.entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    assert_eq!(
        gate.payload_written.available_permits(),
        1,
        "real payload write completed, but its publication index is still gated"
    );
    tokio::time::timeout(DEADLINE, gate.log_written.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    assert!(
        cache
            .read_block(&key(128, 7), &Budget::new(7).unwrap())
            .await
            .is_none()
    );
    if mode == "abrupt" {
        std::process::exit(0);
    }
    assert_eq!(mode, "graceful");
    gate.hold_writes
        .store(false, std::sync::atomic::Ordering::SeqCst);
    gate.release.add_permits(16);
    cache.close(DEADLINE).await.unwrap();
}

#[tokio::test]
async fn corrupt_record_is_a_miss_not_zero_fill_or_destructive_recovery() {
    let dir = support::directory();
    let resources = Budget::new(32 * MIB).unwrap();
    let payload = b"independent integrity test payload";
    let cache = open(dir.path(), &resources).await;
    assert_eq!(
        cache.try_demote(key(0, payload.len()), block(payload)),
        Demotion::Admitted
    );
    cache.drain(DEADLINE).await.unwrap();
    cache.close(DEADLINE).await.unwrap();
    drop(cache);
    let path = dir.path().join("payload-v1.cache");
    let bytes = std::fs::read(&path).unwrap();
    let position = bytes
        .windows(payload.len())
        .position(|window| window == payload)
        .unwrap();
    let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.seek(SeekFrom::Start(position as u64)).unwrap();
    file.write_all(b"X").unwrap();
    drop(file);
    let before = std::fs::read(&path).unwrap();
    let cache = open(dir.path(), &resources).await;
    assert!(
        cache
            .read_block(&key(0, payload.len()), &Budget::new(MIB).unwrap())
            .await
            .is_none()
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "a corrupt cache copy cannot authorize media reset"
    );
    // Read integration later performs the actual S3 fallback; this unit never
    // fabricates a cache hit or substitutes zeroes for the authoritative bytes.
    cache.close(DEADLINE).await.unwrap();
}

#[tokio::test]
async fn capacity_and_payload_budget_shrink_never_truncate_or_wipe_media() {
    let dir = support::directory();
    let resources = Budget::new(32 * MIB).unwrap();
    let cache = open(dir.path(), &resources).await;
    assert_eq!(
        cache.try_demote(key(0, 3), block(b"old")),
        Demotion::Admitted
    );
    cache.drain(DEADLINE).await.unwrap();
    cache.close(DEADLINE).await.unwrap();
    drop(cache);
    let path = dir.path().join("payload-v1.cache");
    let before = std::fs::read(&path).unwrap();
    assert!(PreparedDiskCache::prepare(dir.path(), (8 * MIB) as u64).is_err());
    let prepared = PreparedDiskCache::prepare(dir.path(), (16 * MIB) as u64).unwrap();
    let startup = prepared.status();
    assert!(
        DiskCache::open(prepared, &Budget::new(MIB).unwrap())
            .await
            .is_err()
    );
    assert_eq!(startup.state(), State::Bypass);
    assert_eq!(startup.startup().startup.count, 1);
    assert_eq!(startup.startup().allocation.count, 0);
    assert_eq!(startup.startup().recovery.count, 0);
    assert_eq!(startup.opened_file_allocated_bytes(), None);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let cache = open(dir.path(), &resources).await;
    assert_eq!(
        cache
            .read_block(&key(0, 3), &Budget::new(3).unwrap())
            .await
            .unwrap()
            .as_slice(),
        b"old"
    );
    cache.close(DEADLINE).await.unwrap();
}

#[tokio::test]
async fn unsafe_log_history_and_truncated_media_return_errors_without_reset() {
    let dir = support::directory();
    let resources = Budget::new(32 * MIB).unwrap();
    let cache = open(dir.path(), &resources).await;
    assert_eq!(
        cache.try_demote(key(0, 3), block(b"old")),
        Demotion::Admitted
    );
    cache.drain(DEADLINE).await.unwrap();
    cache.close(DEADLINE).await.unwrap();
    drop(cache);
    let path = dir.path().join("payload-v1.cache");
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    let mut original = [0; 8];
    file.read_exact(&mut original).unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(b"unsafe!!").unwrap();
    let before = std::fs::read(&path).unwrap();
    let prepared = PreparedDiskCache::prepare(dir.path(), (16 * MIB) as u64).unwrap();
    let status = prepared.status();
    assert!(DiskCache::open(prepared, &resources).await.is_err());
    assert_eq!(status.state(), State::Bypass);
    assert_eq!(status.startup().startup.count, 1);
    assert_eq!(status.startup().allocation.count, 1);
    assert_eq!(status.startup().recovery.count, 1);
    assert!(status.opened_file_allocated_bytes().unwrap() >= (16 * MIB) as u64);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(&original).unwrap();
    file.set_len((8 * MIB) as u64).unwrap();
    let truncated = std::fs::read(&path).unwrap();
    assert!(PreparedDiskCache::prepare(dir.path(), (16 * MIB) as u64).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), truncated);
}

/// Explicit opt-in (requires Linux root, mount namespaces, loop devices and
/// e2fsprogs and util-linux). Run from the repository with an outer finite deadline:
///
/// timeout 300s env CARGO_BUILD_JOBS=8 CARGO_TARGET_DIR=target/l2adapter \
/// "$HOME/.cargo/bin/cargo" +1.99.1 test --locked --test disk_cache_recovery \
/// native_filesystem_failure_paths -- --ignored --exact --nocapture
///
/// No existing mounts, device cache advice, or host cache controls are touched.
#[tokio::test]
#[ignore = "opt-in disposable ext4 mount; unavailable prerequisites must fail"]
async fn native_filesystem_failure_paths() {
    use std::os::unix::fs::PermissionsExt;
    let owned = support::directory();
    // Permit the dedicated unprivileged child to traverse the fixture parent.
    std::fs::set_permissions(owned.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut command = tokio::process::Command::new("unshare");
    command
        .args(["--mount", "--propagation", "private", "--"])
        .arg(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "native_filesystem_failure_child",
            "--ignored",
            "--nocapture",
        ])
        .env("GEESEFS_L2_NATIVE_DIR", owned.path())
        .env(
            "GEESEFS_L2_PARENT_MNT",
            std::fs::read_link("/proc/self/ns/mnt").unwrap(),
        );
    support::native_command(&mut command).await;
    assert!(
        std::fs::read_dir(owned.path().join("mount"))
            .unwrap()
            .next()
            .is_none(),
        "the fixture must not remain mounted in the caller namespace"
    );
    let path = owned.path().to_owned();
    owned.close().expect("owned fixture cleanup failed");
    assert!(
        !path.exists(),
        "owned image and mount directory were not removed"
    );
}

#[tokio::test]
#[ignore = "internal child of native_filesystem_failure_paths; never run directly"]
async fn native_filesystem_failure_child() {
    support::assert_private_mount_namespace();
    let owned = std::path::PathBuf::from(std::env::var_os("GEESEFS_L2_NATIVE_DIR").unwrap());
    let image = owned.join("owned.ext4");
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&image)
        .unwrap();
    // Bound host usage to 64 MiB and require real physical allocation. Sparse
    // images/tmpfs do not establish the adapter's allocated-blocks contract.
    rustix::fs::fallocate(
        &file,
        rustix::fs::FallocateFlags::empty(),
        0,
        (64 * MIB) as u64,
    )
    .unwrap();
    file.sync_all().unwrap();
    drop(file);
    let mount = owned.join("mount");
    std::fs::create_dir(&mount).unwrap();
    support::native_command(
        tokio::process::Command::new("mkfs.ext4")
            .args(["-F", "-q", "-m", "0", "-N", "256"])
            .arg(&image),
    )
    .await;
    support::native_command(
        tokio::process::Command::new("mount")
            .args(["-t", "ext4", "-o", "loop", "--"])
            .arg(&image)
            .arg(&mount),
    )
    .await;
    let mounted = support::NativeMount(mount.clone());
    let fs = rustix::fs::statfs(&mount).unwrap();
    assert_eq!(fs.f_type as u64, 0xef53, "a real ext4 fixture is required");
    native_block_exhaustion(&mount).await;
    native_inode_exhaustion(&mount).await;

    let permission = mount.join("permission");
    std::fs::create_dir(&permission).unwrap();
    support::native_command(
        tokio::process::Command::new("chown")
            .args(["65534:65534", "--"])
            .arg(&permission),
    )
    .await;
    // The checkout can live below /root (not traversable by nobody). Copy only
    // this executable to the owned, traversable fixture parent, never chmod the
    // checkout or an existing host directory.
    let executable = owned.join("permission-child");
    std::fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
    support::native_command(
        tokio::process::Command::new("setpriv")
            .args([
                "--reuid",
                "65534",
                "--regid",
                "65534",
                "--clear-groups",
                "--",
            ])
            .arg(&executable)
            .args([
                "--exact",
                "native_permission_failure_child",
                "--ignored",
                "--nocapture",
            ])
            .env("GEESEFS_L2_PERMISSION_DIR", &permission),
    )
    .await;
    drop(mounted);
}

async fn native_block_exhaustion(mount: &std::path::Path) {
    let resources = Budget::new(32 * MIB).unwrap();
    let existing = mount.join("existing");
    let cache = open(&existing, &resources).await;
    assert_eq!(
        cache.try_demote(key(0, 9), block(b"preserved")),
        Demotion::Admitted
    );
    cache.drain(DEADLINE).await.unwrap();
    cache.close(DEADLINE).await.unwrap();
    drop(cache);
    let path = existing.join("payload-v1.cache");
    let before = std::fs::read(&path).unwrap();
    let failing = mount.join("no-blocks");
    let prepared = PreparedDiskCache::prepare(&failing, (16 * MIB) as u64).unwrap();
    let observer = prepared.status();
    let lock_inode = std::fs::metadata(failing.join(".lock")).unwrap().ino();
    let filler_path = mount.join("block-filler");
    let mut filler = std::fs::File::create(&filler_path).unwrap();
    let bytes = [0x5a; 64 * 1024];
    let failure = loop {
        match filler.write_all(&bytes) {
            Ok(()) => (),
            Err(error) => break error,
        }
    };
    assert_eq!(failure.raw_os_error(), Some(libc::ENOSPC));
    let space = rustix::fs::statvfs(mount).unwrap();
    assert!(
        space.f_ffree > 0,
        "block failure must not be inode exhaustion"
    );
    println!(
        "ext4 block fixture: available_bytes={} free_inodes={}",
        space.f_bavail * space.f_frsize,
        space.f_ffree
    );
    let result = DiskCache::open(prepared, &resources).await;
    assert!(
        matches!(result, Err(error::Error::Fs(libc::ENOSPC))),
        "expected native fallocate ENOSPC"
    );
    assert_eq!(observer.state(), State::Bypass);
    assert_eq!(observer.startup().startup.count, 1);
    assert_eq!(observer.startup().allocation.count, 1);
    assert_eq!(observer.startup().recovery.count, 0);
    assert_eq!(observer.opened_file_allocated_bytes(), None);
    assert_eq!(resources.used(), 0);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "unrelated inventory changed"
    );
    assert_eq!(
        std::fs::metadata(failing.join(".lock")).unwrap().ino(),
        lock_inode
    );
    let partial = failing.join("payload-v1.cache");
    let partial_bytes = std::fs::read(&partial).unwrap();
    assert!(PreparedDiskCache::prepare(&failing, (16 * MIB) as u64).is_err());
    assert_eq!(
        std::fs::read(&partial).unwrap(),
        partial_bytes,
        "failed media must not be repaired/reset"
    );
    drop(filler);
    std::fs::remove_file(filler_path).unwrap();
    let cache = open(&existing, &resources).await;
    assert_eq!(
        cache
            .read_block(&key(0, 9), &Budget::new(9).unwrap())
            .await
            .unwrap()
            .as_slice(),
        b"preserved"
    );
    cache.close(DEADLINE).await.unwrap();
    drop(cache);
    println!(
        "native block ENOSPC: bypass, funding released, unrelated payload and failed media preserved"
    );
}

async fn native_inode_exhaustion(mount: &std::path::Path) {
    let failing = mount.join("no-inodes");
    let prepared = PreparedDiskCache::prepare(&failing, (16 * MIB) as u64).unwrap();
    let observer = prepared.status();
    let inodes = mount.join("inode-filler");
    std::fs::create_dir(&inodes).unwrap();
    let mut count = 0;
    let failure = loop {
        match std::fs::File::create(inodes.join(count.to_string())) {
            Ok(_) => count += 1,
            Err(error) => break error,
        }
    };
    assert_eq!(failure.raw_os_error(), Some(libc::ENOSPC));
    let space = rustix::fs::statvfs(mount).unwrap();
    assert_eq!(
        space.f_ffree, 0,
        "the native fixture did not exhaust inodes"
    );
    assert!(
        space.f_bavail * space.f_frsize > (16 * MIB) as u64,
        "must retain payload block capacity"
    );
    println!(
        "ext4 inode fixture: available_bytes={} free_inodes={}",
        space.f_bavail * space.f_frsize,
        space.f_ffree
    );
    let resources = Budget::new(32 * MIB).unwrap();
    assert!(matches!(
        DiskCache::open(prepared, &resources).await,
        Err(error::Error::Fs(libc::ENOSPC))
    ));
    assert_eq!(observer.state(), State::Bypass);
    assert_eq!(resources.used(), 0);
    assert!(!failing.join("payload-v1.cache").exists());
    assert!(failing.join(".lock").exists());
    std::fs::File::open(failing.join(".lock"))
        .unwrap()
        .try_lock()
        .unwrap();
    assert_eq!(
        std::fs::read_dir(&inodes).unwrap().count(),
        count,
        "no automatic inventory cleanup"
    );
    std::fs::remove_dir_all(inodes).unwrap();
    let cache = open(&failing, &resources).await;
    cache.close(DEADLINE).await.unwrap();
    println!(
        "native inode ENOSPC: {count} filler files preserved, no data file created, retry after owned cleanup succeeds"
    );
}

#[test]
#[ignore = "internal unprivileged child of native_filesystem_failure_paths"]
fn native_permission_failure_child() {
    use std::os::unix::fs::PermissionsExt;
    support::assert_private_mount_namespace();
    let path = std::path::PathBuf::from(std::env::var_os("GEESEFS_L2_PERMISSION_DIR").unwrap());
    assert_eq!(rustix::process::geteuid().as_raw(), 65534);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let resources = Budget::new(32 * MIB).unwrap();
        let cache = open(&path, &resources).await;
        assert_eq!(cache.try_demote(key(0, 9), block(b"preserved")), Demotion::Admitted);
        cache.drain(DEADLINE).await.unwrap();
        cache.close(DEADLINE).await.unwrap();
        drop(cache);
        let media = path.join("payload-v1.cache");
        let before = std::fs::read(&media).unwrap();
        std::fs::set_permissions(&media, std::fs::Permissions::from_mode(0o400)).unwrap();
        let prepared = PreparedDiskCache::prepare(&path, (16 * MIB) as u64).unwrap();
        let observer = prepared.status();
        assert!(matches!(DiskCache::open(prepared, &resources).await, Err(error::Error::Fs(libc::EACCES))));
        assert_eq!(observer.state(), State::Bypass);
        assert_eq!(resources.used(), 0);
        assert_eq!(std::fs::read(&media).unwrap(), before);
        std::fs::set_permissions(&media, std::fs::Permissions::from_mode(0o600)).unwrap();
        let cache = open(&path, &resources).await;
        assert_eq!(cache.read_block(&key(0, 9), &Budget::new(9).unwrap()).await.unwrap().as_slice(), b"preserved");
        cache.close(DEADLINE).await.unwrap();
        println!("native EACCES as uid65534: bypass, funding released, full existing media preserved and readable after permission restore");
    });
}
