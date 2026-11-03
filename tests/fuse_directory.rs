//! Explicit native tests: never silently pass when a requested FUSE mount fails.
mod support;

use geesefs_rs::{filesystem::Filesystem, fuse::Fuse};
use std::{
    path::Path,
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};
use support::MemoryStore;

fn mounted(fs: Arc<Filesystem>, check: impl FnOnce(&Path)) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let adapter = Fuse::new(fs, runtime.handle().clone());
    let notifier = adapter.notifier.clone();
    let shutdown_error = adapter.shutdown_error.clone();
    let mut options = fuser::Config::default();
    let threads = std::env::var("GEESEFS_TEST_FUSE_THREADS")
        .map(|value| value.parse::<usize>().expect("native fixture loop count"))
        .unwrap_or(1);
    options.n_threads = Some(threads);
    options.clone_fd = threads > 1;
    options
        .mount_options
        .push(fuser::MountOption::DefaultPermissions);
    let session = fuser::Session::new(adapter, directory.path(), &options)
        .expect("explicitly requested native FUSE mount must be available");
    *notifier.lock().unwrap() = Some(session.notifier());
    let session = session.spawn().unwrap();
    let checked =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| check(directory.path())));
    session
        .umount_and_join()
        .expect("owned FUSE session must stop cleanly");
    if let Err(panic) = checked {
        std::panic::resume_unwind(panic);
    }
    assert!(shutdown_error.lock().unwrap().is_none());
}

#[test]
#[ignore = "requires an explicitly authorized Linux FUSE fixture"]
fn concurrent_receivers_preserve_lookup_read_write_and_unmount_bytes() {
    use std::io::Write;

    let store = MemoryStore::new();
    store.seed("shared", b"independent read bytes");
    mounted(store.filesystem(32), |path| {
        std::thread::scope(|scope| {
            let start = Arc::new(std::sync::Barrier::new(4));
            let mut threads = Vec::new();
            for worker in 0..4 {
                let start = start.clone();
                threads.push(scope.spawn(move || {
                    start.wait();
                    for generation in 0..16 {
                        assert_eq!(
                            std::fs::read(path.join("shared")).unwrap(),
                            b"independent read bytes"
                        );
                        let names = std::fs::read_dir(path)
                            .unwrap()
                            .map(|entry| entry.unwrap().file_name())
                            .collect::<Vec<_>>();
                        assert!(names.iter().any(|name| name == "shared"));
                        let file = path.join(format!("worker-{worker}"));
                        let renamed = path.join(format!("renamed-{worker}"));
                        let bytes = format!("worker {worker}, generation {generation}");
                        let mut handle = std::fs::File::create(&file).unwrap();
                        handle.write_all(bytes.as_bytes()).unwrap();
                        handle.sync_all().unwrap();
                        drop(handle);
                        std::fs::rename(&file, &renamed).unwrap();
                        assert_eq!(std::fs::read(&renamed).unwrap(), bytes.as_bytes());
                        std::fs::remove_file(&renamed).unwrap();
                    }
                    std::fs::write(
                        path.join(format!("durable-{worker}")),
                        b"durable after unmount",
                    )
                    .unwrap();
                }));
            }
            for thread in threads {
                thread.join().unwrap();
            }
        });
    });
    for worker in 0..4 {
        assert_eq!(
            store.bytes(&format!("durable-{worker}")),
            b"durable after unmount"
        );
        assert!(
            !store
                .objects
                .lock()
                .unwrap()
                .contains_key(&format!("worker-{worker}"))
        );
        assert!(
            !store
                .objects
                .lock()
                .unwrap()
                .contains_key(&format!("renamed-{worker}"))
        );
    }
}

