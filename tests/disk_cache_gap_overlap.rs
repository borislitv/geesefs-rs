#![forbid(unsafe_code)]
mod support;

use geesefs_rs::{
    budget::Budget,
    disk_cache::{Demotion, DiskCache, DiskKey, PreparedDiskCache},
    filesystem::Filesystem,
    read::READ_BLOCK_BYTES as BLOCK,
};
use std::{
    os::unix::fs::PermissionsExt,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use support::MemoryStore;

const MIB: usize = 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(10);

struct Fixture {
    _directory: tempfile::TempDir,
    _mount: tempfile::TempDir,
    fs: Arc<Filesystem>,
    store: Arc<MemoryStore>,
    ino: u64,
    bytes: Vec<u8>,
    prefix: Arc<tokio::sync::Semaphore>,
    second: Arc<tokio::sync::Semaphore>,
}

async fn fixture() -> Fixture {
    fixture_with_records(1, &[2, 3, 6, 7]).await
}

async fn fixture_with_records(max_requests: usize, records: &[usize]) -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let mount = tempfile::tempdir().unwrap();
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let store = MemoryStore::new();
    *store.cache_namespace.lock().unwrap() = Some([83; 32]);
    let bytes: Vec<_> = (1..=8u8).flat_map(|byte| vec![byte; BLOCK]).collect();
    store.seed("gaps.rpm", &bytes);
    let etag = store.objects.lock().unwrap()["gaps.rpm"].etag.clone();
    let media = DiskCache::open(
        PreparedDiskCache::prepare(directory.path(), (16 * MIB) as u64).unwrap(),
        &Budget::new(32 * MIB).unwrap(),
    )
    .await
    .unwrap();
    // Two adjacent-block source runs, separated by independently verified records.
    for &index in records {
        let key = DiskKey::new(
            &[83; 32],
            b"gaps.rpm",
            etag.as_bytes(),
            bytes.len() as u64,
            (index * BLOCK) as u64,
            BLOCK,
        )
        .unwrap();
        let block = Budget::new(BLOCK)
            .unwrap()
            .try_reserve(BLOCK)
            .unwrap()
            .copy(&vec![index as u8 + 1; BLOCK])
            .unwrap();
        assert!(matches!(media.try_demote(key, block), Demotion::Admitted));
        media.drain(DEADLINE).await.unwrap();
    }
    media.close(DEADLINE).await.unwrap();
    drop(media);
    let mut config = MemoryStore::config(32);
    config.mountpoint = mount.path().into();
    config.disk_cache_dir = Some(directory.path().into());
    config.disk_cache_max_size = Some((16 * MIB) as u64);
    config.disk_cache_regex = Some(r"\.rpm$".parse().unwrap());
    config.read_ahead = 1024;
    config.read_ahead_small = 1024;
    config.read_ahead_large = 1024;
    config.read_ahead_parallel = 1024;
    config.max_requests = max_requests;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    fs.attach_disk_cache(PreparedDiskCache::prepare(directory.path(), (16 * MIB) as u64).unwrap())
        .await
        .unwrap();
    let ino = fs.lookup(1, "gaps.rpm").await.unwrap().ino;
    let prefix = Arc::new(tokio::sync::Semaphore::new(0));
    let second = Arc::new(tokio::sync::Semaphore::new(0));
    store.read_range_gates.lock().unwrap().extend([
        ((0, 2 * BLOCK), prefix.clone()),
        (((4 * BLOCK) as u64, 2 * BLOCK), second.clone()),
    ]);
    Fixture {
        _directory: directory,
        _mount: mount,
        fs,
        store,
        ino,
        bytes,
        prefix,
        second,
    }
}

