#![forbid(unsafe_code)]
mod support;

use geesefs_rs::{error::Error, filesystem::Filesystem};
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
use support::MemoryStore;

#[tokio::test]
async fn expired_empty_directory_backing_does_not_keep_metadata_admission_charged() {
    let store = MemoryStore::new();
    let mut config = MemoryStore::config(16);
    config.metadata_limit = Some(16 * 1024);
    // Zero TTL is the public, deterministic expiry boundary. Tokio's paused
    // clock cannot advance the std::time::Instant used by filesystem/Moka TTLs.
    config.stat_cache_ttl = Duration::ZERO;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let baseline = fs.metadata_usage().charged;

    // Different READDIR buffer sizes must not retain separate, already expired
    // empty pages. There are no child inodes, kernel refs or caller snapshots
    // here whose continued ownership could legitimately retain the backing.
    for limit in [1, 2] {
        let remainder = tokio::time::timeout(
            Duration::from_secs(3),
            fs.directory_remainder(1, "", "", limit),
        )
        .await
        .expect("empty directory listing must finish")
        .unwrap();
        assert!(remainder.entries.is_empty());
        assert!(remainder.next_after.is_none());
        drop(remainder);
    }
    assert_eq!(store.list_calls.load(Ordering::Relaxed), 2);
    assert_eq!(
        fs.metadata_usage().charged,
        baseline,
        "after the next listing/maintenance, expired empty pages must not consume metadata admission"
    );
}

#[tokio::test]
async fn positive_ttl_releases_an_old_page_when_a_different_page_is_admitted() {
    tokio::time::timeout(Duration::from_secs(15), async {
        let ttl = Duration::from_secs(1);
        let (fs, store, retained) = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let store = MemoryStore::new();
                let mut config = MemoryStore::config(16);
                config.stat_cache_ttl = ttl;
                config.metadata_limit = Some(64 * 1024);
                let fs = Filesystem::new(config, store.clone()).unwrap();
                let baseline = fs.metadata_usage().charged;
                let started = Instant::now();
                let first = fs.directory_remainder(1, "", "", 1).await.unwrap();
                assert!(first.entries.is_empty());
                assert!(first.next_after.is_none());
                drop(first);
                let retained = fs.metadata_usage().charged;

                // Establish a genuinely cached page, not an already-expired
                // publication skipped after a long scheduler pause. A late
                // setup sample is discarded with its whole fresh filesystem.
                if started.elapsed() < ttl / 2 {
                    assert!(retained > baseline, "a fresh page must retain backing");
                    break (fs, store, retained);
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("could not establish a fresh cached-page precondition");

        // Wait for actual std-clock expiry, not Tokio's virtual clock. Moka
        // 0.12.16's per-entry timer wheel has ~1.07-second ticks; allow two
        // seconds beyond the public TTL deadline before driving maintenance.
        // https://github.com/moka-rs/moka/blob/v0.12.16/src/common/timer_wheel.rs
        let maintenance_after = Instant::now() + ttl + Duration::from_secs(2);
        tokio::time::sleep_until(tokio::time::Instant::from_std(maintenance_after)).await;

        let lists = store.list_calls.load(Ordering::Relaxed);
        // Only the limit changes: empty pages have equal-sized backing. This
        // drives maintenance without replacing the first directory-page key.
        let second = fs.directory_remainder(1, "", "", 2).await.unwrap();
        assert!(second.entries.is_empty());
        assert!(second.next_after.is_none());
        drop(second);
        assert!(store.list_calls.load(Ordering::Relaxed) > lists);
        assert!(
            fs.metadata_usage().charged <= retained,
            "expired backing must not accumulate alongside the new page: before={retained}, after={}",
            fs.metadata_usage().charged
        );
    })
    .await
    .expect("real-clock expiry and subsequent listing must finish within 15 seconds");
}

#[tokio::test]
async fn an_expired_listing_response_keeps_its_inodes_until_the_caller_drops_it() {
    let store = MemoryStore::new();
    store.seed("a", b"first");
    store.seed("z", b"last");
    let mut config = MemoryStore::config(16);
    config.stat_cache_ttl = Duration::ZERO;
    config.entry_limit = 2; // Root and one child; the response owns that child.
    config.metadata_limit = Some(64 * 1024);
    let fs = Filesystem::new(config, store).unwrap();

    tokio::time::timeout(Duration::from_secs(3), async {
        let pending = fs.directory_remainder(1, "", "", 1).await.unwrap();
        assert_eq!(pending.entries.len(), 1);
        assert_eq!(pending.entries[0].name, "a");
        let ino = pending.entries[0].ino;

        // A later listing drives maintenance, but expiration must not release
        // the active response's inode ownership to make room for a lookup.
        let eof = fs.directory_remainder(1, "z", "z", 1).await.unwrap();
        assert!(eof.entries.is_empty());
        assert!(eof.next_after.is_none());
        drop(eof);
        assert!(matches!(
            fs.lookup(1, "z").await,
            Err(Error::Fs(libc::ENOMEM))
        ));
        assert_eq!(fs.node(ino).unwrap().attr().size, 5);

        let retained = fs.metadata_usage().charged;
        drop(pending);
        assert!(
            fs.metadata_usage().charged < retained,
            "dropping the caller's entries/cursor must release their own metadata charge"
        );
        let replacement = fs.lookup(1, "z").await.unwrap();
        assert_eq!(replacement.attr().size, 4);
        assert!(fs.node(ino).is_err());
    })
    .await
    .expect("admission must terminate both with and without a held response");
}