#[test]
#[ignore = "requires an explicitly authorized Linux FUSE fixture"]
fn a_contended_inode_read_does_not_block_unrelated_receiver_work() {
    use std::{io::Read, os::unix::fs::MetadataExt, sync::mpsc};

    let store = MemoryStore::new();
    store.seed("slow", b"independent slow bytes");
    store.seed("fast", b"independent fast bytes");
    let mut config = MemoryStore::config(32);
    config.read_only_direct_io = true;
    config.mount_options.push("ro".into());
    let fs = Filesystem::new(config, store).unwrap();
    mounted(fs.clone(), |path| {
        let mut slow = std::fs::File::open(path.join("slow")).unwrap();
        let fast = path.join("fast");
        std::fs::metadata(&fast).unwrap();
        let node = fs.node(slow.metadata().unwrap().ino()).unwrap();
        // Model a preempted owner of a real inode critical section. The unrelated
        // xattr needs no inode/backend lock; it must still reach the receiver.
        let guard = node.state.lock().unwrap();
        std::thread::scope(|scope| {
            let (thread_sender, thread_receiver) = mpsc::channel();
            let reader = scope.spawn(move || {
                thread_sender
                    .send(std::fs::read_link("/proc/thread-self").unwrap())
                    .unwrap();
                let mut bytes = [0_u8; b"independent slow bytes".len()];
                slow.read_exact(&mut bytes).unwrap();
                assert_eq!(&bytes, b"independent slow bytes");
            });
            let thread = thread_receiver
                .recv_timeout(Duration::from_secs(2))
                .unwrap();
            let wchan = Path::new("/proc").join(thread).join("wchan");
            let deadline = Instant::now() + Duration::from_secs(2);
            let queued = loop {
                let state = std::fs::read_to_string(&wchan).unwrap_or_default();
                if state.contains("request_wait") || state.contains("fuse") {
                    break true;
                }
                if Instant::now() >= deadline {
                    break false;
                }
                std::thread::yield_now();
            };
            let (sender, receiver) = mpsc::channel();
            let probe = scope.spawn(move || {
                let mut value = [0_u8; 128];
                sender
                    .send(
                        rustix::fs::getxattr(&fast, "geesefs", &mut value[..])
                            .map(|length| value[..length].to_vec()),
                    )
                    .unwrap();
            });
            let result = receiver.recv_timeout(Duration::from_secs(2));
            drop(guard);
            reader.join().unwrap();
            probe.join().unwrap();
            assert!(
                queued,
                "fixture must observe the slow FUSE request queued in the kernel"
            );
            assert_eq!(
                result.unwrap().unwrap(),
                env!("CARGO_PKG_VERSION").as_bytes()
            );
        });
    });
}

#[test]
#[ignore = "requires an explicitly authorized Linux FUSE fixture"]
fn native_xattr_control_names_and_disabled_attributes_preserve_errors() {
    use std::os::unix::ffi::OsStrExt;
    for disabled in [false, true] {
        let store = MemoryStore::new();
        store.seed("file", b"unchanged bytes");
        let mut config = MemoryStore::config(16);
        config.disable_xattr = disabled;
        let reserved = format!("user.{}", config.symlink_attr);
        mounted(Filesystem::new(config, store.clone()).unwrap(), |path| {
            let file = path.join("file");
            let mut value = [0_u8; 128];
            assert_eq!(
                rustix::fs::getxattr(&file, "geesefs", &mut value[..0]).unwrap(),
                env!("CARGO_PKG_VERSION").len()
            );
            assert_eq!(
                rustix::fs::getxattr(&file, reserved.as_str(), &mut value[..]),
                Err(rustix::io::Errno::NODATA)
            );
            assert_eq!(
                rustix::fs::getxattr(
                    &file,
                    std::ffi::OsStr::from_bytes(b"user.\xff"),
                    &mut value[..]
                ),
                Err(rustix::io::Errno::ILSEQ)
            );
            let expected = if disabled {
                rustix::io::Errno::OPNOTSUPP
            } else {
                rustix::io::Errno::NODATA
            };
            assert_eq!(
                rustix::fs::getxattr(&file, "user.missing", &mut value[..]),
                Err(expected)
            );
        });
        assert_eq!(store.bytes("file"), b"unchanged bytes");
    }
}

#[test]
#[ignore = "requires an explicitly authorized Linux FUSE fixture"]
fn plus_pages_deliver_attributes_and_lookup_ownership_without_overlapping_lists() {
    let store = MemoryStore::new();
    for number in 0..8192 {
        store.seed(&format!("{number:04}"), b"independent bytes");
    }
    let fs = store.filesystem(16);
    mounted(fs.clone(), |path| {
        let mut names = std::fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(
            names,
            (0..8192).map(|n| format!("{n:04}")).collect::<Vec<_>>()
        );
        let references = fs
            .namespace
            .lock()
            .unwrap()
            .nodes
            .values()
            .map(|node| (node.ino, node.state.lock().unwrap().lookups))
            .collect::<Vec<_>>();
        assert_eq!(references.iter().filter(|(ino, _)| *ino != 1).count(), 8192);
        assert!(
            references.iter().all(|(_, count)| *count == 1),
            "READDIRPLUS must deliver exactly one lookup reference per entry"
        );
        assert_eq!(
            fs.node(1).unwrap().state.lock().unwrap().lookups,
            1,
            "Linux does not acquire lookup references for dot entries"
        );
        assert_eq!(
            store.list_calls.load(Ordering::Relaxed),
            10, // Nine <=1000-name backend pages plus EOF, despite smaller FUSE frames.
            "kernel frames must not split backend pages into overlapping LIST requests"
        );
        for name in &names {
            assert_eq!(std::fs::metadata(path.join(name)).unwrap().len(), 17);
        }
        assert_eq!(
            store.head_starts.load(Ordering::Relaxed),
            0,
            "authoritative listing attributes must not cause per-name HEADs"
        );
    });
}

