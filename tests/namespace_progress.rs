#![forbid(unsafe_code)]
mod support;

use geesefs_rs::filesystem::Filesystem;
use std::{sync::mpsc, time::Duration};
use support::MemoryStore;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

// Release the held dependency and join even when the progress assertion fails.
fn while_held<T: Send + 'static>(
    operation: impl FnOnce() -> T + Send + 'static,
    release: impl FnOnce(),
) -> T {
    let (sent, received) = mpsc::channel();
    let thread = std::thread::spawn(move || sent.send(operation()).unwrap());
    let result = received.recv_timeout(Duration::from_secs(2));
    release();
    thread.join().unwrap();
    result.expect("unrelated namespace/inode work must not block progress")
}

#[test]
fn inode_resolution_progresses_during_namespace_maintenance_in_ro_and_rw() {
    for readonly in [false, true] {
        let store = MemoryStore::new();
        store.seed("file", b"payload");
        let mut config = MemoryStore::config(32);
        if readonly {
            config.mount_options.push("ro".into());
        }
        let fs = Filesystem::new(config, store).unwrap();
        let node = runtime().block_on(fs.lookup(1, "file")).unwrap();
        let ino = node.ino;
        let namespace = fs.namespace.lock().unwrap();
        let reader = fs.clone();
        let resolved = while_held(move || reader.node(ino).unwrap(), || drop(namespace));
        assert!(std::sync::Arc::ptr_eq(&node, &resolved));
    }
}

#[test]
fn rw_flush_ignores_unrelated_clean_inode_and_persists_dirty_payload() {
    let store = MemoryStore::new();
    store.seed("clean", b"original");
    let fs = store.filesystem(32);
    let clean = runtime().block_on(fs.lookup(1, "clean")).unwrap();
    let dirty = runtime()
        .block_on(fs.create(1, "dirty", libc::S_IFREG | 0o644, None, 0))
        .unwrap();
    runtime()
        .block_on(fs.write(dirty.ino, 0, b"new bytes"))
        .unwrap();
    let held = clean.state.lock().unwrap();
    let flusher = fs.clone();
    while_held(
        move || runtime().block_on(flusher.flush_all()),
        || drop(held),
    )
    .unwrap();
    assert_eq!(store.bytes("dirty"), b"new bytes");
    assert_eq!(store.bytes("clean"), b"original");
    assert_eq!(
        store.put_calls.load(std::sync::atomic::Ordering::Relaxed),
        1
    );
}

#[test]
fn expired_parent_listing_does_not_inspect_clean_grandchild_state() {
    let store = MemoryStore::new();
    store.seed("dir/a/child", b"nested");
    store.seed("dir/b", b"direct");
    let mut config = MemoryStore::config(32);
    config.stat_cache_ttl = Duration::ZERO;
    let fs = Filesystem::new(config, store).unwrap();
    let dir = runtime().block_on(fs.lookup(1, "dir")).unwrap();
    let a = runtime().block_on(fs.lookup(dir.ino, "a")).unwrap();
    let child = runtime().block_on(fs.lookup(a.ino, "child")).unwrap();
    let held = child.state.lock().unwrap();
    let listing = fs.clone();
    let entries = while_held(
        move || runtime().block_on(listing.directory_chunk(dir.ino, "", 8)),
        || drop(held),
    )
    .unwrap();
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
}

#[test]
fn rw_rename_ignores_unrelated_clean_inode_state() {
    let store = MemoryStore::new();
    store.seed("unrelated", b"untouched");
    store.seed("source", b"moved bytes");
    let fs = store.filesystem(32);
    let clean = runtime().block_on(fs.lookup(1, "unrelated")).unwrap();
    let held = clean.state.lock().unwrap();
    let writer = fs.clone();
    while_held(
        move || runtime().block_on(writer.rename(1, "source", 1, "destination")),
        || drop(held),
    )
    .unwrap();
    assert_eq!(store.bytes("destination"), b"moved bytes");
    assert_eq!(store.bytes("unrelated"), b"untouched");
    assert!(!store.objects.lock().unwrap().contains_key("source"));
}

