mod support;

#[tokio::test]
async fn patch_etag_growth_transfers_prepaid_snapshot_credit() {
    let store = support::MemoryStore::new();
    store.seed("file", b"data");
    // Ordinary unrelated generations make the PATCH ETag grow by one byte.
    for index in 0..9 {
        store.seed(&format!("unrelated-{index}"), b"other");
    }
    let mut config = support::MemoryStore::config(32);
    config.metadata_limit = Some(128 * 1024);
    config.enable_patch = true;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let node = fs.lookup(1, "file").await.unwrap();
    assert_eq!(
        node.state.lock().unwrap().remote.as_ref().unwrap().etag,
        "\"generation-0\""
    );
    fs.write(node.ino, 0, b"N").await.unwrap();
    let result = fs.fsync(node.ino).await;
    assert!(
        result.is_ok(),
        "fsync must transfer prepaid ETag growth: {result:?}; PATCH={}, charged={}",
        store.patch_calls.load(Ordering::SeqCst),
        fs.metadata_usage().charged
    );
    assert_eq!(store.patch_calls.load(Ordering::SeqCst), 1);
    assert_eq!(store.put_calls.load(Ordering::SeqCst), 0);
    {
        let state = node.state.lock().unwrap();
        assert_eq!(state.version, state.durable);
        assert!(state.extents.is_empty());
        // PATCH produces generation-10; persisting generated attributes uses
        // MemoryStore's metadata replacement and produces generation-11.
        assert_eq!(state.remote.as_ref().unwrap().etag, "\"generation-11\"");
    }
    assert_eq!(store.objects.lock().unwrap()["file"].bytes, b"Nata");
    assert_eq!(fs.read(node.ino, 0, 4).await.unwrap().as_slice(), b"Nata");
    fs.fsync(node.ino).await.unwrap();
    assert_eq!(store.patch_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn dirty_rename_uses_reserved_projection_credit_after_put_commits() {
    let store = support::MemoryStore::new();
    store.seed("old", b"data");
    store
        .objects
        .lock()
        .unwrap()
        .get_mut("old")
        .unwrap()
        .metadata
        .insert("label".into(), "x".repeat(2048));
    let mut config = support::MemoryStore::config(32);
    config.metadata_limit = Some(316 * 1024);
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let node = fs.lookup(1, "old").await.unwrap();
    fs.write(node.ino, 0, b"N").await.unwrap();
    let result = fs.rename(1, "old", 1, "new").await;
    assert!(
        result.is_ok(),
        "rename must reuse pre-admitted postcommit credit: {result:?}; PUT={}, COPY={}",
        store.put_calls.load(Ordering::SeqCst),
        store.copy_calls.load(Ordering::SeqCst)
    );
    assert_eq!(store.put_calls.load(Ordering::SeqCst), 1);
    assert_eq!(store.copy_calls.load(Ordering::SeqCst), 1);
    let state = node.state.lock().unwrap();
    assert_eq!(state.key.as_str(), "new");
    assert_eq!(state.version, state.durable);
    assert_eq!(
        state.metadata.get("label"),
        Some(vec![b'x'; 2048].as_slice())
    );
    let objects = store.objects.lock().unwrap();
    assert_eq!(objects["new"].bytes, b"Nata");
    assert!(!objects.contains_key("old"));
}

#[tokio::test]
async fn unexpected_postcommit_head_growth_is_rejected_without_losing_our_generation() {
    let store = support::MemoryStore::new();
    store.seed("file", b"data");
    let mut config = support::MemoryStore::config(32);
    config.metadata_limit = Some(64 * 1024);
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let node = fs.lookup(1, "file").await.unwrap();
    fs.write(node.ino, 0, b"N").await.unwrap();
    let (entered, release) = store.pause_head();
    let flush = tokio::spawn({
        let fs = fs.clone();
        let ino = node.ino;
        async move { fs.fsync(ino).await }
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), entered)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(store.put_calls.load(Ordering::SeqCst), 1);
    let committed = {
        let mut objects = store.objects.lock().unwrap();
        let object = objects.get_mut("file").unwrap();
        // Metadata-only external edits need not change the content ETag.
        object
            .metadata
            .insert("external".into(), "x".repeat(128 * 1024));
        object.etag.clone()
    };
    release.send(()).unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), flush)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        result,
        Err(geesefs_rs::error::Error::Fs(libc::ENOMEM))
    ));
    {
        let state = node.state.lock().unwrap();
        assert_eq!(state.remote.as_ref().unwrap().etag, committed);
        assert!(state.version > state.durable);
        assert!(!state.extents.is_empty());
        assert!(!state.metadata.contains_key("external"));
    }
    assert!(fs.metadata_usage().charged <= 64 * 1024);
    assert_eq!(fs.read(node.ino, 0, 4).await.unwrap().as_slice(), b"Nata");
    store
        .objects
        .lock()
        .unwrap()
        .get_mut("file")
        .unwrap()
        .metadata
        .remove("external");
    fs.fsync(node.ino).await.unwrap();
    let state = node.state.lock().unwrap();
    assert_eq!(state.version, state.durable);
    assert_eq!(store.objects.lock().unwrap()["file"].bytes, b"Nata");
}

