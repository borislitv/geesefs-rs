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
async fn one_flat_page_serves_complete_descendant_directories_and_attributes() {
    let store = MemoryStore::new();
    for name in ["a/one", "a/two", "b/nested/three", "c/link"] {
        store.seed(name, b"contents");
    }
    store
        .objects
        .lock()
        .unwrap()
        .get_mut("c/link")
        .unwrap()
        .metadata
        .insert("--symlink-target".into(), "../a/one".into());
    let fs = filesystem(&store);
    let mut pending = vec![1];
    let mut files = 0;
    let mut symlinks = 0;
    while let Some(parent) = pending.pop() {
        let entries = fs.directory_chunk(parent, "", 1000).await.unwrap();
        for entry in entries {
            let attr = fs.refresh_inode_if_expired(entry.ino).await.unwrap().attr();
            assert_eq!(attr.kind, entry.kind);
            if attr.kind == fuser::FileType::Directory {
                pending.push(entry.ino);
            } else {
                assert_eq!(attr.size, 8);
                assert_eq!(attr.blocks, 1);
                files += 1;
                symlinks += usize::from(attr.kind == fuser::FileType::Symlink);
            }
        }
    }
    assert_eq!((files, symlinks), (4, 1));
    assert_eq!(
        store.list_calls.load(Ordering::SeqCst),
        1,
        "one complete bounded flat LIST must serve the entire small subtree"
    );
    assert!(
        store.head_keys.lock().unwrap().is_empty(),
        "the same flat page proves missing implicit-directory markers"
    );
}

#[tokio::test]
async fn flat_markers_preserve_authoritative_directory_metadata() {
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
    let fs = filesystem(&store);
    let entry = fs.directory_chunk(1, "", 1000).await.unwrap().remove(0);
    let attr = fs.refresh_inode_if_expired(entry.ino).await.unwrap().attr();
    assert_eq!((attr.perm, attr.uid, attr.gid), (0o750, 1234, 5678));
    assert_eq!(
        fs.directory_chunk(entry.ino, "", 1000).await.unwrap()[0].name,
        "child"
    );
    assert_eq!(store.list_calls.load(Ordering::SeqCst), 1);
    assert!(store.head_keys.lock().unwrap().is_empty());
}

fn names(entries: Vec<geesefs_rs::filesystem::DirEntry>) -> Vec<String> {
    entries
        .into_iter()
        .map(|entry| entry.name.as_str().to_owned())
        .collect()
}

#[tokio::test]
async fn a_truncated_flat_page_seals_only_complete_subtrees() {
    let store = MemoryStore::new();
    store.page_size.store(3, Ordering::Relaxed);
    for name in ["a/one", "a/two", "b/one", "b/two", "c/one"] {
        store.seed(name, b"data");
    }
    let fs = filesystem(&store);
    let root = fs.directory_chunk(1, "", 1000).await.unwrap();
    assert_eq!(
        root.iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["a", "b", "c"]
    );
    assert_eq!(
        *store.list_requests.lock().unwrap(),
        [("".into(), None), ("".into(), Some("/".into())),],
        "an incomplete root falls back after exactly one flat page"
    );
    assert_eq!(
        names(fs.directory_chunk(root[0].ino, "", 1000).await.unwrap()),
        ["one", "two"]
    );
    assert_eq!(
        store.list_calls.load(Ordering::Relaxed),
        2,
        "a/ is completely inside the flat range"
    );
    assert_eq!(
        names(fs.directory_chunk(root[1].ino, "", 1000).await.unwrap()),
        ["one", "two"],
        "b/ must not be sealed with only its first child"
    );
    assert_eq!(
        names(fs.directory_chunk(root[2].ino, "", 1000).await.unwrap()),
        ["one"]
    );
    assert_eq!(
        store.list_calls.load(Ordering::Relaxed),
        3,
        "the second flat page also warms c/"
    );
}