#[tokio::test]
async fn pipeline_funding_is_limited_by_possible_tasks_not_the_configured_request_cap() {
    let f = fixture_with_records(4096, &[2, 3, 6, 7]).await;
    let budgets = f.fs.effective_budgets();
    assert_eq!(budgets.payload_limit_bytes, 32 * MIB);
    assert!(
        budgets.max_effective_window_bytes >= MIB,
        "one 1-MiB window must not reserve metadata for 4096 impossible active range tasks"
    );
    f.prefix.add_permits(1);
    f.second.add_permits(1);
    let data = tokio::time::timeout(DEADLINE, f.fs.read(f.ino, 0, MIB))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(data.as_slice(), f.bytes);
    assert!(f.fs.effective_budgets().data_used_bytes <= budgets.data_shared_bytes);
    assert_eq!(
        *f.store.read_ranges.lock().unwrap(),
        [(0, 2 * BLOCK), ((4 * BLOCK) as u64, 2 * BLOCK)]
    );
    drop(data);
    f.fs.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_full_gap_queue_does_not_hold_its_already_verified_disk_hit() {
    let f = fixture_with_records(1, &[1, 3, 5, 6, 7]).await;
    *f.store.read_range_gates.lock().unwrap() = [
        ((0, BLOCK), f.prefix.clone()),
        (((2 * BLOCK) as u64, BLOCK), f.second.clone()),
    ]
    .into();
    let reader = start_read(&f);
    tokio::time::timeout(DEADLINE, f.store.read_started.notified())
        .await
        .unwrap();
    // First run active, second queued, third descriptor pending: the hit that
    // closed the third run is already verified and can still be published.
    let later = tokio::time::timeout(
        Duration::from_secs(2),
        f.fs.read(f.ino, (5 * BLOCK) as u64, BLOCK),
    )
    .await;
    let stalled = f.store.read_ranges.lock().unwrap().clone();
    f.prefix.add_permits(1);
    f.second.add_permits(1);
    let data = tokio::time::timeout(DEADLINE, reader)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(data.as_slice(), f.bytes);
    let ranges = f.store.read_ranges.lock().unwrap().clone();
    f.fs.shutdown().await.unwrap();
    assert_eq!(stalled, [(0, BLOCK)]);
    assert_eq!(
        later
            .expect("descriptor backpressure must not delay its verified disk hit")
            .unwrap()
            .as_slice(),
        vec![6; BLOCK]
    );
    assert_eq!(
        ranges,
        [
            (0, BLOCK),
            ((2 * BLOCK) as u64, BLOCK),
            ((4 * BLOCK) as u64, BLOCK)
        ]
    );
}

#[tokio::test]
async fn stalled_source_gap_does_not_block_later_disk_publication_or_multiply_gets() {
    let f = fixture().await;
    let reader = tokio::spawn({
        let fs = f.fs.clone();
        let ino = f.ino;
        async move { fs.read(ino, 0, MIB).await }
    });
    tokio::time::timeout(DEADLINE, f.store.read_started.notified())
        .await
        .unwrap();
    // This is an overlapping public follower, not a private lookup-order probe.
    // Its data lie after BOTH missing runs while the first GET is still gated.
    let later = tokio::time::timeout(
        Duration::from_secs(2),
        f.fs.read(f.ino, (6 * BLOCK) as u64, BLOCK),
    )
    .await;
    let gets_while_stalled = f.store.read_ranges.lock().unwrap().clone();
    assert!(
        !reader.is_finished(),
        "required missing bytes cannot complete early"
    );
    // Always release dependency gates and join actual work before the RED assertion.
    f.prefix.add_permits(1);
    f.second.add_permits(1);
    let data = tokio::time::timeout(DEADLINE, reader)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(data.as_slice(), f.bytes);
    let ranges = f.store.read_ranges.lock().unwrap().clone();
    let later = later.map(|result| result.unwrap());
    f.fs.shutdown().await.unwrap();
    assert_eq!(
        gets_while_stalled,
        [(0, 2 * BLOCK)],
        "only one source GET may be in flight"
    );
    assert_eq!(
        ranges,
        [(0, 2 * BLOCK), ((4 * BLOCK) as u64, 2 * BLOCK)],
        "adjacent misses coalesce without fetching their cached separators"
    );
    assert_eq!(
        later
            .expect("later verified disk data must progress while the prefix GET is stalled")
            .as_slice(),
        vec![7; BLOCK]
    );
    assert_eq!(f.store.read_calls.load(Ordering::Relaxed), 2);
}

async fn later_disk_data(f: &Fixture) {
    let block = tokio::time::timeout(DEADLINE, f.fs.read(f.ino, (7 * BLOCK) as u64, BLOCK))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(block.as_slice(), vec![8; BLOCK]);
}

fn start_read(
    f: &Fixture,
) -> tokio::task::JoinHandle<geesefs_rs::error::Result<geesefs_rs::budget::Block>> {
    tokio::spawn({
        let fs = f.fs.clone();
        let ino = f.ino;
        async move { fs.read(ino, 0, MIB).await }
    })
}

#[tokio::test]
async fn required_source_error_is_preserved_and_queued_gap_is_not_started() {
    let f = fixture().await;
    let reader = start_read(&f);
    tokio::time::timeout(DEADLINE, f.store.read_started.notified())
        .await
        .unwrap();
    later_disk_data(&f).await;
    // A reachable dependency failure: the remote object disappears while GET
    // is pending. Cached suffix bytes must not turn the required prefix into success.
    f.store.objects.lock().unwrap().remove("gaps.rpm");
    f.prefix.add_permits(1);
    let result = tokio::time::timeout(DEADLINE, reader)
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(result, Err(geesefs_rs::error::Error::Fs(libc::ENOENT))),
        "expected ENOENT, got {:?}",
        result.as_ref().err()
    );
    assert_eq!(
        *f.store.read_ranges.lock().unwrap(),
        [(0, 2 * BLOCK)],
        "the first source error cancels discovery/queued work, not its error identity"
    );
    f.fs.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancellation_releases_gap_pipeline_and_does_not_start_queued_get() {
    let f = fixture().await;
    let funding = f.fs.effective_budgets().data_used_bytes;
    let reader = start_read(&f);
    tokio::time::timeout(DEADLINE, f.store.read_started.notified())
        .await
        .unwrap();
    later_disk_data(&f).await;
    reader.abort();
    assert!(matches!(reader.await, Err(error) if error.is_cancelled()));
    // A new generation must not serve the old tail. Truncate does not purge LRU
    // entries: the four already published records remain legitimate charged cache.
    f.fs.truncate(f.ino, 0).unwrap();
    f.fs.drain_disk_cache(DEADLINE).await.unwrap();
    tokio::time::timeout(DEADLINE, async {
        while f.fs.effective_budgets().data_used_bytes != funding + 4 * BLOCK {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("cancelled source/descriptor/fill owners must release their data credits");
    assert!(
        f.fs.read(f.ino, (6 * BLOCK) as u64, BLOCK)
            .await
            .unwrap()
            .as_slice()
            .is_empty()
    );
    assert_eq!(f.store.read_calls.load(Ordering::Relaxed), 1);
    assert_eq!(f.fs.disk_cache_stats().unwrap().queued_reads, 0);
    f.fs.shutdown().await.unwrap();
}

#[tokio::test]
async fn old_gap_fill_cannot_publish_over_a_concurrent_dirty_generation() {
    let f = fixture().await;
    let funding = f.fs.effective_budgets().data_used_bytes;
    let reader = start_read(&f);
    tokio::time::timeout(DEADLINE, f.store.read_started.notified())
        .await
        .unwrap();
    later_disk_data(&f).await;
    f.fs.write(f.ino, (6 * BLOCK) as u64, b"N").await.unwrap();
    assert_eq!(
        f.fs.read(f.ino, (6 * BLOCK) as u64, 1)
            .await
            .unwrap()
            .as_slice(),
        b"N"
    );
    f.prefix.add_permits(1);
    f.second.add_permits(1);
    let old = tokio::time::timeout(DEADLINE, reader)
        .await
        .unwrap()
        .unwrap();
    // The overlapping old read may finish its captured snapshot or request a
    // generation retry, but must never mix old and new bytes.
    match old {
        Ok(block) => assert_eq!(block.as_slice(), f.bytes),
        Err(geesefs_rs::error::Error::Fs(libc::EAGAIN)) => (),
        Err(error) => panic!("unexpected old-snapshot result: {error:?}"),
    }
    assert_eq!(
        f.fs.read(f.ino, (6 * BLOCK) as u64, 1)
            .await
            .unwrap()
            .as_slice(),
        b"N"
    );
    f.fs.drain_disk_cache(DEADLINE).await.unwrap();
    assert_eq!(
        f.fs.effective_budgets().data_used_bytes,
        funding + 4 * BLOCK + 1,
        "only pre-write cached records and the dirty byte remain, not late old-source backing or pipeline credit"
    );
    f.fs.shutdown().await.unwrap();
}

#[tokio::test]
async fn requested_source_prefix_is_delivered_before_a_late_gap_error() {
    let f = fixture().await;
    let (ready, mut reply) = tokio::sync::oneshot::channel();
    let reader = tokio::spawn({
        let fs = f.fs.clone();
        let ino = f.ino;
        async move {
            fs.read_reply_data(ino, 0, BLOCK, move |result| {
                ready.send(result.map(<[u8]>::to_vec)).unwrap();
            })
            .await
        }
    });
    tokio::time::timeout(DEADLINE, f.store.read_started.notified())
        .await
        .unwrap();
    later_disk_data(&f).await;
    assert!(matches!(
        reply.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    f.prefix.add_permits(1);
    tokio::time::timeout(DEADLINE, f.store.read_started.notified())
        .await
        .unwrap();
    assert_eq!(
        tokio::time::timeout(DEADLINE, reply)
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        vec![1; BLOCK]
    );
    f.second.close();
    let late = tokio::time::timeout(DEADLINE, reader)
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(late, Some(geesefs_rs::error::Error::Cancelled)),
        "{late:?}"
    );
    assert_eq!(
        *f.store.read_ranges.lock().unwrap(),
        [(0, 2 * BLOCK), ((4 * BLOCK) as u64, 2 * BLOCK)]
    );
    f.fs.shutdown().await.unwrap();
}
