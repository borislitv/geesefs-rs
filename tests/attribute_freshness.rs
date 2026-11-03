mod support;

use std::time::{Duration, Instant};
use support::MemoryStore;

#[tokio::test]
async fn cached_attributes_return_only_the_remaining_remote_freshness() {
    let store = MemoryStore::new();
    store.seed("file", b"original");
    let fs = store.filesystem(16);
    let node = fs.lookup(1, "file").await.unwrap();
    let ttl = Duration::from_secs(60);
    node.state.lock().unwrap().fetched = Instant::now() - Duration::from_secs(40);
    let (attr, remaining) = node.cached_attr(ttl);
    assert_eq!(attr.size, 8);
    assert!(remaining <= Duration::from_secs(20));
    assert!(!remaining.is_zero());
    node.state.lock().unwrap().fetched = Instant::now() - 2 * ttl;
    assert_eq!(node.cached_attr(ttl).1, Duration::ZERO);
}

#[tokio::test]
async fn unknown_attributes_are_not_advertised_as_authoritative_kernel_cache_entries() {
    let store = MemoryStore::new();
    store.seed("file", b"original");
    let fs = store.filesystem(16);
    let node = fs.lookup(1, "file").await.unwrap();
    node.state.lock().unwrap().metadata_known = false;
    assert_eq!(node.cached_attr(Duration::from_secs(60)).1, Duration::ZERO);
}

#[tokio::test]
async fn local_dirty_attributes_and_synthetic_root_remain_authoritative() {
    let store = MemoryStore::new();
    store.seed("file", b"old");
    let fs = store.filesystem(16);
    let node = fs.lookup(1, "file").await.unwrap();
    let ttl = Duration::from_secs(60);
    node.state.lock().unwrap().fetched = Instant::now() - 2 * ttl;
    fs.write(node.ino, 0, b"local dirty bytes").await.unwrap();
    let (attr, remaining) = node.cached_attr(ttl);
    assert_eq!(attr.size, 17);
    assert_eq!(remaining, ttl);
    let root = fs.node(1).unwrap();
    root.state.lock().unwrap().fetched = Instant::now() - 2 * ttl;
    assert_eq!(root.cached_attr(ttl).1, ttl);
}
