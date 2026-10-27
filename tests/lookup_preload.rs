mod support;

use geesefs_rs::{config::ListType, filesystem::Filesystem};
use std::sync::atomic::Ordering;
use support::MemoryStore;

fn filesystem(store: &std::sync::Arc<MemoryStore>) -> std::sync::Arc<Filesystem> {
    let mut config = MemoryStore::config(16);
    config.list_type = Some(ListType::Extended);
    Filesystem::new(config, store.clone()).unwrap()
}

#[tokio::test]
async fn extended_lookup_reuses_one_bounded_page_and_preserves_metadata() {
    let store = MemoryStore::new();
    for name in ["a", "b", "c"] {
        store.seed(name, name.as_bytes());
    }
    store
        .objects
        .lock()
        .unwrap()
        .get_mut("b")
        .unwrap()
        .metadata
        .insert("mode".into(), "384".into());
    let fs = filesystem(&store);
    for name in ["a", "b", "c"] {
        let node = fs.lookup(1, name).await.unwrap();
        assert_eq!(
            fs.read(node.ino, 0, 1).await.unwrap().as_slice(),
            name.as_bytes()
        );
        if name == "b" {
            assert_eq!(node.attr().perm, 0o600);
        }
    }
    assert_eq!(store.list_calls.load(Ordering::SeqCst), 1);
    assert!(store.head_keys.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_object_beyond_the_preloaded_page_is_still_looked_up() {
    let store = MemoryStore::new();
    store.page_size.store(1, Ordering::SeqCst);
    store.seed("a", b"first");
    store.seed("z", b"last");
    let fs = filesystem(&store);
    let node = fs.lookup(1, "z").await.unwrap();
    assert_eq!(fs.read(node.ino, 0, 4).await.unwrap().as_slice(), b"last");
    assert_eq!(store.list_calls.load(Ordering::SeqCst), 1);
    assert_eq!(*store.head_keys.lock().unwrap(), ["z"]);
}

#[tokio::test]
async fn absent_standard_metadata_never_claims_a_regular_file_type() {
    let store = MemoryStore::new();
    store.standard_listing.store(true, Ordering::SeqCst);
    store.seed("link", b"");
    store
        .objects
        .lock()
        .unwrap()
        .get_mut("link")
        .unwrap()
        .metadata
        .insert("--symlink-target".into(), "target".into());
    let fs = filesystem(&store);
    let node = fs.lookup(1, "link").await.unwrap();
    assert_eq!(node.attr().kind, fuser::FileType::Symlink);
    assert_eq!(node.attr().size, 6);
    assert_eq!(*store.head_keys.lock().unwrap(), ["link"]);
}

#[tokio::test]
async fn local_creation_during_preload_is_not_overwritten_by_the_old_page() {
    let store = MemoryStore::new();
    store.seed("dir/a", b"remote");
    let fs = filesystem(&store);
    let directory = fs.lookup(1, "dir").await.unwrap();
    let (entered, release) = store.pause_list();
    let worker = {
        let fs = fs.clone();
        tokio::spawn(async move { fs.lookup(directory.ino, "a").await })
    };
    tokio::time::timeout(std::time::Duration::from_secs(2), entered)
        .await
        .unwrap()
        .unwrap();
    let local = fs
        .create(1, "new", libc::S_IFREG | 0o600, None, 0)
        .await
        .unwrap();
    fs.write(local.ino, 0, b"local").await.unwrap();
    store.seed("dir/a", b"replacement");
    release.send(()).unwrap();
    assert_eq!(worker.await.unwrap().unwrap().attr().size, 11);
    let node = fs.lookup(1, "new").await.unwrap();
    assert_eq!(node.ino, local.ino);
    assert_eq!(fs.read(node.ino, 0, 5).await.unwrap().as_slice(), b"local");
}

#[tokio::test]
async fn simultaneous_lookups_share_the_bounded_preload_attempt() {
    use std::{future::Future, task::Poll};
    let store = MemoryStore::new();
    store.seed("a", b"a");
    store.seed("b", b"b");
    let fs = filesystem(&store);
    let (entered, release) = store.pause_list();
    let first = {
        let fs = fs.clone();
        tokio::spawn(async move { fs.lookup(1, "a").await })
    };
    tokio::time::timeout(std::time::Duration::from_secs(2), entered)
        .await
        .unwrap()
        .unwrap();
    let mut second = Box::pin(fs.lookup(1, "b"));
    std::future::poll_fn(|cx| {
        assert!(second.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    release.send(()).unwrap();
    assert_eq!(first.await.unwrap().unwrap().attr().size, 1);
    assert_eq!(second.await.unwrap().attr().size, 1);
    assert_eq!(store.list_calls.load(Ordering::SeqCst), 1);
    assert!(store.head_keys.lock().unwrap().is_empty());
}

#[tokio::test]
async fn zero_ttl_disables_speculative_lookup_preloading() {
    let store = MemoryStore::new();
    store.seed("a", b"old");
    let mut config = MemoryStore::config(16);
    config.list_type = Some(ListType::Extended);
    config.stat_cache_ttl = std::time::Duration::ZERO;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    assert_eq!(fs.lookup(1, "a").await.unwrap().attr().size, 3);
    store.seed("a", b"new generation");
    assert_eq!(fs.lookup(1, "a").await.unwrap().attr().size, 14);
    assert_eq!(store.list_calls.load(Ordering::SeqCst), 0);
    assert_eq!(*store.head_keys.lock().unwrap(), ["a", "a"]);
}

#[tokio::test]
async fn speculative_listing_never_replaces_dirty_existing_inode_data() {
    let store = MemoryStore::new();
    store.seed("local", b"original");
    let fs = filesystem(&store);
    let entries = fs.directory_chunk(1, "", 10).await.unwrap();
    let local = entries[0].ino;
    fs.write(local, 0, b"changed!").await.unwrap();
    store.seed("new", b"remote");
    let (entered, release) = store.pause_list();
    let worker = {
        let fs = fs.clone();
        tokio::spawn(async move { fs.lookup(1, "new").await })
    };
    tokio::time::timeout(std::time::Duration::from_secs(2), entered)
        .await
        .unwrap()
        .unwrap();
    fs.write(local, 0, b"updated!").await.unwrap();
    release.send(()).unwrap();
    assert_eq!(worker.await.unwrap().unwrap().attr().size, 6);
    assert_eq!(fs.read(local, 0, 8).await.unwrap().as_slice(), b"updated!");
    assert_eq!(fs.lookup(1, "local").await.unwrap().ino, local);
}

#[tokio::test]
async fn cancellation_releases_preload_ownership_for_the_next_lookup() {
    let store = MemoryStore::new();
    store.seed("a", b"a");
    store.seed("b", b"b");
    let fs = filesystem(&store);
    let (entered, _release) = store.pause_list();
    let first = {
        let fs = fs.clone();
        tokio::spawn(async move { fs.lookup(1, "a").await })
    };
    tokio::time::timeout(std::time::Duration::from_secs(2), entered)
        .await
        .unwrap()
        .unwrap();
    first.abort();
    assert!(matches!(first.await, Err(error) if error.is_cancelled()));
    let node = tokio::time::timeout(std::time::Duration::from_secs(2), fs.lookup(1, "b"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fs.read(node.ino, 0, 1).await.unwrap().as_slice(), b"b");
    assert_eq!(store.list_calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn ordinary_creates_do_not_repeat_a_still_fresh_speculative_page() {
    let store = MemoryStore::new();
    let fs = filesystem(&store);
    for name in ["a", "b", "c"] {
        let node = fs
            .create(1, name, libc::S_IFREG | 0o600, None, 0)
            .await
            .unwrap();
        fs.write(node.ino, 0, name.as_bytes()).await.unwrap();
        assert_eq!(
            fs.read(node.ino, 0, 1).await.unwrap().as_slice(),
            name.as_bytes()
        );
    }
    // One speculative page, then one authoritative implicit-directory absence
    // probe per creation. A failed speculative page is never a negative cache.
    assert_eq!(store.list_calls.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn denied_speculative_listing_preserves_authoritative_head_and_payload_reads() {
    let store = MemoryStore::new();
    store.seed("a", b"first");
    store.seed("b", b"second");
    store.fail_list.store(true, Ordering::SeqCst);
    let fs = filesystem(&store);
    for (name, expected) in [("a", b"first".as_slice()), ("b", b"second".as_slice())] {
        let node = fs.lookup(1, name).await.unwrap();
        assert_eq!(
            fs.read(node.ino, 0, expected.len())
                .await
                .unwrap()
                .as_slice(),
            expected
        );
    }
    assert_eq!(store.list_calls.load(Ordering::SeqCst), 1);
    assert_eq!(*store.head_keys.lock().unwrap(), ["a", "b"]);
}
