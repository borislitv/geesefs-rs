use geesefs_rs::{budget::Budget, cache::ReadCache};

#[test]
fn weighted_recency_eviction_removes_coverage_but_preserves_pinned_backings() {
    let budget = Budget::new(16).unwrap();
    let cache = ReadCache::new(8);
    cache.insert(
        1,
        1,
        0,
        budget.try_reserve(4).unwrap().copy(b"aaaa").unwrap(),
    );
    cache.insert(
        1,
        1,
        4,
        budget.try_reserve(4).unwrap().copy(b"bbbb").unwrap(),
    );
    let pinned = cache.get(1, 1, 4).unwrap();
    assert_eq!(cache.get(1, 1, 0).unwrap().as_slice(), b"aaaa");
    cache.insert(
        1,
        1,
        8,
        budget.try_reserve(4).unwrap().copy(b"cccc").unwrap(),
    );
    assert!(cache.get(1, 1, 4).is_none());
    assert!(cache.covering(1, 1, 5, 1).is_none());
    assert_eq!(pinned.as_slice(), b"bbbb");
    assert_eq!(cache.get(1, 1, 0).unwrap().as_slice(), b"aaaa");
    assert_eq!(cache.get(1, 1, 8).unwrap().as_slice(), b"cccc");
    assert_eq!(budget.used(), 12);
    drop(pinned);
    assert_eq!(budget.used(), 8);
    cache.clear();
    assert_eq!(budget.used(), 0);
}

#[test]
fn repeated_entry_evictions_do_not_accumulate_stale_byte_weight() {
    let budget = Budget::new(8).unwrap();
    let cache = ReadCache::with_entry_limit(8, 2);
    for offset in 0..8 {
        let bytes = [offset as u8, 99];
        cache.insert(
            1,
            1,
            offset,
            budget.try_reserve(2).unwrap().copy(&bytes).unwrap(),
        );
        if offset >= 2 {
            assert!(cache.get(1, 1, offset - 2).is_none());
        }
        if offset > 0 {
            assert_eq!(
                cache.get(1, 1, offset - 1).unwrap().as_slice(),
                [offset as u8 - 1, 99]
            );
        }
        assert_eq!(cache.get(1, 1, offset).unwrap().as_slice(), bytes);
    }
    assert_eq!(budget.used(), 4);
    cache.insert(
        1,
        1,
        100,
        budget.try_reserve(4).unwrap().copy(b"tail").unwrap(),
    );
    assert_eq!(cache.get(1, 1, 7).unwrap().as_slice(), [7, 99]);
    assert_eq!(cache.get(1, 1, 100).unwrap().as_slice(), b"tail");
    assert_eq!(budget.used(), 6);
}

#[test]
fn replacement_and_oversized_rejection_keep_weight_and_coverage_consistent() {
    let budget = Budget::new(24).unwrap();
    let cache = ReadCache::new(8);
    cache.insert(
        1,
        1,
        0,
        budget.try_reserve(4).unwrap().copy(b"prev").unwrap(),
    );
    cache.insert(
        2,
        1,
        0,
        budget.try_reserve(4).unwrap().copy(b"next").unwrap(),
    );
    let pinned = cache.get(1, 1, 0).unwrap();
    cache.insert(
        1,
        1,
        0,
        budget.try_reserve(6).unwrap().copy(b"larger").unwrap(),
    );
    assert_eq!(cache.covering(1, 1, 4, 2).unwrap().0.as_slice(), b"larger");
    assert!(cache.get(2, 1, 0).is_none());
    assert_eq!(budget.used(), 10);
    cache.insert(
        1,
        1,
        0,
        budget.try_reserve(9).unwrap().copy(b"oversized").unwrap(),
    );
    assert!(cache.covering(1, 1, 0, 1).is_none());
    assert_eq!(pinned.as_slice(), b"prev");
    assert_eq!(budget.used(), 4);
    drop(pinned);
    cache.insert(
        3,
        1,
        0,
        budget.try_reserve(8).unwrap().copy(b"exactly8").unwrap(),
    );
    assert_eq!(cache.get(3, 1, 0).unwrap().as_slice(), b"exactly8");
    cache.clear();
    assert_eq!(budget.used(), 0);
}