#[test]
#[ignore = "requires an explicitly authorized Linux FUSE fixture"]
fn failed_plus_buffer_does_not_transfer_unsent_lookup_references() {
    let store = MemoryStore::new();
    for number in 0..5 {
        store.seed(&format!("{number:04}"), b"data");
    }
    store.seed("z-directory/child", b"data");
    *store.failed_list_prefix.lock().unwrap() = Some("z-directory/".into());
    let fs = store.filesystem(16);
    mounted(fs.clone(), |path| {
        let result = std::fs::read_dir(path)
            .unwrap()
            .collect::<std::io::Result<Vec<_>>>();
        assert!(
            result.is_err(),
            "a failed metadata fetch must not publish a partial PLUS buffer"
        );
        let references = fs
            .namespace
            .lock()
            .unwrap()
            .nodes
            .values()
            .filter(|node| node.ino != 1)
            .map(|node| node.state.lock().unwrap().lookups)
            .collect::<Vec<_>>();
        assert!(!references.is_empty());
        assert!(
            references.iter().all(|count| *count == 0),
            "no reference may escape an errored reply, even for earlier buffered names"
        );
    });
}

#[test]
#[ignore = "requires an explicitly authorized Linux FUSE fixture"]
fn a_cached_lookup_does_not_restart_the_kernel_attribute_ttl() {
    let store = MemoryStore::new();
    store.seed("file", b"old");
    let mut config = MemoryStore::config(16);
    config.stat_cache_ttl = Duration::from_secs(4);
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let node = runtime.block_on(fs.lookup(1, "file")).unwrap();
    mounted(fs.clone(), |path| {
        node.state.lock().unwrap().fetched = Instant::now() - Duration::from_secs(3);
        let started = Instant::now();
        assert_eq!(std::fs::metadata(path.join("file")).unwrap().len(), 3);
        store.seed("file", b"new generation");
        // The real kernel's cache cannot use a mocked Rust clock. Wait past its
        // remaining 1s, but well before the former restarted 4s TTL would expire.
        std::thread::sleep(Duration::from_millis(1100));
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "fixture scheduling exceeded the TTL discrimination window"
        );
        assert_eq!(std::fs::metadata(path.join("file")).unwrap().len(), 14);
    });
}

#[test]
#[ignore = "requires an explicitly authorized Linux FUSE fixture"]
fn a_standard_readdir_does_not_hide_changes_past_the_original_attribute_ttl() {
    use std::os::unix::fs::MetadataExt;

    let store = MemoryStore::new();
    store.standard_listing.store(true, Ordering::Relaxed);
    store.seed("file", b"old");
    store
        .objects
        .lock()
        .unwrap()
        .get_mut("file")
        .unwrap()
        .metadata
        .insert("mode".into(), (libc::S_IFREG | 0o640).to_string());
    let mut config = MemoryStore::config(16);
    config.stat_cache_ttl = Duration::from_secs(4);
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let node = runtime.block_on(fs.lookup(1, "file")).unwrap();
    mounted(fs, |path| {
        node.state.lock().unwrap().fetched = Instant::now() - Duration::from_secs(3);
        let started = Instant::now();
        store
            .objects
            .lock()
            .unwrap()
            .get_mut("file")
            .unwrap()
            .metadata
            .insert("mode".into(), (libc::S_IFREG | 0o600).to_string());
        let names = std::fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(names, ["file"]);
        assert_eq!(
            std::fs::metadata(path.join("file")).unwrap().mode() & 0o7777,
            0o640
        );
        std::thread::sleep(Duration::from_millis(1100));
        assert!(started.elapsed() < Duration::from_secs(3));
        assert_eq!(
            std::fs::metadata(path.join("file")).unwrap().mode() & 0o7777,
            0o600
        );
    });
}