#[tokio::test]
async fn flush_all_tracks_every_writer_mutation() {
    for mutation in ["write", "truncate", "setattr", "setxattr", "removexattr"] {
        let store = MemoryStore::new();
        store.seed("file", b"abcdef");
        store
            .objects
            .lock()
            .unwrap()
            .get_mut("file")
            .unwrap()
            .metadata
            .insert("label".into(), "old".into());
        let fs = store.filesystem(32);
        let file = fs.lookup(1, "file").await.unwrap();
        match mutation {
            "write" => {
                fs.write(file.ino, 0, b"payload").await.unwrap();
            }
            "truncate" => fs.truncate(file.ino, 3).unwrap(),
            "setattr" => {
                fs.setattr(file.ino, Some(0o600), None, None, None).unwrap();
            }
            "setxattr" => fs
                .setxattr(file.ino, "user.label", Some(b"new"), 0)
                .unwrap(),
            "removexattr" => fs.setxattr(file.ino, "user.label", None, 0).unwrap(),
            _ => unreachable!(),
        }
        fs.flush_all().await.unwrap();
        let expected: &[u8] = match mutation {
            "write" => b"payload",
            "truncate" => b"abc",
            _ => b"abcdef",
        };
        assert_eq!(store.bytes("file"), expected, "{mutation}");
        let objects = store.objects.lock().unwrap();
        let metadata = &objects["file"].metadata;
        match mutation {
            "setattr" => assert_eq!(metadata["mode"], "33152"),
            "setxattr" => assert_eq!(metadata["label"], "new"),
            "removexattr" => assert!(!metadata.contains_key("label")),
            _ => {}
        }
    }
}

#[tokio::test]
async fn failed_and_cancelled_flushes_retain_newer_generations_for_retry() {
    for cancel in [false, true] {
        let store = MemoryStore::new();
        let fs = store.filesystem(32);
        let node = fs
            .create(1, "file", libc::S_IFREG | 0o644, None, 0)
            .await
            .unwrap();
        fs.write(node.ino, 0, b"initial").await.unwrap();
        if cancel {
            let (seen, _release) = store.pause_put();
            let task = tokio::spawn({
                let fs = fs.clone();
                async move { fs.flush_all().await }
            });
            tokio::time::timeout(Duration::from_secs(5), seen)
                .await
                .unwrap()
                .unwrap();
            fs.write(node.ino, 0, b"new bytes").await.unwrap();
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
        } else {
            store
                .fail_put
                .store(true, std::sync::atomic::Ordering::Relaxed);
            assert!(fs.flush_all().await.is_err());
            fs.write(node.ino, 0, b"new bytes").await.unwrap();
            store
                .fail_put
                .store(false, std::sync::atomic::Ordering::Relaxed);
        }
        fs.flush_all().await.unwrap();
        assert_eq!(store.bytes("file"), b"new bytes");
        let puts = store.put_calls.load(std::sync::atomic::Ordering::Relaxed);
        fs.flush_all().await.unwrap();
        assert_eq!(
            store.put_calls.load(std::sync::atomic::Ordering::Relaxed),
            puts
        );
    }
}

#[tokio::test]
async fn write_during_successful_flush_is_not_lost_from_dirty_index() {
    let store = MemoryStore::new();
    let fs = store.filesystem(32);
    let node = fs
        .create(1, "file", libc::S_IFREG | 0o644, None, 0)
        .await
        .unwrap();
    fs.write(node.ino, 0, b"initial").await.unwrap();
    let (seen, release) = store.pause_put();
    let task = tokio::spawn({
        let fs = fs.clone();
        async move { fs.flush_all().await }
    });
    tokio::time::timeout(Duration::from_secs(5), seen)
        .await
        .unwrap()
        .unwrap();
    fs.write(node.ino, 0, b"new bytes").await.unwrap();
    release.send(()).unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(store.bytes("file"), b"initial");
    fs.flush_all().await.unwrap();
    assert_eq!(store.bytes("file"), b"new bytes");
}

#[tokio::test]
async fn dirty_descendant_is_rekeyed_by_directory_rename_and_removed_by_unlink() {
    let store = MemoryStore::new();
    store.seed("left/old/", b"");
    let mut config = MemoryStore::config(32);
    config.stat_cache_ttl = Duration::ZERO;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let left = fs.lookup(1, "left").await.unwrap();
    let dir = fs.lookup(left.ino, "old").await.unwrap();
    let file = fs
        .create(dir.ino, "file", libc::S_IFREG | 0o644, None, 0)
        .await
        .unwrap();
    fs.write(file.ino, 0, b"local child").await.unwrap();
    // The child has not been uploaded; rename must retain its dirty generation
    // under the destination, even though no remote child participates in COPY.
    fs.rename(left.ino, "old", 1, "new").await.unwrap();
    fs.flush_all().await.unwrap();
    assert_eq!(store.bytes("new/file"), b"local child");
    assert!(!store.objects.lock().unwrap().contains_key("left/old/file"));
    assert!(matches!(
        fs.refresh_inode_if_expired(left.ino).await,
        Err(geesefs_rs::error::Error::Fs(libc::ENOENT))
    ));
    fs.write(file.ino, 0, b"discarded").await.unwrap();
    fs.unlink(dir.ino, "file", false).await.unwrap();
    fs.flush_all().await.unwrap();
    assert!(!store.objects.lock().unwrap().contains_key("new/file"));
}