#[tokio::test]
async fn overlapping_shorter_blocks_do_not_hide_an_earlier_complete_range() {
    let budget = Budget::new(64).unwrap();
    let cache = ReadCache::new(64);
    cache.insert(
        1,
        1,
        0,
        budget
            .reserve(16)
            .await
            .unwrap()
            .copy(b"abcdefghijklmnop")
            .unwrap(),
    );
    cache.insert(
        1,
        1,
        4,
        budget.reserve(4).await.unwrap().copy(b"efgh").unwrap(),
    );

    let (block, relative) = cache.covering(1, 1, 10, 4).unwrap();
    assert_eq!(&block.as_slice()[relative..relative + 4], b"klmn");
    let (block, relative) = cache.covering(1, 1, 6, 6).unwrap();
    assert_eq!(&block.as_slice()[relative..relative + 6], b"ghijkl");
    assert!(cache.covering(1, 1, 16, 1).is_none());
    assert!(cache.covering(1, 1, 1, usize::MAX).is_none());
    assert!(cache.covering(1, 2, 10, 4).is_none());
    assert!(cache.covering(2, 1, 10, 4).is_none());
    assert_eq!(budget.used(), 20);
}

#[tokio::test]
async fn replacement_updates_coverage_without_invalidating_a_pinned_old_view() {
    let budget = Budget::new(64).unwrap();
    let cache = ReadCache::new(64);
    cache.insert(
        1,
        1,
        0,
        budget.reserve(2).await.unwrap().copy(b"ab").unwrap(),
    );
    cache.insert(
        1,
        1,
        0,
        budget
            .reserve(16)
            .await
            .unwrap()
            .copy(b"abcdefghijklmnop")
            .unwrap(),
    );
    let (pinned, relative) = cache.covering(1, 1, 10, 4).unwrap();
    assert_eq!(&pinned.as_slice()[relative..relative + 4], b"klmn");
    cache.insert(
        1,
        1,
        0,
        budget.reserve(2).await.unwrap().copy(b"XY").unwrap(),
    );
    assert!(cache.covering(1, 1, 10, 1).is_none());
    let (replacement, relative) = cache.covering(1, 1, 1, 1).unwrap();
    assert_eq!(&replacement.as_slice()[relative..relative + 1], b"Y");
    cache.clear();
    assert!(cache.covering(1, 1, 1, 1).is_none());
    drop(cache);
    assert_eq!(pinned.as_slice(), b"abcdefghijklmnop");
    assert_eq!(budget.used(), 18);
    drop(replacement);
    drop(pinned);
    assert_eq!(budget.used(), 0);
}

#[tokio::test]
async fn coverage_preserves_gaps_entry_eviction_and_the_last_representable_offset() {
    let budget = Budget::new(64).unwrap();
    let cache = ReadCache::with_entry_limit(64, 2);
    cache.insert(
        1,
        1,
        0,
        budget
            .reserve(16)
            .await
            .unwrap()
            .copy(b"abcdefghijklmnop")
            .unwrap(),
    );
    cache.insert(
        1,
        1,
        4,
        budget.reserve(4).await.unwrap().copy(b"efgh").unwrap(),
    );
    cache.insert(
        1,
        1,
        u64::MAX - 3,
        budget.reserve(4).await.unwrap().copy(b"WXYZ").unwrap(),
    );
    assert!(cache.get(1, 1, 0).is_none());
    assert!(cache.covering(1, 1, 10, 1).is_none());
    let (block, relative) = cache.covering(1, 1, u64::MAX - 1, 2).unwrap();
    assert_eq!(&block.as_slice()[relative..relative + 2], b"YZ");
    assert!(cache.covering(1, 1, u64::MAX, 2).is_none());
}