#[test]
#[ignore = "requires an explicitly authorized Linux FUSE fixture"]
fn exhausted_file_handles_reject_create_before_namespace_mutation() {
    let store = MemoryStore::new();
    store.seed("source", b"original bytes");
    let mut config = MemoryStore::config(16);
    config.entry_limit = 8;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    mounted(fs, |path| {
        let mut files = (0..8)
            .map(|_| std::fs::File::open(path.join("source")).unwrap())
            .collect::<Vec<_>>();
        let error = std::fs::File::create(path.join("new")).unwrap_err();
        assert_eq!(error.raw_os_error(), Some(libc::EMFILE));
        assert!(
            !path.join("new").exists(),
            "handle admission must precede creating a dirty namespace entry"
        );
        files.pop();
        let mut created = std::fs::File::create(path.join("new")).unwrap();
        std::io::Write::write_all(&mut created, b"committed after release").unwrap();
        created.sync_all().unwrap();
    });
    assert_eq!(store.bytes("source"), b"original bytes");
    assert_eq!(store.bytes("new"), b"committed after release");
}

#[test]
#[ignore = "requires an explicitly authorized Linux FUSE fixture"]
fn namespace_pressure_does_not_reclaim_or_reference_unsent_plus_entries() {
    let store = MemoryStore::new();
    for number in 0..12 {
        store.seed(&format!("{number:04}"), b"data");
    }
    let mut config = MemoryStore::config(16);
    config.entry_limit = 8;
    let fs = Filesystem::new(config, store).unwrap();
    mounted(fs.clone(), |path| {
        let error = std::fs::read_dir(path)
            .unwrap()
            .collect::<std::io::Result<Vec<_>>>()
            .unwrap_err();
        assert_eq!(error.raw_os_error(), Some(libc::ENOMEM));
        let namespace = fs.namespace.lock().unwrap();
        assert!(namespace.nodes.len() <= 8);
        assert!(
            namespace
                .nodes
                .values()
                .filter(|node| node.ino != 1)
                .all(|node| node.state.lock().unwrap().lookups == 0),
            "an unsent buffer must neither acquire references nor evade the inode bound"
        );
    });
}

#[test]
#[ignore = "requires an explicitly authorized Linux FUSE fixture"]
fn plus_common_prefixes_preserve_directory_marker_permissions_and_ownership() {
    use std::os::unix::fs::MetadataExt;

    let store = MemoryStore::new();
    store.seed("directory/", b"");
    store.seed("directory/child", b"data");
    store
        .objects
        .lock()
        .unwrap()
        .get_mut("directory/")
        .unwrap()
        .metadata
        .extend([
            ("mode".into(), (libc::S_IFDIR | 0o750).to_string()),
            ("uid".into(), "1234".into()),
            ("gid".into(), "5678".into()),
        ]);
    let fs = store.filesystem(16);
    mounted(fs, |path| {
        let names = std::fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(names, ["directory"]);
        let metadata = std::fs::metadata(path.join("directory")).unwrap();
        assert_eq!(metadata.mode() & 0o7777, 0o750);
        assert_eq!((metadata.uid(), metadata.gid()), (1234, 5678));
    });
    assert_eq!(store.head_starts.load(Ordering::Relaxed), 1);
}

#[test]
#[ignore = "requires an explicitly authorized Linux FUSE fixture"]
fn unsupported_hardlink_emulation_fails_before_creating_an_alias() {
    let store = MemoryStore::new();
    store.seed("source", b"original bytes");
    let mut config = MemoryStore::config(16);
    config.emulate_hardlinks_as_symlinks = true;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    mounted(fs, |path| {
        let error = std::fs::hard_link(path.join("source"), path.join("alias")).unwrap_err();
        assert_eq!(
            std::fs::symlink_metadata(path.join("alias"))
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::NotFound,
            "a rejected Linux LINK must not leave a symlink behind"
        );
        assert_eq!(error.raw_os_error(), Some(libc::EOPNOTSUPP));
        assert_eq!(
            std::fs::read(path.join("source")).unwrap(),
            b"original bytes"
        );
    });
    assert!(!store.objects.lock().unwrap().contains_key("alias"));
}

