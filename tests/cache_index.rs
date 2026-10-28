use geesefs_rs::{budget::Budget, cache::ReadCache};

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
