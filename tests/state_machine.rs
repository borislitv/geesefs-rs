mod support;
use geesefs_rs::write::{MAX_IO, PART_SIZE};
use std::sync::atomic::Ordering;
use support::MemoryStore;

#[tokio::test]
async fn sparse_overwrite_truncate_and_fresh_reader_durability() {
    let store = MemoryStore::new();
    let fs = store.filesystem(32);
    let node = fs
        .create(1, "file", libc::S_IFREG | 0o640, None, 0)
        .await
        .unwrap();
    fs.write(node.ino, 3, b"abcdef").await.unwrap();
    fs.write(node.ino, 5, b"XY").await.unwrap();
    assert_eq!(
        fs.read(node.ino, 0, 20).await.unwrap().as_slice(),
        b"\0\0\0abXYef"
    );
    fs.truncate(node.ino, 4).unwrap();
    fs.truncate(node.ino, 8).unwrap();
    assert_eq!(
        fs.read(node.ino, 0, 20).await.unwrap().as_slice(),
        b"\0\0\0a\0\0\0\0"
    );
    fs.fsync(node.ino).await.unwrap();
    let fresh = store.filesystem(32);
    let inode = fresh.lookup(1, "file").await.unwrap();
    assert_eq!(
        fresh.read(inode.ino, 0, 20).await.unwrap().as_slice(),
        b"\0\0\0a\0\0\0\0"
    );
    assert_eq!(inode.attr().perm, 0o640);
}

#[tokio::test]
async fn failed_upload_never_acknowledges_or_discards_dirty_bytes() {
    let store = MemoryStore::new();
    let fs = store.filesystem(32);
    let node = fs
        .create(1, "file", libc::S_IFREG | 0o644, None, 0)
        .await
        .unwrap();
    fs.write(node.ino, 0, b"pending").await.unwrap();
    store.fail_put.store(true, Ordering::Relaxed);
    assert!(fs.fsync(node.ino).await.is_err());
    assert!(!store.objects.lock().unwrap().contains_key("file"));
    assert_eq!(
        fs.read(node.ino, 0, 7).await.unwrap().as_slice(),
        b"pending"
    );
    store.fail_put.store(false, Ordering::Relaxed);
    fs.fsync(node.ino).await.unwrap();
    assert_eq!(store.bytes("file"), b"pending");
}

#[tokio::test]
async fn multipart_completion_and_copy_errors_preserve_retryable_generation() {
    let store = MemoryStore::new();
    let fs = store.filesystem(64);
    let node = fs
        .create(1, "large", libc::S_IFREG | 0o644, None, 0)
        .await
        .unwrap();
    let block = vec![b'A'; MAX_IO];
    for i in 0..5 {
        fs.write(node.ino, i * MAX_IO as u64, &block).await.unwrap();
    }
    fs.write(node.ino, PART_SIZE as u64, b"tail").await.unwrap();
    store.fail_complete.store(true, Ordering::Relaxed);
    assert!(fs.fsync(node.ino).await.is_err());
    assert!(!store.objects.lock().unwrap().contains_key("large"));
    store.fail_complete.store(false, Ordering::Relaxed);
    fs.fsync(node.ino).await.unwrap();
    fs.write(node.ino, PART_SIZE as u64, b"TAIL").await.unwrap();
    store.fail_copy.store(true, Ordering::Relaxed);
    assert!(fs.fsync(node.ino).await.is_err());
    assert_eq!(&store.bytes("large")[PART_SIZE..], b"tail");
    store.fail_copy.store(false, Ordering::Relaxed);
    fs.fsync(node.ino).await.unwrap();
    let bytes = store.bytes("large");
    assert!(bytes[..PART_SIZE].iter().all(|b| *b == b'A'));
    assert_eq!(&bytes[PART_SIZE..], b"TAIL");
    assert!(store.copied_parts.load(Ordering::Relaxed) > 0);
}

#[tokio::test]
async fn rename_before_close_and_retryable_delete_failure() {
    let store = MemoryStore::new();
    let fs = store.filesystem(32);
    let node = fs
        .create(1, "old", libc::S_IFREG | 0o644, None, 0)
        .await
        .unwrap();
    fs.open(node.ino, true).unwrap();
    fs.write(node.ino, 0, b"renamed").await.unwrap();
    fs.rename(1, "old", 1, "new").await.unwrap();
    assert_eq!(fs.lookup(1, "new").await.unwrap().ino, node.ino);
    assert!(fs.lookup(1, "old").await.is_err());
    assert_eq!(
        fs.read(node.ino, 0, 7).await.unwrap().as_slice(),
        b"renamed"
    );
    fs.write(node.ino, 0, b"updated").await.unwrap();
    fs.fsync(node.ino).await.unwrap();
    assert_eq!(store.bytes("new"), b"updated");
    store.fail_delete.store(true, Ordering::Relaxed);
    assert!(fs.unlink(1, "new", false).await.is_err());
    assert!(fs.lookup(1, "new").await.is_ok());
    store.fail_delete.store(false, Ordering::Relaxed);
    fs.unlink(1, "new", false).await.unwrap();
    assert!(!store.objects.lock().unwrap().contains_key("new"));
    assert!(fs.write(node.ino, 0, b"invisible").await.is_err());
    fs.release(node.ino);
}