#[test]
#[ignore = "requires an explicitly authorized Linux FUSE fixture"]
fn native_writer_metadata_xattrs_links_and_removal_preserve_committed_bytes() {
    use rustix::fs::{FallocateFlags, Mode, XattrFlags};
    use std::{
        io::{Read, Seek, SeekFrom, Write},
        os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
        time::UNIX_EPOCH,
    };

    let store = MemoryStore::new();
    let mut config = MemoryStore::config(16);
    config.enable_specials = true;
    config.fsync_on_close = true;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    mounted(fs, |path| {
        let directory = path.join("directory");
        std::fs::create_dir(&directory).unwrap();
        let source = directory.join("file");
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&source)
            .unwrap();
        file.write_all(b"independent full-byte proof").unwrap();
        file.sync_all().unwrap();
        assert_eq!(
            store.bytes("directory/file"),
            b"independent full-byte proof"
        );
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o640)).unwrap();
        std::os::unix::fs::chown(&source, Some(1234), Some(5678)).unwrap();
        let modified = UNIX_EPOCH + Duration::from_secs(1_700_000_123);
        file.set_modified(modified).unwrap();
        let metadata = file.metadata().unwrap();
        assert_eq!(metadata.mode() & 0o7777, 0o640);
        assert_eq!((metadata.uid(), metadata.gid()), (1234, 5678));
        assert_eq!(metadata.modified().unwrap(), modified);

        let attribute = [0, 255, 83];
        rustix::fs::setxattr(&source, "user.fixture", &attribute, XattrFlags::CREATE).unwrap();
        assert_eq!(
            rustix::fs::setxattr(&source, "user.fixture", b"other", XattrFlags::CREATE),
            Err(rustix::io::Errno::EXIST)
        );
        let mut short = [0_u8; 2];
        assert_eq!(
            rustix::fs::getxattr(&source, "user.fixture", &mut short[..]),
            Err(rustix::io::Errno::RANGE)
        );
        let mut value = [0_u8; 16];
        let length = rustix::fs::getxattr(&source, "user.fixture", &mut value[..]).unwrap();
        assert_eq!(&value[..length], &attribute);
        let mut names = [0_u8; 1024];
        let length = rustix::fs::listxattr(&source, &mut names[..]).unwrap();
        assert!(
            names[..length]
                .split(|&byte| byte == 0)
                .any(|name| name == b"user.fixture")
        );
        rustix::fs::removexattr(&source, "user.fixture").unwrap();
        assert_eq!(
            rustix::fs::getxattr(&source, "user.fixture", &mut value[..]),
            Err(rustix::io::Errno::NODATA)
        );

        rustix::fs::fallocate(&file, FallocateFlags::empty(), 0, 8192).unwrap();
        file.set_len(4096).unwrap();
        rustix::fs::fallocate(&file, FallocateFlags::KEEP_SIZE, 8192, 4096).unwrap();
        assert_eq!(file.metadata().unwrap().len(), 4096);
        assert_eq!(
            rustix::fs::fallocate(
                &file,
                FallocateFlags::KEEP_SIZE | FallocateFlags::PUNCH_HOLE,
                0,
                4096
            ),
            Err(rustix::io::Errno::OPNOTSUPP)
        );
        let mut expected = b"independent full-byte proof".to_vec();
        expected.resize(4096, 0);
        file.seek(SeekFrom::Start(0)).unwrap();
        let mut read = Vec::new();
        file.read_to_end(&mut read).unwrap();
        assert_eq!(read, expected);
        file.sync_all().unwrap();
        assert_eq!(store.bytes("directory/file"), expected);
        drop(file);

        let moved = directory.join("moved");
        std::fs::rename(&source, &moved).unwrap();
        let link = directory.join("link");
        std::os::unix::fs::symlink("moved", &link).unwrap();
        assert_eq!(std::fs::read_link(&link).unwrap(), Path::new("moved"));
        assert_eq!(std::fs::read(&link).unwrap(), expected);
        let fifo = directory.join("fifo");
        rustix::fs::mkfifoat(rustix::fs::CWD, &fifo, Mode::from_bits_truncate(0o600)).unwrap();
        assert!(
            std::fs::symlink_metadata(&fifo)
                .unwrap()
                .file_type()
                .is_fifo()
        );
        std::fs::File::open(&directory).unwrap().sync_all().unwrap();
        assert_eq!(store.bytes("directory/moved"), expected);
        assert_eq!(
            std::fs::remove_dir(&directory).unwrap_err().kind(),
            std::io::ErrorKind::DirectoryNotEmpty
        );
        std::fs::remove_file(&link).unwrap();
        std::fs::remove_file(&fifo).unwrap();
        std::fs::remove_file(&moved).unwrap();
        std::fs::remove_dir(&directory).unwrap();
    });
    assert!(store.objects.lock().unwrap().is_empty());
}

#[test]
#[ignore = "requires an explicitly authorized Linux FUSE fixture"]
fn fsync_on_close_commits_dirty_bytes_before_the_mount_is_destroyed() {
    let store = MemoryStore::new();
    let mut config = MemoryStore::config(16);
    config.fsync_on_close = true;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    mounted(fs, |path| {
        std::fs::write(path.join("file"), b"durable close boundary").unwrap();
        assert_eq!(store.bytes("file"), b"durable close boundary");
    });
}