#[tokio::test]
async fn fsync_of_deleted_identity_cannot_retire_recreated_names_dirty_generation() {
    let store = MemoryStore::new();
    store.seed("file", b"old object");
    let fs = store.filesystem(32);
    let old = fs.lookup(1, "file").await.unwrap();
    fs.unlink(1, "file", false).await.unwrap();
    let new = fs
        .create(1, "file", libc::S_IFREG | 0o644, None, 0)
        .await
        .unwrap();
    fs.write(new.ino, 0, b"new object").await.unwrap();
    assert_ne!(old.ino, new.ino);
    assert!(matches!(
        fs.fsync(old.ino).await,
        Err(geesefs_rs::error::Error::Fs(libc::ENOENT))
    ));
    fs.flush_all().await.unwrap();
    assert_eq!(store.bytes("file"), b"new object");
}

#[tokio::test]
async fn dirty_implicit_parent_survives_until_child_commit_then_expires() {
    let store = MemoryStore::new();
    store.seed("dir/seed", b"seed");
    let mut config = MemoryStore::config(32);
    config.stat_cache_ttl = Duration::ZERO;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let dir = fs.lookup(1, "dir").await.unwrap();
    store.objects.lock().unwrap().remove("dir/seed");
    let file = fs
        .create(dir.ino, "file", libc::S_IFREG | 0o644, None, 0)
        .await
        .unwrap();
    assert_eq!(
        fs.refresh_inode_if_expired(dir.ino).await.unwrap().ino,
        dir.ino
    );
    fs.flush_all().await.unwrap();
    assert_eq!(store.bytes("dir/file"), b"");
    store.objects.lock().unwrap().remove("dir/file");
    assert!(matches!(
        fs.refresh_inode_if_expired(dir.ino).await,
        Err(geesefs_rs::error::Error::Fs(libc::ENOENT))
    ));
    assert!(fs.node(file.ino).is_ok());
}

#[tokio::test]
async fn local_pages_seek_past_subtrees_without_skipping_lexical_neighbours() {
    let store = MemoryStore::new();
    store.seed("a/child/leaf", b"nested");
    for name in ["a!", "a.", "a0", "aa", "β", "😀"] {
        store.seed(name, b"direct");
    }
    let fs = store.filesystem(32);
    let a = fs.lookup(1, "a").await.unwrap();
    let child = fs.lookup(a.ino, "child").await.unwrap();
    fs.lookup(child.ino, "leaf").await.unwrap();
    for name in ["a!", "a.", "a0", "aa", "β", "😀"] {
        fs.lookup(1, name).await.unwrap();
    }
    // Fresh local names remain visible for their TTL even if the remote listing
    // no longer contains them. This exercises the overlay, not remote sorting.
    store.objects.lock().unwrap().clear();
    let mut after = String::new();
    let mut names = Vec::new();
    loop {
        let page = fs.directory_chunk(1, &after, 2).await.unwrap();
        if page.is_empty() {
            break;
        }
        assert!(page.len() <= 2);
        after = page.last().unwrap().name.as_str().to_owned();
        names.extend(page.into_iter().map(|entry| entry.name));
    }
    assert_eq!(names, ["a", "a!", "a.", "a0", "aa", "β", "😀"]);
    assert!(fs.directory_chunk(1, "", 0).await.unwrap().is_empty());
}

#[test]
fn lock_free_resolver_retains_owners_and_never_reuses_evicted_identity() {
    let store = MemoryStore::new();
    for name in ["a", "b", "c"] {
        store.seed(name, name.as_bytes());
    }
    let mut config = MemoryStore::config(32);
    config.entry_limit = 2;
    let fs = Filesystem::new(config, store).unwrap();
    let a = runtime().block_on(fs.lookup(1, "a")).unwrap();
    let ino = a.ino;
    let reader = fs.node(ino).unwrap();
    drop(a);
    assert!(matches!(
        runtime().block_on(fs.lookup(1, "b")),
        Err(geesefs_rs::error::Error::Fs(libc::ENOMEM))
    ));
    assert_eq!(reader.attr().ino.0, ino);
    drop(reader);
    let b = runtime().block_on(fs.lookup(1, "b")).unwrap();
    assert!(matches!(
        fs.node(ino),
        Err(geesefs_rs::error::Error::Fs(libc::ENOENT))
    ));
    assert_ne!(b.ino, ino);
    let reader_fs = fs.clone();
    let held = fs.namespace.lock().unwrap();
    let returned = while_held(move || reader_fs.node(b.ino).unwrap(), || drop(held));
    assert_eq!(returned.attr().size, 1);
}

