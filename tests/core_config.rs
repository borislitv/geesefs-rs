#![forbid(unsafe_code)]
mod support;
use geesefs_rs::{error::Error, filesystem::Filesystem, write::MAX_IO};
use std::{sync::atomic::Ordering, time::Duration};
use support::MemoryStore;

#[tokio::test]
async fn explicit_directory_lookup_never_uses_implicit_listing() {
    let store = MemoryStore::new();
    store.seed("implicit/child", b"x");
    store.seed("explicit/", b"");
    let mut config = MemoryStore::config(32);
    config.no_implicit_dir = true;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    assert!(matches!(
        fs.lookup(1, "implicit").await,
        Err(Error::Fs(libc::ENOENT))
    ));
    assert!(fs.lookup(1, "explicit").await.is_ok());
    assert_eq!(store.list_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn no_directory_objects_skips_marker_probes_and_creates_only_child_objects() {
    let store = MemoryStore::new();
    store.seed("existing/child", b"x");
    let mut config = MemoryStore::config(32);
    config.no_dir_object = true;
    let fs = Filesystem::new(config.clone(), store.clone()).unwrap();
    fs.lookup(1, "existing").await.unwrap();
    assert!(
        !store
            .head_keys
            .lock()
            .unwrap()
            .iter()
            .any(|key| key == "existing/")
    );
    let dir = fs
        .create(1, "new", libc::S_IFDIR | 0o755, None, 0)
        .await
        .unwrap();
    fs.fsync(dir.ino).await.unwrap();
    assert!(!store.objects.lock().unwrap().contains_key("new/"));
    let file = fs
        .create(dir.ino, "file", libc::S_IFREG | 0o644, None, 0)
        .await
        .unwrap();
    fs.write(file.ino, 0, b"data").await.unwrap();
    fs.fsync(file.ino).await.unwrap();
    let fresh = Filesystem::new(config, store.clone()).unwrap();
    let dir = fresh.lookup(1, "new").await.unwrap();
    let file = fresh.lookup(dir.ino, "file").await.unwrap();
    assert_eq!(
        fresh.read(file.ino, 0, 4).await.unwrap().as_slice(),
        b"data"
    );
}

#[tokio::test]
async fn zero_metadata_ttl_refreshes_clean_objects_without_sleeping() {
    let store = MemoryStore::new();
    store.seed("file", b"old");
    let mut config = MemoryStore::config(32);
    config.stat_cache_ttl = Duration::ZERO;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let first = fs.lookup(1, "file").await.unwrap();
    assert_eq!(fs.read(first.ino, 0, 3).await.unwrap().as_slice(), b"old");
    store.seed("file", b"new");
    let next = fs.lookup(1, "file").await.unwrap();
    assert_eq!(next.ino, first.ino);
    assert_eq!(fs.read(next.ino, 0, 3).await.unwrap().as_slice(), b"new");
}

#[tokio::test]
async fn enomem_pressure_policy_rejects_without_flushing_other_dirty_data() {
    for enomem in [true, false] {
        let store = MemoryStore::new();
        let mut config = MemoryStore::config(16);
        config.use_enomem = enomem;
        let fs = Filesystem::new(config, store.clone()).unwrap();
        let file = fs
            .create(1, "file", libc::S_IFREG | 0o644, None, 0)
            .await
            .unwrap();
        let chunk = vec![b'a'; MAX_IO];
        for i in 0..5 {
            fs.write(file.ino, (i * MAX_IO) as u64, &chunk)
                .await
                .unwrap();
        }
        let result = fs.write(file.ino, (5 * MAX_IO) as u64, b"tail").await;
        if enomem {
            assert!(matches!(result, Err(Error::Fs(libc::ENOMEM))));
            assert!(store.objects.lock().unwrap().is_empty());
        } else {
            assert_eq!(result.unwrap(), 4);
            assert_eq!(store.bytes("file").len(), 5 * MAX_IO);
        }
        fs.fsync(file.ino).await.unwrap();
        let bytes = store.bytes("file");
        assert!(bytes[..5 * MAX_IO].iter().all(|b| *b == b'a'));
        assert_eq!(
            &bytes[5 * MAX_IO..],
            if enomem { &b""[..] } else { &b"tail"[..] }
        );
    }
}

#[tokio::test]
async fn a_stale_negative_probe_does_not_hide_a_concurrently_created_child() {
    let store = MemoryStore::new();
    let fs = store.filesystem(32);
    let (entered, release) = store.pause_list();
    let worker = fs.clone();
    let task = tokio::spawn(async move { worker.lookup(1, "child").await });
    tokio::time::timeout(Duration::from_secs(5), entered)
        .await
        .unwrap()
        .unwrap();
    let created = fs
        .create(1, "child", libc::S_IFREG | 0o644, None, 0)
        .await
        .unwrap();
    release.send(()).unwrap();
    let result = task.await.unwrap();
    assert!(
        matches!(result, Err(Error::Fs(libc::EAGAIN))),
        "a stale miss must be retried, not published"
    );
    assert_eq!(fs.lookup(1, "child").await.unwrap().ino, created.ino);
}

#[tokio::test]
async fn negative_cache_is_per_directory_bounded_and_caches_only_authoritative_misses() {
    let store = MemoryStore::new();
    store.seed("a/", b"");
    store.seed("b/", b"");
    let mut config = MemoryStore::config(32);
    config.no_implicit_dir = true;
    config.negative_lookup_cache_size = 2;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let a = fs.lookup(1, "a").await.unwrap();
    let b = fs.lookup(1, "b").await.unwrap();
    for (parent, name) in [(a.ino, "x"), (a.ino, "y"), (b.ino, "z")] {
        assert!(matches!(
            fs.lookup(parent, name).await,
            Err(Error::Fs(libc::ENOENT))
        ));
    }
    let calls = store.head_keys.lock().unwrap().len();
    for (parent, name) in [(a.ino, "x"), (a.ino, "y"), (b.ino, "z")] {
        assert!(fs.lookup(parent, name).await.is_err());
    }
    assert_eq!(
        store.head_keys.lock().unwrap().len(),
        calls,
        "capacity is per directory, not a smaller global cap"
    );
    assert!(fs.lookup(a.ino, "third").await.is_err());
    let calls = store.head_keys.lock().unwrap().len();
    assert!(fs.lookup(a.ino, "x").await.is_err());
    assert_eq!(
        store.head_keys.lock().unwrap().len(),
        calls + 2,
        "oldest local miss is evicted at the configured count"
    );
    let child = fs
        .create(a.ino, "x", libc::S_IFREG | 0o644, None, 0)
        .await
        .unwrap();
    assert_eq!(fs.lookup(a.ino, "x").await.unwrap().ino, child.ino);
}

#[tokio::test]
async fn an_authoritative_remote_deletion_retires_the_old_name_before_recreate() {
    let store = MemoryStore::new();
    store.seed("file", b"old");
    let mut config = MemoryStore::config(32);
    config.stat_cache_ttl = Duration::ZERO;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let old = fs.lookup(1, "file").await.unwrap();
    store.objects.lock().unwrap().remove("file");
    let new = fs
        .create(1, "file", libc::S_IFREG | 0o644, None, 0)
        .await
        .unwrap();
    assert_ne!(old.ino, new.ino);
    fs.write(new.ino, 0, b"new").await.unwrap();
    fs.fsync(new.ino).await.unwrap();
    assert_eq!(store.bytes("file"), b"new");
    assert!(fs.open(old.ino, true).is_err());
}

#[tokio::test]
async fn dirty_local_children_keep_a_markerless_parent_visible_after_its_ttl() {
    let store = MemoryStore::new();
    let mut config = MemoryStore::config(32);
    config.stat_cache_ttl = Duration::ZERO;
    config.no_dir_object = true;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let parent = fs
        .create(1, "dir", libc::S_IFDIR | 0o755, None, 0)
        .await
        .unwrap();
    fs.fsync(parent.ino).await.unwrap();
    let child = fs
        .create(parent.ino, "file", libc::S_IFREG | 0o644, None, 0)
        .await
        .unwrap();
    fs.write(child.ino, 0, b"local").await.unwrap();
    assert_eq!(fs.lookup(1, "dir").await.unwrap().ino, parent.ino);
    fs.fsync(child.ino).await.unwrap();
    assert_eq!(store.bytes("dir/file"), b"local");
}