use std::{collections::BTreeMap, sync::atomic::Ordering};

use geesefs_rs::{
    backend::metadata::{CachedMetadata, decode_metadata},
    filesystem::Filesystem,
    metadata_budget::MetadataBudget,
    read,
};

#[test]
fn canonical_metadata_is_sorted_binary_and_borrowed() {
    let budget = MetadataBudget::new(Some(64 * 1024)).unwrap();
    let metadata = CachedMetadata::new(
        decode_metadata([
            ("Z%2B", "%FF%00"),
            ("a", ""),
            ("geesefs-upload-receipt", "private"),
        ]),
        &budget,
    )
    .unwrap();
    assert_eq!(metadata.keys().collect::<Vec<_>>(), ["a", "z+"]);
    assert_eq!(metadata.get("z+"), Some([255, 0].as_slice()));
    assert_eq!(metadata.get("a"), Some([].as_slice()));
    assert_eq!(metadata.get("missing"), None);
    assert!(!metadata.contains_key("geesefs-upload-receipt"));
    assert!(budget.usage().charged > 0);
    drop(metadata);
    assert_eq!(budget.usage().charged, 0);
}

#[tokio::test]
async fn clean_inode_and_read_snapshots_share_the_committed_owner() {
    let store = support::MemoryStore::new();
    store.seed("file", b"payload");
    store
        .objects
        .lock()
        .unwrap()
        .get_mut("file")
        .unwrap()
        .metadata
        .insert("label".into(), "original".into());
    let mut config = support::MemoryStore::config(16);
    config.metadata_limit = Some(256 * 1024);
    let fs = Filesystem::new(config, store).unwrap();
    let node = fs.lookup(1, "file").await.unwrap();
    let snapshot = read::snapshot(&node).unwrap();
    let committed = snapshot.remote().unwrap().metadata.clone();
    let original_ptr = committed.get("label").unwrap().as_ptr();
    {
        let state = node.state.lock().unwrap();
        assert_eq!(state.metadata.get("label").unwrap().as_ptr(), original_ptr);
        assert_eq!(
            state
                .remote
                .as_ref()
                .unwrap()
                .metadata
                .get("label")
                .unwrap()
                .as_ptr(),
            original_ptr
        );
        assert_eq!(state.key.as_ptr(), snapshot.key().as_ptr());
        let namespace = fs.namespace.lock().unwrap();
        assert_eq!(
            namespace.names.get_key_value("file").unwrap().0.as_ptr(),
            state.key.as_ptr()
        );
    }
    fs.setxattr(node.ino, "user.label", Some(b"local edit"), 0)
        .unwrap();
    assert_eq!(committed.get("label"), Some(b"original".as_slice()));
    assert_eq!(
        snapshot.remote().unwrap().metadata.get("label"),
        Some(b"original".as_slice())
    );
    assert_eq!(
        fs.xattrs(node.ino).unwrap().get("user.label"),
        Some(b"local edit".as_slice())
    );
    fs.fsync(node.ino).await.unwrap();
    let state = node.state.lock().unwrap();
    assert_eq!(state.version, state.durable);
    assert_eq!(
        state.metadata.get("label").unwrap().as_ptr(),
        state
            .remote
            .as_ref()
            .unwrap()
            .metadata
            .get("label")
            .unwrap()
            .as_ptr()
    );
    assert_eq!(committed.get("label"), Some(b"original".as_slice()));
}

