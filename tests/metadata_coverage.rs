mod support;

use geesefs_rs::{config::ListType, error::Error, filesystem::Filesystem};
use std::{
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};
use support::MemoryStore;

fn filesystem(store: &Arc<MemoryStore>) -> Arc<Filesystem> {
    let mut config = MemoryStore::config(16);
    config.list_type = Some(ListType::Extended);
    config.stat_cache_ttl = Duration::from_secs(60);
    config.negative_lookup_cache_size = 64;
    Filesystem::new(config, store.clone()).unwrap()
}

#[tokio::test]
async fn complete_fresh_coverage_answers_distinct_absences_without_s3() {
    let store = MemoryStore::new();
    store.seed("a", b"abc");
    let fs = filesystem(&store);
    let page = fs.directory_chunk(1, "", 10).await.unwrap();
    assert_eq!(page[0].name, "a");
    assert_eq!(store.list_calls.load(Ordering::SeqCst), 1);

    // Completeness is stronger than retaining 64 individual negative names.
    for index in 0..66 {
        assert!(matches!(
            fs.lookup(1, &format!("missing-{index}")).await,
            Err(Error::Fs(libc::ENOENT))
        ));
    }
    assert_eq!(store.list_calls.load(Ordering::SeqCst), 1);
    assert!(store.head_keys.lock().unwrap().is_empty());
    assert_eq!(store.read_calls.load(Ordering::SeqCst), 0);
    let node = fs.lookup(1, "a").await.unwrap();
    assert_eq!(
        fs.refresh_inode_if_expired(node.ino)
            .await
            .unwrap()
            .attr()
            .size,
        3
    );
    assert!(store.head_keys.lock().unwrap().is_empty());
}

#[tokio::test]
async fn complete_descendant_coverage_is_reused_without_sealing_its_parent() {
    let store = MemoryStore::new();
    for name in ["0", "a/one", "b/two"] {
        store.seed(name, b"data");
    }
    let fs = filesystem(&store);
    let a = fs.lookup(1, "a").await.unwrap();
    assert_eq!(
        fs.directory_chunk(a.ino, "", 10).await.unwrap()[0].name,
        "one"
    );
    let lists = store.list_calls.load(Ordering::SeqCst);
    let heads = store.head_keys.lock().unwrap().len();
    assert!(matches!(
        fs.lookup(a.ino, "absent").await,
        Err(Error::Fs(libc::ENOENT))
    ));
    assert_eq!(store.list_calls.load(Ordering::SeqCst), lists);
    assert_eq!(store.head_keys.lock().unwrap().len(), heads);
    // The flat response started after a/: it cannot exclude earlier root names.
    assert_eq!(fs.lookup(1, "0").await.unwrap().attr().size, 4);
}

#[tokio::test]
async fn objects_markers_and_descendants_outside_the_emitted_chunk_are_not_absent() {
    let store = MemoryStore::new();
    for name in ["a", "a.", "a0", "dir/child", "marker/"] {
        store.seed(name, b"data");
    }
    let fs = filesystem(&store);
    let page = fs.directory_chunk(1, "", 1).await.unwrap();
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].name, "a");
    assert_eq!(fs.lookup(1, "a.").await.unwrap().attr().size, 4);
    assert_eq!(fs.lookup(1, "a0").await.unwrap().attr().size, 4);
    for name in ["dir", "marker"] {
        assert_eq!(
            fs.lookup(1, name).await.unwrap().attr().kind,
            fuser::FileType::Directory
        );
    }
}

#[tokio::test]
async fn truncated_root_coverage_does_not_hide_a_later_object() {
    let store = MemoryStore::new();
    store.page_size.store(1, Ordering::SeqCst);
    store.seed("a", b"first");
    store.seed("z", b"last");
    let fs = filesystem(&store);
    assert_eq!(fs.directory_chunk(1, "", 1).await.unwrap()[0].name, "a");
    assert_eq!(fs.lookup(1, "z").await.unwrap().attr().size, 4);
    assert_eq!(*store.head_keys.lock().unwrap(), ["z"]);
}