#[tokio::test]
async fn paged_implicit_directories_symlinks_and_metadata() {
    let store = MemoryStore::new();
    store.page_size.store(2, Ordering::Relaxed);
    for name in ["dir/a", "dir/b", "dir/c", "dir/sub/d"] {
        store.seed(name, b"data");
    }
    let fs = store.filesystem(32);
    let directory = fs.lookup(1, "dir").await.unwrap();
    let first = fs.directory_chunk(directory.ino, "", 2).await.unwrap();
    assert_eq!(
        first.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        ["a", "b"]
    );
    let second = fs.directory_chunk(directory.ino, "b", 2).await.unwrap();
    assert_eq!(
        second.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        ["c", "sub"]
    );
    let link = fs
        .create(1, "link", libc::S_IFLNK | 0o777, Some(b"dir/a"), 0)
        .await
        .unwrap();
    fs.setxattr(link.ino, "user.custom", Some(b"value%\n"), 0)
        .unwrap();
    fs.fsync(link.ino).await.unwrap();
    let fresh = store.filesystem(32);
    let loaded = fresh.lookup(1, "link").await.unwrap();
    assert_eq!(loaded.attr().kind, fuser::FileType::Symlink);
    assert_eq!(
        fresh.xattrs(loaded.ino).unwrap()["user.custom"],
        b"value%\n"
    );
}

#[tokio::test]
async fn paged_directory_rename_retains_dirty_live_child_and_resumes_partial_failure() {
    let store = MemoryStore::new();
    store.page_size.store(1, Ordering::Relaxed);
    store.seed("old/a", b"base");
    store.seed("old/b", b"second");
    store.seed("old-keep", b"outside subtree");
    let fs = store.filesystem(32);
    fs.lookup(1, "old-keep").await.unwrap();
    let directory = fs.lookup(1, "old").await.unwrap();
    let child = fs.lookup(directory.ino, "a").await.unwrap();
    fs.reference(&child);
    fs.write(child.ino, 0, b"NEXT").await.unwrap();
    store.fail_copy_after.store(1, Ordering::Relaxed);
    assert!(fs.rename(1, "old", 1, "new").await.is_err());
    assert_eq!(store.bytes("new/a"), b"base");
    assert!(store.objects.lock().unwrap().contains_key("old/b"));
    store.fail_copy_after.store(usize::MAX, Ordering::Relaxed);
    fs.rename(1, "old", 1, "new").await.unwrap();
    assert_eq!(fs.lookup(1, "new").await.unwrap().ino, directory.ino);
    assert_eq!(fs.lookup(directory.ino, "a").await.unwrap().ino, child.ino);
    assert_eq!(fs.read(child.ino, 0, 4).await.unwrap().as_slice(), b"NEXT");
    fs.fsync(child.ino).await.unwrap();
    assert_eq!(store.bytes("new/a"), b"NEXT");
    assert_eq!(store.bytes("new/b"), b"second");
    assert!(
        !store
            .objects
            .lock()
            .unwrap()
            .keys()
            .any(|key| key.starts_with("old/"))
    );
}

#[tokio::test]
async fn read_ahead_hits_are_owned_and_writes_do_not_return_stale_cached_bytes() {
    let store = MemoryStore::new();
    store.seed("file", &vec![b'A'; 1024 * 1024]);
    let fs = store.filesystem(32);
    let node = fs.lookup(1, "file").await.unwrap();
    assert_eq!(fs.read(node.ino, 0, 4).await.unwrap().as_slice(), b"AAAA");
    let reads = store.read_calls.load(Ordering::Relaxed);
    assert_eq!(fs.read(node.ino, 4, 4).await.unwrap().as_slice(), b"AAAA");
    assert_eq!(store.read_calls.load(Ordering::Relaxed), reads);
    fs.write(node.ino, 4, b"NEXT").await.unwrap();
    assert_eq!(fs.read(node.ino, 4, 4).await.unwrap().as_slice(), b"NEXT");
    fs.fsync(node.ino).await.unwrap();
    assert_eq!(fs.read(node.ino, 4, 4).await.unwrap().as_slice(), b"NEXT");
}

#[tokio::test]
async fn evicting_replaced_inode_does_not_forget_live_destination_identity() {
    let store = MemoryStore::new();
    let mut config = MemoryStore::config(32);
    config.entry_limit = 3;
    let fs = geesefs_rs::filesystem::Filesystem::new(config, store.clone()).unwrap();
    let source = fs
        .create(1, "source", libc::S_IFREG | 0o644, None, 0)
        .await
        .unwrap();
    fs.reference(&source);
    fs.write(source.ino, 0, b"source").await.unwrap();
    let target = fs
        .create(1, "target", libc::S_IFREG | 0o644, None, 0)
        .await
        .unwrap();
    fs.fsync(target.ino).await.unwrap();
    drop(target);
    fs.rename(1, "source", 1, "target").await.unwrap();
    fs.create(1, "other", libc::S_IFREG | 0o644, None, 0)
        .await
        .unwrap();
    assert_eq!(fs.lookup(1, "target").await.unwrap().ino, source.ino);
}