#[tokio::test]
async fn large_directories_use_delimiter_pages_after_one_flat_attempt() {
    let store = MemoryStore::new();
    store.page_size.store(2, Ordering::Relaxed);
    for name in ["big/", "big/a", "big/b", "big/c", "big/d", "big/e"] {
        store.seed(name, b"data");
    }
    let fs = store.filesystem(16);
    let directory = fs.lookup(1, "big").await.unwrap();
    store.list_requests.lock().unwrap().clear();
    let mut after = String::new();
    let mut actual = Vec::new();
    loop {
        let entries = fs.directory_chunk(directory.ino, &after, 2).await.unwrap();
        if entries.is_empty() {
            break;
        }
        after = entries.last().unwrap().name.as_str().to_owned();
        actual.extend(names(entries));
    }
    assert_eq!(actual, ["a", "b", "c", "d", "e"]);
    let requests = store.list_requests.lock().unwrap();
    assert_eq!(requests[0], ("".into(), None));
    assert!(
        requests[1..]
            .iter()
            .all(|request| *request == ("big/".into(), Some("/".into())))
    );
    assert_eq!(
        requests.len(),
        6,
        "one flat attempt, two pages past the marker, two resume pages and EOF"
    );
}

#[tokio::test]
async fn slurping_a_child_warms_siblings_without_sealing_its_parent() {
    let store = MemoryStore::new();
    for name in ["0", "a/", "a/one", "b/", "b/two"] {
        store.seed(name, b"data");
    }
    let fs = store.filesystem(16);
    let a = fs.lookup(1, "a").await.unwrap();
    assert_eq!(
        names(fs.directory_chunk(a.ino, "", 1000).await.unwrap()),
        ["one"]
    );
    let b = fs.lookup(1, "b").await.unwrap();
    assert_eq!(
        names(fs.directory_chunk(b.ino, "", 1000).await.unwrap()),
        ["two"]
    );
    let before = store.list_calls.load(Ordering::Relaxed);
    assert_eq!(before, 1, "the flat page for a/ covers b/ too");
    assert_eq!(
        names(fs.directory_chunk(1, "", 1000).await.unwrap()),
        ["0", "a", "b"]
    );
    assert_eq!(
        store.list_calls.load(Ordering::Relaxed),
        before + 1,
        "the parent was not listed from its beginning"
    );
}

#[tokio::test]
async fn flat_standard_metadata_still_requires_authoritative_file_heads() {
    let store = MemoryStore::new();
    store.standard_listing.store(true, Ordering::Relaxed);
    store.seed("dir/link", b"body");
    store
        .objects
        .lock()
        .unwrap()
        .get_mut("dir/link")
        .unwrap()
        .metadata
        .insert("--symlink-target".into(), "target".into());
    let fs = store.filesystem(16);
    let root = fs.directory_chunk(1, "", 1000).await.unwrap();
    let entries = fs.directory_chunk(root[0].ino, "", 1000).await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].kind, fuser::FileType::Symlink);
    assert_eq!(fs.node(entries[0].ino).unwrap().attr().size, 4);
    assert_eq!(*store.head_keys.lock().unwrap(), ["dir/link"]);
    assert_eq!(store.list_calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn expired_child_metadata_is_not_rejuvenated_by_a_warm_flat_page() {
    let store = MemoryStore::new();
    store.seed("dir/file", b"old");
    let fs = store.filesystem(16);
    let root = fs.directory_chunk(1, "", 1000).await.unwrap();
    let old = fs.directory_chunk(root[0].ino, "", 1000).await.unwrap();
    fs.node(old[0].ino).unwrap().state.lock().unwrap().fetched =
        std::time::Instant::now() - 2 * fs.config.stat_cache_ttl;
    store.seed("dir/file", b"new generation");
    let fresh = fs.directory_chunk(root[0].ino, "", 1000).await.unwrap();
    assert_eq!(fresh[0].ino, old[0].ino);
    assert_eq!(fs.node(fresh[0].ino).unwrap().attr().size, 14);
    assert_eq!(store.list_calls.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn a_failed_flat_request_propagates_and_can_be_retried() {
    let store = MemoryStore::new();
    store.seed("a/one", b"data");
    let fs = filesystem(&store);
    store.fail_list.store(true, Ordering::Relaxed);
    assert!(fs.directory_chunk(1, "", 1000).await.is_err());
    store.fail_list.store(false, Ordering::Relaxed);
    assert_eq!(names(fs.directory_chunk(1, "", 1000).await.unwrap()), ["a"]);
    assert_eq!(
        *store.list_requests.lock().unwrap(),
        [("".into(), None), ("".into(), None)]
    );
}