#[tokio::test]
async fn cached_flat_listing_reinstalls_the_same_metadata_backing_after_eviction() {
    let store = support::MemoryStore::new();
    for key in ["a", "z"] {
        store.seed(key, b"payload");
        store
            .objects
            .lock()
            .unwrap()
            .get_mut(key)
            .unwrap()
            .metadata
            .insert("label".into(), "%FF%00".into());
    }
    let mut config = support::MemoryStore::config(16);
    config.metadata_limit = Some(256 * 1024);
    config.entry_limit = 2;
    config.list_type = Some(geesefs_rs::config::ListType::Extended);
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let initial = fs.lookup(1, "a").await.unwrap();
    let initial_key = initial.state.lock().unwrap().key.clone();
    let initial_metadata = initial.state.lock().unwrap().metadata.clone();
    drop(initial);
    let entries = fs.directory_chunk(1, "", 1).await.unwrap();
    assert_eq!(entries[0].name, "a");
    let old_ino = entries[0].ino;
    let metadata = fs
        .node(old_ino)
        .unwrap()
        .state
        .lock()
        .unwrap()
        .metadata
        .clone();
    let old_key = fs.node(old_ino).unwrap().state.lock().unwrap().key.clone();
    let original_ptr = metadata.get("label").unwrap().as_ptr();
    assert_eq!(
        original_ptr,
        initial_metadata.get("label").unwrap().as_ptr()
    );
    assert_eq!(old_key.as_ptr(), initial_key.as_ptr());
    assert_eq!(metadata.get("label"), Some([255, 0].as_slice()));
    drop(entries);
    drop(fs.lookup(1, "z").await.unwrap());
    assert!(
        fs.node(old_ino).is_err(),
        "fixture must actually evict the installed inode"
    );
    let lists_before_reinstall = store.list_calls.load(Ordering::SeqCst);
    store.head_keys.lock().unwrap().clear();
    let entries = fs.directory_chunk(1, "", 1).await.unwrap();
    assert_eq!(entries[0].name, "a");
    let node = fs.node(entries[0].ino).unwrap();
    let state = node.state.lock().unwrap();
    assert_eq!(state.metadata.get("label").unwrap().as_ptr(), original_ptr);
    assert_eq!(state.key.as_ptr(), old_key.as_ptr());
    assert_eq!(
        store.list_calls.load(Ordering::SeqCst),
        lists_before_reinstall
    );
    assert!(store.head_keys.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_evicted_key_remains_charged_until_its_last_snapshot_owner_drops() {
    let store = support::MemoryStore::new();
    let name = "k".repeat(900);
    store.seed(&name, b"payload");
    store.seed("replacement", b"payload");
    let mut config = support::MemoryStore::config(16);
    config.entry_limit = 2;
    config.metadata_limit = Some(256 * 1024);
    let fs = Filesystem::new(config, store).unwrap();
    let node = fs.lookup(1, &name).await.unwrap();
    let key = node.state.lock().unwrap().key.clone();
    let snapshot = read::snapshot(&node).unwrap();
    let old_ino = node.ino;
    drop(node);
    drop(fs.lookup(1, "replacement").await.unwrap());
    assert!(fs.node(old_ino).is_err());
    assert_eq!(snapshot.key(), name);
    assert_eq!(key.as_str(), name);
    drop(snapshot);
    let with_last_owner = fs.metadata_usage().charged;
    drop(key);
    assert!(with_last_owner - fs.metadata_usage().charged >= name.len());
}

#[tokio::test]
async fn rename_publishes_one_new_key_owner_without_changing_old_snapshots() {
    let store = support::MemoryStore::new();
    store.seed("old", b"payload");
    let mut config = support::MemoryStore::config(16);
    config.metadata_limit = Some(256 * 1024);
    let fs = Filesystem::new(config, store).unwrap();
    let node = fs.lookup(1, "old").await.unwrap();
    let old = read::snapshot(&node).unwrap();
    fs.rename(1, "old", 1, "new").await.unwrap();
    let fresh = read::snapshot(&node).unwrap();
    assert_eq!(old.key(), "old");
    assert_eq!(fresh.key(), "new");
    let namespace = fs.namespace.lock().unwrap();
    let state = node.state.lock().unwrap();
    assert_eq!(state.key.as_ptr(), fresh.key().as_ptr());
    assert_eq!(
        namespace.names.get_key_value("new").unwrap().0.as_ptr(),
        fresh.key().as_ptr()
    );
    assert!(!namespace.names.contains_key("old"));
    assert_ne!(old.key().as_ptr(), fresh.key().as_ptr());
}

#[test]
fn clone_shares_backing_and_last_owner_holds_admission() {
    let budget = MetadataBudget::new(Some(64 * 1024)).unwrap();
    let baseline = CachedMetadata::new(
        BTreeMap::from([("key".into(), b"original".to_vec())]),
        &budget,
    )
    .unwrap();
    let charged = budget.usage().charged;
    let snapshot = baseline.clone();
    assert_eq!(
        baseline.get("key").unwrap().as_ptr(),
        snapshot.get("key").unwrap().as_ptr()
    );
    assert_eq!(budget.usage().charged, charged);
    drop(baseline);
    assert_eq!(snapshot.get("key"), Some(b"original".as_slice()));
    assert_eq!(budget.usage().charged, charged);
    drop(snapshot);
    assert_eq!(budget.usage().charged, 0);
}

#[test]
fn cow_edit_preserves_committed_and_snapshot_values() {
    let budget = MetadataBudget::new(Some(64 * 1024)).unwrap();
    let mut current = CachedMetadata::new(
        BTreeMap::from([("key".into(), b"original".to_vec())]),
        &budget,
    )
    .unwrap();
    let baseline = current.clone();
    let snapshot = current.clone();
    current.try_insert("key", b"changed").unwrap();
    assert_eq!(current.get("key"), Some(b"changed".as_slice()));
    assert_eq!(baseline.get("key"), Some(b"original".as_slice()));
    assert_eq!(snapshot.get("key"), Some(b"original".as_slice()));
    current.try_remove("key").unwrap();
    assert!(current.is_empty());
    assert_eq!(baseline.get("key"), Some(b"original".as_slice()));
}

#[test]
fn rejected_cow_edit_is_atomic() {
    let budget = MetadataBudget::new(Some(4096)).unwrap();
    let mut metadata = CachedMetadata::new(
        BTreeMap::from([("key".into(), b"original".to_vec())]),
        &budget,
    )
    .unwrap();
    let snapshot = metadata.clone();
    let charged = budget.usage().charged;
    assert!(matches!(
        metadata.try_insert("large", &[1; 8192]),
        Err(geesefs_rs::error::Error::Fs(libc::ENOMEM))
    ));
    assert_eq!(metadata.get("key"), Some(b"original".as_slice()));
    assert!(!metadata.contains_key("large"));
    assert_eq!(
        metadata.get("key").unwrap().as_ptr(),
        snapshot.get("key").unwrap().as_ptr()
    );
    assert_eq!(budget.usage().charged, charged);
}
