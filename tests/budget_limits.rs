#![forbid(unsafe_code)]
use geesefs_rs::{budget::Budget, error::Error};
mod support;

#[tokio::test]
#[cfg(target_pointer_width = "64")]
async fn large_total_budget_admits_small_owners_without_allocating_the_limit() {
    let budget = Budget::new(32 * 1024 * 1024 * 1024)
        .expect("32 GiB is a permit capacity, not an allocation");
    let maximum = budget.reserve(u32::MAX as usize).await.unwrap();
    assert_eq!(budget.used(), u32::MAX as usize);
    drop(maximum);
    let first = budget.reserve(3).await.unwrap().copy(b"abc").unwrap();
    let second = budget.try_reserve(2).unwrap().copy(b"xy").unwrap();
    assert_eq!(budget.used(), 5);
    assert_eq!(first.as_slice(), b"abc");
    drop(first);
    drop(second);
    assert_eq!(budget.used(), 0);
    for size in [u32::MAX as usize + 1, u32::MAX as usize + 4] {
        assert!(
            matches!(budget.try_reserve(size), Err(Error::Budget { .. })),
            "oversized request must not wrap permits"
        );
        assert!(matches!(
            budget.reserve(size).await,
            Err(Error::Budget { .. })
        ));
        assert_eq!(budget.used(), 0);
    }
}

#[tokio::test]
#[cfg(target_pointer_width = "64")]
async fn configured_32_gib_filesystem_can_write_small_data_without_preallocation() {
    let store = support::MemoryStore::new();
    let fs = store.filesystem(32768);
    let node = fs
        .create(1, "small", libc::S_IFREG | 0o644, None, 0)
        .await
        .unwrap();
    fs.write(node.ino, 0, b"small").await.unwrap();
    fs.fsync(node.ino).await.unwrap();
    assert_eq!(store.bytes("small"), b"small");
}