#[tokio::test]
async fn local_name_mutation_invalidates_complete_coverage() {
    let store = MemoryStore::new();
    let fs = filesystem(&store);
    assert!(fs.directory_chunk(1, "", 10).await.unwrap().is_empty());
    let local = fs
        .create(1, "local", libc::S_IFREG | 0o600, None, 0)
        .await
        .unwrap();
    fs.write(local.ino, 0, b"local bytes").await.unwrap();
    store.seed("remote", b"new generation");
    assert_eq!(fs.lookup(1, "remote").await.unwrap().attr().size, 14);
    assert_eq!(
        fs.read(local.ino, 0, 11).await.unwrap().as_slice(),
        b"local bytes"
    );
}

#[tokio::test]
async fn failed_or_cancelled_listing_never_proves_absence() {
    let store = MemoryStore::new();
    let fs = filesystem(&store);
    store.fail_list.store(true, Ordering::SeqCst);
    assert!(fs.directory_chunk(1, "", 10).await.is_err());
    store.fail_list.store(false, Ordering::SeqCst);
    let (entered, _release) = store.pause_list();
    let listing = {
        let fs = fs.clone();
        tokio::spawn(async move { fs.directory_chunk(1, "", 10).await })
    };
    tokio::time::timeout(Duration::from_secs(2), entered)
        .await
        .unwrap()
        .unwrap();
    listing.abort();
    assert!(listing.await.unwrap_err().is_cancelled());
    store.seed("new", b"new bytes");
    assert_eq!(fs.lookup(1, "new").await.unwrap().attr().size, 9);
}

#[tokio::test]
async fn disabling_negative_caching_preserves_authoritative_miss_probes() {
    let store = MemoryStore::new();
    let mut config = MemoryStore::config(16);
    config.list_type = Some(ListType::Extended);
    config.negative_lookup_cache_size = 0;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    assert!(fs.directory_chunk(1, "", 10).await.unwrap().is_empty());
    store.seed("new", b"visible");
    assert_eq!(fs.lookup(1, "new").await.unwrap().attr().size, 7);
}

#[tokio::test]
async fn zero_ttl_never_reuses_old_absence() {
    let store = MemoryStore::new();
    let mut config = MemoryStore::config(16);
    config.negative_lookup_cache_size = 64;
    config.stat_cache_ttl = Duration::ZERO;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    assert!(fs.directory_chunk(1, "", 10).await.unwrap().is_empty());
    store.seed("new", b"visible");
    assert_eq!(fs.lookup(1, "new").await.unwrap().attr().size, 7);
}

#[tokio::test]
async fn a_list_that_consumed_its_ttl_does_not_gain_new_absence_freshness() {
    let store = MemoryStore::new();
    let mut config = MemoryStore::config(16);
    config.list_type = Some(ListType::Extended);
    config.negative_lookup_cache_size = 64;
    let ttl = Duration::from_millis(100);
    config.stat_cache_ttl = ttl;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let (entered, release) = store.pause_list();
    let listing = {
        let fs = fs.clone();
        tokio::spawn(async move { fs.directory_chunk(1, "", 10).await })
    };
    tokio::time::timeout(Duration::from_secs(2), entered)
        .await
        .unwrap()
        .unwrap();
    // Filesystem freshness uses std::Instant. Wait for that actual TTL boundary,
    // not an arbitrary concurrency delay or Tokio's unrelated virtual clock.
    let expired = Instant::now() + ttl;
    tokio::time::sleep_until(tokio::time::Instant::from_std(expired)).await;
    release.send(()).unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(2), listing)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .is_empty()
    );
    store.seed("new", b"visible");
    assert_eq!(fs.lookup(1, "new").await.unwrap().attr().size, 7);
}
