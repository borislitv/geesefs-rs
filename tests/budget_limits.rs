#![forbid(unsafe_code)]
use geesefs_rs::{budget::Budget, error::Error};
mod support;

#[test]
fn nested_views_keep_the_full_charge_until_the_last_owner_is_dropped() {
    let budget = Budget::new(8).unwrap();
    let allocation = budget.try_reserve(8).unwrap().copy(b"abcdefgh").unwrap();
    let middle = allocation.slice(2..7);
    let tail = middle.slice(2..5);
    let empty = tail.slice(1..1);
    assert_eq!(middle.as_slice(), b"cdefg");
    assert_eq!(tail.as_slice(), b"efg");
    assert_eq!(empty.as_slice(), b"");
    drop(allocation);
    drop(middle);
    drop(tail);
    assert_eq!(budget.used(), 8);
    assert!(budget.try_reserve(1).is_err());
    drop(empty);
    assert_eq!(budget.used(), 0);
    assert_eq!(
        budget
            .try_reserve(8)
            .unwrap()
            .copy(b"ABCDEFGH")
            .unwrap()
            .as_slice(),
        b"ABCDEFGH"
    );
}

#[test]
fn views_cannot_read_outside_their_parent_even_when_the_backing_allocation_is_larger() {
    let budget = Budget::new(8).unwrap();
    let allocation = budget.try_reserve(8).unwrap().copy(b"abcdefgh").unwrap();
    let view = allocation.slice(2..5);
    for (start, end) in [(0, 4), (4, 4), (2, 1)] {
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| view.slice(start..end)))
                .is_err()
        );
    }
    assert_eq!(view.as_slice(), b"cde");
    assert_eq!(budget.used(), 8);
}

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