#[tokio::test]
async fn expired_wide_directory_yields_before_finishing_its_local_scan() {
    use std::{
        future::{Future, poll_fn},
        task::Poll,
    };
    let store = MemoryStore::new();
    for index in 0..512 {
        store.seed(&format!("file-{index:04}"), b"cached");
    }
    let mut config = MemoryStore::config(32);
    config.stat_cache_ttl = Duration::ZERO;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    for index in 0..512 {
        fs.lookup(1, &format!("file-{index:04}")).await.unwrap();
    }
    store.objects.lock().unwrap().clear();
    let mut listing = std::pin::pin!(fs.directory_chunk(1, "", 8));
    assert!(
        poll_fn(|cx| Poll::Ready(listing.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    assert!(listing.await.unwrap().is_empty());
}

#[tokio::test]
async fn optional_preload_cannot_finish_a_full_namespace_scan_in_one_poll() {
    use geesefs_rs::{config::ListType, inode::Inode};
    use std::{
        future::{Future, poll_fn},
        task::Poll,
    };

    let store = MemoryStore::new();
    store.seed("target", b"independent target bytes");
    let mut config = MemoryStore::config(32);
    config.list_type = Some(ListType::Extended);
    config.entry_limit = 388;
    let fs = Filesystem::new(config.clone(), store).unwrap();
    // Model a traversed directory whose real lookup references have not yet
    // been forgotten. Only the final inode is an eligible cache victim.
    kernel_owned_prefix(&fs, 386, false);
    fs.namespace
        .lock()
        .unwrap()
        .install("victim", |ino| {
            Inode::new(ino, "victim".into(), false, None, &config)
        })
        .unwrap();
    let mut lookup = std::pin::pin!(fs.lookup(1, "target"));
    assert!(
        poll_fn(|cx| Poll::Ready(lookup.as_mut().poll(cx)))
            .await
            .is_pending(),
        "optional preloading must not bypass yielding admission with a full synchronous scan"
    );
    // Other users of this runtime and the namespace must get a turn before the
    // tail victim is reached; references are not invented or cleared to pass.
    let text = geesefs_rs::metrics::render(&fs).unwrap();
    assert!(text.contains("geesefs_namespace_entries 388"));
    let target = lookup.await.unwrap();
    assert_eq!(
        fs.read(target.ino, 0, 24).await.unwrap().as_ref(),
        b"independent target bytes"
    );
    assert!(fs.namespace.lock().unwrap().named("victim").is_none());
    assert_eq!(fs.namespace.lock().unwrap().evictions, 1);
}

#[tokio::test]
async fn consecutive_admissions_reuse_reclaim_progress_past_kernel_owned_prefix() {
    use geesefs_rs::{config::ListType, inode::Inode};
    use std::{
        future::{Future, poll_fn},
        task::Poll,
    };

    let store = MemoryStore::new();
    store.seed("first", b"first bytes");
    store.seed("second", b"second bytes");
    let mut config = MemoryStore::config(32);
    config.list_type = Some(ListType::V2);
    config.entry_limit = 4100;
    let fs = Filesystem::new(config.clone(), store).unwrap();
    kernel_owned_prefix(&fs, 4096, false);
    for index in 0..3 {
        let key = format!("victim-{index}");
        fs.namespace
            .lock()
            .unwrap()
            .install(&key, |ino| {
                Inode::new(ino, key.clone(), false, None, &config)
            })
            .unwrap();
    }
    let first = fs.lookup(1, "first").await.unwrap();
    fs.reference(&first);
    let mut second = std::pin::pin!(fs.lookup(1, "second"));
    let mut admitted = None;
    // A prior admission already found this unowned tail. A few scheduler turns
    // must suffice for the next victim, independently of the old pinned prefix.
    for _ in 0..4 {
        if let Poll::Ready(result) = poll_fn(|cx| Poll::Ready(second.as_mut().poll(cx))).await {
            admitted = Some(result.unwrap());
            break;
        }
        tokio::task::yield_now().await;
    }
    let admitted =
        admitted.expect("each admission must not rescan the same long kernel-owned prefix");
    assert_eq!(
        fs.read(first.ino, 0, 11).await.unwrap().as_ref(),
        b"first bytes"
    );
    assert_eq!(
        fs.read(admitted.ino, 0, 12).await.unwrap().as_ref(),
        b"second bytes"
    );
    let ns = fs.namespace.lock().unwrap();
    assert_eq!(ns.nodes.len(), 4100);
    assert_eq!(ns.evictions, 2);
    assert!(ns.named("victim-0").is_none());
    assert!(ns.named("victim-1").is_none());
}

#[tokio::test]
async fn preload_rechecks_a_mutation_while_yielding_for_namespace_admission() {
    use geesefs_rs::{config::ListType, inode::Inode};
    use std::{
        future::{Future, poll_fn},
        task::Poll,
    };

    let store = MemoryStore::new();
    store.seed("target", b"old");
    store.seed("cached-0000/existing", b"directory child");
    let mut config = MemoryStore::config(32);
    config.list_type = Some(ListType::Extended);
    config.entry_limit = 388;
    let fs = Filesystem::new(config.clone(), store.clone()).unwrap();
    let pinned = kernel_owned_prefix(&fs, 386, true);
    fs.namespace
        .lock()
        .unwrap()
        .install("victim", |ino| {
            Inode::new(ino, "victim".into(), false, None, &config)
        })
        .unwrap();
    let mut lookup = std::pin::pin!(fs.lookup(1, "target"));
    assert!(
        poll_fn(|cx| Poll::Ready(lookup.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    // A local mutation invalidates the old LIST snapshot while the unrelated
    // target is replaced remotely. Publishing the old page would hide its HEAD.
    store.seed("target", b"new target bytes");
    let local = fs
        .create(pinned[0], "local", libc::S_IFREG | 0o600, None, 0)
        .await
        .unwrap();
    fs.write(local.ino, 0, b"local dirty bytes").await.unwrap();
    fs.forget(pinned[380], 1);
    let target = lookup.await.unwrap();
    assert_eq!(target.attr().size, 16);
    assert_eq!(
        fs.read(target.ino, 0, 16).await.unwrap().as_ref(),
        b"new target bytes"
    );
    assert_eq!(
        fs.read(local.ino, 0, 17).await.unwrap().as_ref(),
        b"local dirty bytes"
    );
    assert_eq!(fs.namespace.lock().unwrap().nodes.len(), 388);
}

fn kernel_owned_prefix(fs: &Filesystem, count: usize, first_directory: bool) -> Vec<u64> {
    use geesefs_rs::inode::Inode;
    let mut namespace = fs.namespace.lock().unwrap();
    (0..count)
        .map(|index| {
            let key = format!("cached-{index:04}");
            let node = namespace
                .install(&key, |ino| {
                    Inode::new(
                        ino,
                        key.clone(),
                        first_directory && index == 0,
                        None,
                        &fs.config,
                    )
                })
                .unwrap();
            fs.reference(&node);
            node.ino
        })
        .collect()
}

#[test]
fn concurrent_resolution_and_eviction_preserve_canonical_owners() {
    use std::sync::{
        Arc, Barrier,
        atomic::{AtomicU64, Ordering},
    };
    let store = MemoryStore::new();
    for name in ["left", "right"] {
        store.seed(name, b"payload");
    }
    let mut config = MemoryStore::config(32);
    config.entry_limit = 2;
    let fs = Filesystem::new(config, store).unwrap();
    let first = runtime().block_on(fs.lookup(1, "left")).unwrap();
    let current = AtomicU64::new(first.ino);
    drop(first);
    let barrier = Barrier::new(2);
    std::thread::scope(|scope| {
        let reader = scope.spawn(|| {
            barrier.wait();
            for _ in 0..20_000 {
                let ino = current.load(Ordering::Acquire);
                match fs.node(ino) {
                    Ok(node) => {
                        let ns = fs.namespace.lock().unwrap();
                        assert!(Arc::ptr_eq(ns.nodes.get(&ino).unwrap(), &node));
                        assert_eq!(node.attr().ino.0, ino);
                    }
                    Err(geesefs_rs::error::Error::Fs(libc::ENOENT)) => {}
                    Err(error) => panic!("unexpected resolution error: {error}"),
                }
            }
        });
        barrier.wait();
        let rt = runtime();
        for index in 0..500 {
            let name = if index % 2 == 0 { "right" } else { "left" };
            match rt.block_on(fs.lookup(1, name)) {
                Ok(node) => {
                    current.store(node.ino, Ordering::Release);
                }
                Err(geesefs_rs::error::Error::Fs(libc::ENOMEM)) => {}
                Err(error) => panic!("unexpected eviction error: {error}"),
            }
        }
        reader.join().unwrap();
    });
    assert_eq!(fs.namespace.lock().unwrap().nodes.len(), 2);
}
