#![forbid(unsafe_code)]
mod support;

use geesefs_rs::{
    config::Config, disk_cache::PreparedDiskCache, filesystem::Filesystem, read::READ_BLOCK_BYTES,
};
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use support::MemoryStore;

const MIB: usize = 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(10);

fn configured(dir: &std::path::Path, mount: &std::path::Path) -> Config {
    let mut config = MemoryStore::config(32);
    config.mountpoint = mount.into();
    config.disk_cache_dir = Some(dir.into());
    config.disk_cache_max_size = Some(16 * MIB as u64);
    config.disk_cache_regex = Some(r"\.(deb|rpm)$".parse().unwrap());
    config.entry_limit = 32;
    config.read_ahead = 128;
    config.read_ahead_small = 128;
    config.read_ahead_large = 128;
    config
}

async fn attach(config: Config, store: Arc<MemoryStore>) -> Arc<Filesystem> {
    use std::os::unix::fs::PermissionsExt;
    let disk = config.disk_cache_config().unwrap().unwrap();
    std::fs::set_permissions(&disk.directory, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut prepared = PreparedDiskCache::prepare(&disk.directory, disk.max_bytes).unwrap();
    let fs = Filesystem::new(config, store).unwrap();
    if fs.disk_cache_bypass_reason().is_none() && !disk.directory.join("payload-v1.cache").exists()
    {
        // These read tests require an already Ready tier. Seed empty persistent
        // media explicitly; fresh deferred attachment is covered separately.
        let resources = geesefs_rs::budget::Budget::new(32 * MIB).unwrap();
        let cache = geesefs_rs::disk_cache::DiskCache::open(prepared, &resources)
            .await
            .unwrap();
        cache.close(DEADLINE).await.unwrap();
        drop(cache);
        prepared = PreparedDiskCache::prepare(&disk.directory, disk.max_bytes).unwrap();
    }
    fs.attach_disk_cache(prepared).await.unwrap();
    fs
}

fn scoped_store() -> Arc<MemoryStore> {
    let store = MemoryStore::new();
    *store.cache_namespace.lock().unwrap() = Some([7; 32]);
    store
}

#[tokio::test]
async fn source_population_survives_brief_namespace_contention() {
    for recovered in [false, true] {
        population_with_namespace_contention(recovered, "release").await;
    }
}

#[tokio::test]
async fn multichunk_source_reply_does_not_wait_for_contended_population() {
    let dir = tempfile::tempdir().unwrap();
    let mount = tempfile::tempdir().unwrap();
    let store = scoped_store();
    let bytes: Vec<_> = (0..2 * READ_BLOCK_BYTES).map(|i| (i % 251) as u8).collect();
    store.seed("package.rpm", &bytes);
    let mut config = configured(dir.path(), mount.path());
    config.read_ahead = 256;
    config.read_ahead_small = 256;
    config.read_ahead_large = 256;
    config.read_ahead_parallel = 128;
    let fs = attach(config, store.clone()).await;
    let ino = fs.lookup(1, "package.rpm").await.unwrap().ino;
    let source = Arc::new(tokio::sync::Semaphore::new(0));
    *store.read_gate.lock().unwrap() = Some(source.clone());
    let (delivered, delivery) = tokio::sync::oneshot::channel();
    let mut reader = tokio::spawn({
        let fs = fs.clone();
        async move {
            let observer = fs.clone();
            fs.read_reply_data(ino, 0, 2 * READ_BLOCK_BYTES, move |result| {
                delivered
                    .send((
                        result.unwrap().to_vec(),
                        observer.disk_read_decisions()
                            [geesefs_rs::read::DiskDecision::NamespaceContended as usize],
                    ))
                    .unwrap();
            })
            .await
        }
    });
    // Both independent source producers have passed required-read qualification,
    // but neither can deliver its requested chunk until the source gate opens.
    tokio::time::timeout(DEADLINE, async {
        while store.read_calls.load(Ordering::Relaxed) != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let (locked, lock_ready) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel();
    let locker = std::thread::spawn({
        let fs = fs.clone();
        move || {
            let _namespace = fs.namespace.lock().unwrap();
            locked.send(()).unwrap();
            released.recv_timeout(DEADLINE).unwrap();
        }
    });
    lock_ready.recv_timeout(DEADLINE).unwrap();
    source.add_permits(2);
    let delivery = tokio::time::timeout(DEADLINE, delivery).await;
    let observed = tokio::time::timeout(DEADLINE, async {
        while !reader.is_finished()
            && fs.disk_read_decisions()[geesefs_rs::read::DiskDecision::NamespaceContended as usize]
                == 0
        {
            tokio::task::yield_now().await;
        }
    })
    .await;
    release.send(()).unwrap();
    locker.join().unwrap();
    observed.unwrap();
    let (actual, population_waits_before_reply) = delivery.unwrap().unwrap();
    assert_eq!(actual, bytes);
    assert_eq!(
        population_waits_before_reply, 0,
        "optional population must not consume its wait allowance before the remaining requested source chunks reach the reply"
    );
    assert!(
        tokio::time::timeout(DEADLINE, &mut reader)
            .await
            .unwrap()
            .unwrap()
            .is_none()
    );
    fs.drain_disk_cache(DEADLINE).await.unwrap();
    let gets = store.read_calls.load(Ordering::Relaxed);
    let cached = fs.read(ino, 0, bytes.len()).await.unwrap();
    assert_eq!(cached.as_slice(), bytes);
    assert_eq!(store.read_calls.load(Ordering::Relaxed), gets);
    drop(cached);
    fs.shutdown().await.unwrap();
}

#[tokio::test]
async fn source_reply_and_fill_completion_are_bounded_while_population_is_contended() {
    population_with_namespace_contention(true, "deadline").await;
}

#[tokio::test]
async fn cancelling_contended_population_releases_its_existing_fill_ownership() {
    population_with_namespace_contention(true, "cancel").await;
}

async fn population_with_namespace_contention(recovered: bool, finish: &str) {
    let dir = tempfile::tempdir().unwrap();
    let mount = tempfile::tempdir().unwrap();
    let store = scoped_store();
    let bytes: Vec<_> = (0..READ_BLOCK_BYTES).map(|i| (i % 251) as u8).collect();
    store.seed("package.rpm", &bytes);
    let config = configured(dir.path(), mount.path());
    let fs = if recovered {
        attach(config, store.clone()).await
    } else {
        let fs = Filesystem::new(config, store.clone()).unwrap();
        fs.attach_disk_cache(PreparedDiskCache::prepare(dir.path(), 16 * MIB as u64).unwrap())
            .await
            .unwrap();
        fs
    };
    let node = fs.lookup(1, "package.rpm").await.unwrap();
    let ino = node.ino;
    let (hold, held) = std::sync::mpsc::channel();
    let (locked, lock_ready) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel();
    let locker = std::thread::spawn({
        let fs = fs.clone();
        move || {
            held.recv_timeout(DEADLINE).unwrap();
            let _namespace = fs.namespace.lock().unwrap();
            locked.send(()).unwrap();
            released.recv_timeout(DEADLINE).unwrap();
        }
    });
    let (delivered, delivery) = tokio::sync::oneshot::channel();
    let mut reader = tokio::spawn({
        let fs = fs.clone();
        async move {
            fs.read_reply_data(ino, 0, READ_BLOCK_BYTES, move |result| {
                let bytes = result.unwrap().to_vec();
                // Like a concurrent namespace metadata writer: begin only
                // after the source has delivered its verified reply, before
                // the same fill performs optional population.
                hold.send(()).unwrap();
                lock_ready.recv_timeout(DEADLINE).unwrap();
                delivered.send(bytes).unwrap();
            })
            .await
        }
    });
    let delivered = tokio::time::timeout(DEADLINE, delivery)
        .await
        .unwrap()
        .unwrap();
    let observed = tokio::time::timeout(DEADLINE, async {
        while !reader.is_finished()
            && fs.disk_read_decisions()[geesefs_rs::read::DiskDecision::NamespaceContended as usize]
                == 0
        {
            tokio::task::yield_now().await;
        }
    })
    .await;
    let completed_while_locked = if finish == "release" {
        None
    } else {
        if finish == "cancel" {
            reader.abort();
        }
        // The production population allowance remains 250 ms. Keep the
        // metadata owner held until completion/cancellation: neither may
        // wait indefinitely for that owner or retract the successful reply.
        Some(tokio::time::timeout(Duration::from_secs(1), &mut reader).await)
    };
    release.send(()).unwrap();
    locker.join().unwrap();
    observed.unwrap();
    assert_eq!(delivered, bytes);
    let completed = match completed_while_locked {
        Some(completed) => completed.unwrap(),
        None => tokio::time::timeout(DEADLINE, reader).await.unwrap(),
    };
    if finish == "cancel" {
        assert!(completed.unwrap_err().is_cancelled());
    } else {
        assert!(completed.unwrap().is_none());
    }
    // A delivered source reply does not promise persistent publication.
    // Finish the fill, then use the public bounded publication barrier.
    fs.drain_disk_cache(DEADLINE).await.unwrap();
    assert_eq!(
        fs.effective_budgets().data_used_bytes,
        fs.effective_budgets().disk_cache_funding_bytes,
        "completed/cancelled population must not retain source or waiter owners"
    );
    let gets = store.read_calls.load(Ordering::Relaxed);
    assert_eq!(gets, 1);
    let cached = fs.read(node.ino, 0, bytes.len()).await.unwrap();
    assert_eq!(cached.as_slice(), bytes);
    assert_eq!(
        store.read_calls.load(Ordering::Relaxed),
        if finish == "release" { gets } else { gets + 1 },
        "brief metadata contention must not leave a hole in eligible L2 population; recovered={recovered}"
    );
    drop(cached);
    fs.shutdown().await.unwrap();
}

#[tokio::test]
async fn changed_clean_eof_during_window_admission_releases_the_old_plan() {
    for enabled in [true, false] {
        for new_size in [128 * 1024, 64 * 1024, 32 * 1024, 0] {
            let dir = tempfile::tempdir().unwrap();
            let mount = tempfile::tempdir().unwrap();
            let store = scoped_store();
            store.seed("target.rpm", &vec![0x42; 64 * 1024]);
            let mut config = configured(dir.path(), mount.path());
            let fs = if enabled {
                attach(config, store.clone()).await
            } else {
                config.disk_cache_dir = None;
                config.disk_cache_max_size = None;
                config.disk_cache_regex = None;
                Filesystem::new(config, store.clone()).unwrap()
            };
            let target = fs.lookup(1, "target.rpm").await.unwrap();
            let old_etag = store.objects.lock().unwrap()["target.rpm"].etag.clone();
            // Occupy the real shared lane, leaving enough for the initial reply
            // check but not the atomic source/reply/disk window reservation.
            for index in 0..32 {
                let budgets = fs.effective_budgets();
                let free = budgets.data_shared_bytes - budgets.data_used_bytes;
                if free <= 50 * 1024 {
                    break;
                }
                let amount = (free - 50 * 1024).min(MIB);
                let holder = fs
                    .create(
                        1,
                        &format!("holder-{index}"),
                        libc::S_IFREG | 0o644,
                        None,
                        0,
                    )
                    .await
                    .unwrap();
                fs.write(holder.ino, 0, &vec![0x71; amount]).await.unwrap();
            }
            let (entered, release) = store.pause_put();
            let reader = tokio::spawn({
                let fs = fs.clone();
                let ino = target.ino;
                async move { fs.read(ino, 0, 64 * 1024).await }
            });
            tokio::time::timeout(DEADLINE, entered)
                .await
                .unwrap()
                .unwrap();
            // Admission is waiting inside a holder flush. Publish a different
            // clean generation, including unchanged-size/changed-ETag coverage.
            if new_size == 64 * 1024 {
                fs.write(target.ino, 0, b"N").await.unwrap();
            } else {
                fs.truncate(target.ino, new_size).unwrap();
            }
            tokio::time::timeout(DEADLINE, fs.fsync(target.ino))
                .await
                .unwrap()
                .unwrap();
            assert_ne!(store.objects.lock().unwrap()["target.rpm"].etag, old_etag);
            release.send(()).unwrap();
            let joined = tokio::time::timeout(DEADLINE, reader).await.unwrap();
            let result = joined.expect("a clean generation change during admission must not panic");
            let mut expected = vec![b'B'; (new_size as usize).min(64 * 1024)];
            if new_size == 64 * 1024 {
                expected[0] = b'N';
            }
            match result {
                Err(geesefs_rs::error::Error::Fs(libc::EAGAIN)) => (),
                Ok(block) if !enabled => assert_eq!(block.as_slice(), expected),
                Ok(_) => panic!("an obsolete L2 plan must request a retry: new_size={new_size}"),
                Err(error) => panic!("unexpected read error: {error:?}"),
            }
            let funding = fs.effective_budgets().disk_cache_funding_bytes;
            assert_eq!(
                fs.effective_budgets().data_used_bytes,
                funding,
                "obsolete admission/fill must release its source, reply and disk reservations"
            );
            let block = tokio::time::timeout(DEADLINE, fs.read(target.ino, 0, 64 * 1024))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(block.as_slice(), expected);
            drop(block);
            fs.shutdown().await.unwrap();
        }
    }
}

#[tokio::test]
async fn excluded_and_disabled_reads_do_not_populate_disk() {
    for enabled in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mount = tempfile::tempdir().unwrap();
        let store = scoped_store();
        store.seed("package.txt", &vec![0x2a; READ_BLOCK_BYTES]);
        let mut config = configured(dir.path(), mount.path());
        let fs = if enabled {
            attach(config, store.clone()).await
        } else {
            config.disk_cache_dir = None;
            config.disk_cache_max_size = None;
            config.disk_cache_regex = None;
            Filesystem::new(config, store.clone()).unwrap()
        };
        let node = fs.lookup(1, "package.txt").await.unwrap();
        assert!(
            fs.read(node.ino, 0, READ_BLOCK_BYTES)
                .await
                .unwrap()
                .as_slice()
                .iter()
                .all(|byte| *byte == 0x2a)
        );
        fs.drain_disk_cache(DEADLINE).await.unwrap();
        let gets = store.read_calls.load(Ordering::Relaxed);
        assert!(
            fs.read(node.ino, 0, READ_BLOCK_BYTES)
                .await
                .unwrap()
                .as_slice()
                .iter()
                .all(|byte| *byte == 0x2a)
        );
        assert_eq!(store.read_calls.load(Ordering::Relaxed), gets + 1);
        if enabled {
            assert_eq!(fs.disk_cache_stats().unwrap().index_entries, 0);
        } else {
            assert!(fs.disk_cache_stats().is_none());
            assert!(fs.disk_cache_state().is_none());
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
            assert_eq!(fs.effective_budgets().disk_cache_funding_bytes, 0);
        }
        fs.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn unaligned_disk_hits_release_decoded_backings_and_never_repopulate_fragments() {
    let dir = tempfile::tempdir().unwrap();
    let mount = tempfile::tempdir().unwrap();
    let store = scoped_store();
    let bytes: Vec<u8> = (0..READ_BLOCK_BYTES).map(|i| (i % 251) as u8).collect();
    store.seed("package.rpm", &bytes);
    let mut config = configured(dir.path(), mount.path());
    let fs = attach(config.clone(), store.clone()).await;
    let node = fs.lookup(1, "package.rpm").await.unwrap();
    assert_eq!(
        fs.read(node.ino, 0, bytes.len()).await.unwrap().as_slice(),
        bytes
    );
    fs.drain_disk_cache(DEADLINE).await.unwrap();
    assert_eq!(fs.disk_cache_stats().unwrap().index_entries, 1);
    fs.shutdown().await.unwrap();
    drop(fs);

    config.read_ahead = 0;
    config.read_ahead_small = 0;
    config.read_ahead_large = 0;
    let fs = attach(config.clone(), store.clone()).await;
    let node = fs.lookup(1, "package.rpm").await.unwrap();
    let gets = store.read_calls.load(Ordering::Relaxed);
    let before = fs.disk_cache_stats().unwrap();
    assert_eq!(
        fs.read(node.ino, 11, 37).await.unwrap().as_slice(),
        &bytes[11..48]
    );
    assert_eq!(store.read_calls.load(Ordering::Relaxed), gets);
    assert_eq!(fs.disk_cache_stats().unwrap().hits, before.hits + 1);
    assert_eq!(
        fs.disk_cache_read_bytes(),
        37,
        "reply/window delivery stays sliced"
    );
    assert_eq!(
        fs.effective_budgets().data_used_bytes,
        fs.effective_budgets().disk_cache_funding_bytes,
        "the validated record must release its decoded allocation after the reply"
    );
    let loaded = fs.disk_cache_stats().unwrap();
    fs.drain_disk_cache(DEADLINE).await.unwrap();
    let evicted = fs.disk_cache_stats().unwrap();
    assert_eq!(
        evicted.engine_admitted_demotions, loaded.engine_admitted_demotions,
        "reusing a validated backing must not admit a new fragment"
    );
    assert_eq!(evicted.io_write_bytes, loaded.io_write_bytes);
    assert_eq!(evicted.index_entries, loaded.index_entries);
    let gets = store.read_calls.load(Ordering::Relaxed);
    assert_eq!(
        fs.read(node.ino, 0, bytes.len()).await.unwrap().as_slice(),
        bytes
    );
    assert_eq!(store.read_calls.load(Ordering::Relaxed), gets);
    assert!(
        fs.disk_cache_stats().unwrap().hits > evicted.hits,
        "independent reuse must execute a new L2 read"
    );
    let disk_bytes = fs.disk_cache_read_bytes();
    assert_eq!(
        fs.read(node.ino, 65547, 37).await.unwrap().as_slice(),
        &bytes[65547..65584]
    );
    assert_eq!(
        fs.disk_cache_read_bytes(),
        disk_bytes + 37,
        "only the independent requested span is delivered from the canonical L2 record"
    );
    fs.shutdown().await.unwrap();
    drop(fs);

    let fs = attach(config, store.clone()).await;
    let node = fs.lookup(1, "package.rpm").await.unwrap();
    let gets = store.read_calls.load(Ordering::Relaxed);
    assert_eq!(
        fs.read(node.ino, 0, bytes.len()).await.unwrap().as_slice(),
        bytes
    );
    assert_eq!(store.read_calls.load(Ordering::Relaxed), gets);
    fs.shutdown().await.unwrap();
}

#[tokio::test]
async fn restart_reuses_stable_identity_not_inode_or_read_shape() {
    let dir = tempfile::tempdir().unwrap();
    let mount = tempfile::tempdir().unwrap();
    let store = scoped_store();
    store.seed("noise", b"noise");
    store.seed("package.rpm", &vec![0x39; READ_BLOCK_BYTES]);
    let config = configured(dir.path(), mount.path());
    let fs = attach(config.clone(), store.clone()).await;
    let _noise = fs.lookup(1, "noise").await.unwrap();
    let node = fs.lookup(1, "package.rpm").await.unwrap();
    let old_ino = node.ino;
    fs.read(node.ino, 0, READ_BLOCK_BYTES).await.unwrap();
    fs.drain_disk_cache(DEADLINE).await.unwrap();
    assert!(fs.disk_cache_stats().unwrap().index_entries > 0);
    fs.shutdown().await.unwrap();
    drop(fs);
    let mut config = config;
    config.read_ahead = 0;
    config.read_ahead_small = 0;
    config.read_ahead_large = 0;
    let fs = attach(config, store.clone()).await;
    let node = fs.lookup(1, "package.rpm").await.unwrap();
    assert_ne!(node.ino, old_ino);
    let gets = store.read_calls.load(Ordering::Relaxed);
    assert_eq!(
        fs.read(node.ino, 11, 37).await.unwrap().as_slice(),
        &[0x39; 37]
    );
    assert_eq!(store.read_calls.load(Ordering::Relaxed), gets);
    // Canonical disk lookup covers an exact unaligned span without delivering
    // an unnecessary prefix or issuing a source GET.
    assert_eq!(fs.disk_cache_read_bytes(), 37);
    fs.shutdown().await.unwrap();
}

#[tokio::test]
async fn cold_unaligned_zero_read_ahead_fetches_only_requested_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let mount = tempfile::tempdir().unwrap();
    let store = scoped_store();
    let bytes: Vec<u8> = (0..READ_BLOCK_BYTES).map(|i| (i % 251) as u8).collect();
    store.seed("package.rpm", &bytes);
    let mut config = configured(dir.path(), mount.path());
    config.read_ahead = 0;
    config.read_ahead_small = 0;
    config.read_ahead_large = 0;
    let fs = attach(config, store.clone()).await;
    let node = fs.lookup(1, "package.rpm").await.unwrap();
    assert_eq!(fs.disk_cache_stats().unwrap().index_entries, 0);
    assert_eq!(
        tokio::time::timeout(DEADLINE, fs.read(node.ino, 11, 37))
            .await
            .unwrap()
            .unwrap()
            .as_slice(),
        &bytes[11..48]
    );
    fs.drain_disk_cache(DEADLINE).await.unwrap();
    fs.shutdown().await.unwrap();
    assert_eq!(
        *store.read_ranges.lock().unwrap(),
        [(11, 37)],
        "a cold L2 miss must not download an aligned prefix when read-ahead is zero"
    );
}

#[tokio::test]
async fn neighboring_zero_read_ahead_miss_does_not_refetch_the_completed_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let mount = tempfile::tempdir().unwrap();
    let store = scoped_store();
    let bytes: Vec<u8> = (0..READ_BLOCK_BYTES).map(|i| (i % 251) as u8).collect();
    store.seed("package.rpm", &bytes);
    let mut config = configured(dir.path(), mount.path());
    config.read_ahead = 0;
    config.read_ahead_small = 0;
    config.read_ahead_large = 0;
    let fs = attach(config, store.clone()).await;
    let node = fs.lookup(1, "package.rpm").await.unwrap();
    for offset in [0, 4096] {
        assert_eq!(
            tokio::time::timeout(DEADLINE, fs.read(node.ino, offset as u64, 4096))
                .await
                .unwrap()
                .unwrap()
                .as_slice(),
            &bytes[offset..offset + 4096]
        );
        // Complete population before the next independent read: this is not
        // an overlapping in-flight subscriber or an uncommitted disk fill.
        fs.drain_disk_cache(DEADLINE).await.unwrap();
    }
    fs.shutdown().await.unwrap();
    assert_eq!(
        *store.read_ranges.lock().unwrap(),
        [(0, 4096), (4096, 4096)],
        "the neighboring miss must not download the preceding completed fragment again"
    );
}

#[tokio::test]
async fn exact_fragment_and_eof_coverage_never_invents_a_hole_or_extends_get() {
    let dir = tempfile::tempdir().unwrap();
    let mount = tempfile::tempdir().unwrap();
    let store = scoped_store();
    let mut bytes = vec![0x4c; READ_BLOCK_BYTES];
    bytes.extend_from_slice(b"tail-07");
    store.seed("package.deb", &bytes);
    let mut config = configured(dir.path(), mount.path());
    config.read_ahead = 0;
    config.read_ahead_small = 0;
    config.read_ahead_large = 0;
    let fs = attach(config, store.clone()).await;
    let node = fs.lookup(1, "package.deb").await.unwrap();
    assert_eq!(
        fs.read(node.ino, 0, 17).await.unwrap().as_slice(),
        &[0x4c; 17]
    );
    assert_eq!(
        fs.read(node.ino, READ_BLOCK_BYTES as u64, 128)
            .await
            .unwrap()
            .as_slice(),
        b"tail-07"
    );
    assert_eq!(
        *store.read_ranges.lock().unwrap(),
        [(0, 17), (READ_BLOCK_BYTES as u64, 7)]
    );
    fs.drain_disk_cache(DEADLINE).await.unwrap();
    let gets = store.read_calls.load(Ordering::Relaxed);
    assert_eq!(
        fs.read(node.ino, 0, 17).await.unwrap().as_slice(),
        &[0x4c; 17]
    );
    assert_eq!(
        fs.read(node.ino, READ_BLOCK_BYTES as u64, 128)
            .await
            .unwrap()
            .as_slice(),
        b"tail-07"
    );
    assert!(
        fs.read(node.ino, bytes.len() as u64, 10)
            .await
            .unwrap()
            .as_slice()
            .is_empty()
    );
    assert_eq!(store.read_calls.load(Ordering::Relaxed), gets);
    let loaded = fs.disk_cache_stats().unwrap();
    assert_eq!(
        fs.read(node.ino, 11, 6).await.unwrap().as_slice(),
        &[0x4c; 6]
    );
    fs.drain_disk_cache(DEADLINE).await.unwrap();
    assert_eq!(
        fs.disk_cache_stats().unwrap().engine_admitted_demotions,
        loaded.engine_admitted_demotions + 1
    );
    // Exact fragment identities do not alias the existing (0, 17) record.
    // The new request fetches only its own bytes and then reuses that fragment.
    assert_eq!(store.read_ranges.lock().unwrap().last(), Some(&(11, 6)));
    let gets = store.read_calls.load(Ordering::Relaxed);
    assert_eq!(
        fs.read(node.ino, 11, 6).await.unwrap().as_slice(),
        &[0x4c; 6]
    );
    assert_eq!(store.read_calls.load(Ordering::Relaxed), gets);
    assert_eq!(
        fs.read(node.ino, 0, 18).await.unwrap().as_slice(),
        &[0x4c; 18]
    );
    assert_eq!(
        store.read_calls.load(Ordering::Relaxed),
        gets + 1,
        "a 17-byte record does not cover byte 18"
    );
    assert_eq!(store.read_ranges.lock().unwrap().last(), Some(&(0, 18)));
    fs.shutdown().await.unwrap();
}

#[tokio::test]
async fn persistent_identity_separates_etag_size_and_backend_but_not_metadata() {
    for change in ["etag", "size", "backend", "metadata"] {
        let dir = tempfile::tempdir().unwrap();
        let mount = tempfile::tempdir().unwrap();
        let store = scoped_store();
        store.seed("package.rpm", &vec![0x32; READ_BLOCK_BYTES]);
        let config = configured(dir.path(), mount.path());
        let fs = attach(config.clone(), store.clone()).await;
        let node = fs.lookup(1, "package.rpm").await.unwrap();
        fs.read(node.ino, 0, READ_BLOCK_BYTES).await.unwrap();
        fs.drain_disk_cache(DEADLINE).await.unwrap();
        fs.shutdown().await.unwrap();
        drop(fs);
        match change {
            "etag" => store.seed("package.rpm", &vec![0x48; READ_BLOCK_BYTES]),
            "size" => store
                .objects
                .lock()
                .unwrap()
                .get_mut("package.rpm")
                .unwrap()
                .bytes
                .truncate(31),
            "backend" => *store.cache_namespace.lock().unwrap() = Some([8; 32]),
            "metadata" => {
                store
                    .objects
                    .lock()
                    .unwrap()
                    .get_mut("package.rpm")
                    .unwrap()
                    .metadata
                    .insert("label".into(), "changed".into());
            }
            _ => unreachable!(),
        }
        let fs = attach(config, store.clone()).await;
        assert_eq!(
            fs.effective_budgets().data_used_bytes,
            fs.disk_cache_funding().unwrap().required_bytes
        );
        let node = fs.lookup(1, "package.rpm").await.unwrap();
        let gets = store.read_calls.load(Ordering::Relaxed);
        if change == "metadata" {
            assert_eq!(&fs.xattrs(node.ino).unwrap()["user.label"], b"changed");
        }
        let block = fs.read(node.ino, 11, 37).await.unwrap();
        let expected_byte = if change == "etag" { 0x48 } else { 0x32 };
        let expected_length = if change == "size" { 20 } else { 37 };
        assert_eq!(block.as_slice(), vec![expected_byte; expected_length]);
        assert_eq!(
            store.read_calls.load(Ordering::Relaxed),
            gets + usize::from(change != "metadata"),
            "change={change}"
        );
        drop(block);
        fs.drain_disk_cache(DEADLINE).await.unwrap();
        let gets = store.read_calls.load(Ordering::Relaxed);
        let block = fs.read(node.ino, 0, READ_BLOCK_BYTES).await.unwrap();
        let expected_length = if change == "size" {
            31
        } else {
            READ_BLOCK_BYTES
        };
        assert_eq!(block.as_slice(), vec![expected_byte; expected_length]);
        assert_eq!(
            store.read_calls.load(Ordering::Relaxed),
            gets + usize::from(change == "size"),
            "an EOF-clipped exact fragment at offset 11 does not cover the preceding bytes"
        );
        if change == "size" {
            assert_eq!(store.read_ranges.lock().unwrap().last(), Some(&(0, 31)));
        }
        fs.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn dirty_sparse_and_namespace_changes_do_not_reuse_old_disk_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let mount = tempfile::tempdir().unwrap();
    let store = scoped_store();
    store.seed("package.rpm", &vec![0x35; READ_BLOCK_BYTES]);
    let fs = attach(configured(dir.path(), mount.path()), store.clone()).await;
    let node = fs.lookup(1, "package.rpm").await.unwrap();
    fs.read(node.ino, 0, READ_BLOCK_BYTES).await.unwrap();
    fs.drain_disk_cache(DEADLINE).await.unwrap();
    let hits = fs.disk_cache_stats().unwrap().hits;
    fs.write(node.ino, 2, b"XY").await.unwrap();
    assert_eq!(
        fs.read(node.ino, 0, 7).await.unwrap().as_slice(),
        b"55XY555"
    );
    fs.truncate(node.ino, 4).unwrap();
    fs.truncate(node.ino, 8).unwrap();
    assert_eq!(
        fs.read(node.ino, 0, 8).await.unwrap().as_slice(),
        b"55XY\0\0\0\0"
    );
    assert_eq!(fs.disk_cache_stats().unwrap().hits, hits);
    store.fail_put.store(true, Ordering::Relaxed);
    assert!(fs.fsync(node.ino).await.is_err());
    assert_eq!(
        fs.read(node.ino, 0, 8).await.unwrap().as_slice(),
        b"55XY\0\0\0\0"
    );
    store.fail_put.store(false, Ordering::Relaxed);
    fs.fsync(node.ino).await.unwrap();
    fs.rename(1, "package.rpm", 1, "package.txt").await.unwrap();
    fs.drain_disk_cache(DEADLINE).await.unwrap();
    assert_eq!(
        fs.read(node.ino, 0, 8).await.unwrap().as_slice(),
        b"55XY\0\0\0\0"
    );
    fs.unlink(1, "package.txt", false).await.unwrap();
    let replacement = fs
        .create(1, "package.rpm", libc::S_IFREG | 0o644, None, 0)
        .await
        .unwrap();
    fs.write(replacement.ino, 0, b"replacement").await.unwrap();
    fs.fsync(replacement.ino).await.unwrap();
    assert_ne!(replacement.ino, node.ino);
    assert_eq!(
        fs.read(replacement.ino, 0, 64).await.unwrap().as_slice(),
        b"replacement"
    );
    fs.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancelled_source_leader_releases_fill_and_old_generation_is_not_demoted() {
    let dir = tempfile::tempdir().unwrap();
    let mount = tempfile::tempdir().unwrap();
    let store = scoped_store();
    store.seed("package.rpm", &vec![0x35; READ_BLOCK_BYTES]);
    let fs = attach(configured(dir.path(), mount.path()), store.clone()).await;
    let node = fs.lookup(1, "package.rpm").await.unwrap();
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *store.read_gate.lock().unwrap() = Some(gate.clone());
    let task = tokio::spawn({
        let fs = fs.clone();
        let ino = node.ino;
        async move { fs.read(ino, 0, 128).await }
    });
    tokio::time::timeout(DEADLINE, store.read_started.notified())
        .await
        .unwrap();
    task.abort();
    assert!(matches!(task.await, Err(error) if error.is_cancelled()));
    *store.read_gate.lock().unwrap() = None;
    let task = tokio::spawn({
        let fs = fs.clone();
        let ino = node.ino;
        *store.read_gate.lock().unwrap() = Some(gate.clone());
        async move { fs.read(ino, 0, 128).await }
    });
    tokio::time::timeout(DEADLINE, store.read_started.notified())
        .await
        .unwrap();
    fs.write(node.ino, 0, b"new!").await.unwrap();
    *store.read_gate.lock().unwrap() = None;
    gate.add_permits(1);
    assert_eq!(task.await.unwrap().unwrap().as_slice(), &[0x35; 128]);
    assert_eq!(
        fs.effective_budgets().data_used_bytes,
        fs.disk_cache_funding().unwrap().required_bytes + 4,
        "after the reply drops only disk funding and the four dirty bytes may remain, not an old-generation L1 backing"
    );
    fs.drain_disk_cache(DEADLINE).await.unwrap();
    assert_eq!(
        fs.disk_cache_stats().unwrap().index_entries,
        0,
        "an old-generation fill must never enter L1 or L2"
    );
    assert_eq!(fs.read(node.ino, 0, 4).await.unwrap().as_slice(), b"new!");
    fs.fsync(node.ino).await.unwrap();
    fs.shutdown().await.unwrap();
}

#[tokio::test]
async fn opaque_backend_bypasses_without_preallocation_and_shutdown_closes_after_flush_error() {
    let dir = tempfile::tempdir().unwrap();
    let mount = tempfile::tempdir().unwrap();
    let store = MemoryStore::new();
    let fs = attach(configured(dir.path(), mount.path()), store).await;
    assert_eq!(
        fs.disk_cache_state(),
        Some(geesefs_rs::disk_cache::State::Bypass)
    );
    assert!(fs.disk_cache_stats().is_none());
    assert_eq!(fs.effective_budgets().disk_cache_funding_bytes, 0);
    assert!(!dir.path().join("payload-v1.cache").exists());
    fs.shutdown().await.unwrap();

    let dir = tempfile::tempdir().unwrap();
    let store = scoped_store();
    store.seed("package.rpm", b"old");
    let fs = attach(configured(dir.path(), mount.path()), store.clone()).await;
    let node = fs.lookup(1, "package.rpm").await.unwrap();
    fs.write(node.ino, 0, b"new").await.unwrap();
    store.fail_put.store(true, Ordering::Relaxed);
    assert!(fs.shutdown().await.is_err());
    assert_eq!(
        fs.disk_cache_state(),
        Some(geesefs_rs::disk_cache::State::Closed)
    );
    let _lock = PreparedDiskCache::prepare(dir.path(), 16 * MIB as u64).unwrap();
}

#[tokio::test]
async fn bursty_persistent_population_and_excluded_reads_preserve_fsync_progress() {
    for saturated in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mount = tempfile::tempdir().unwrap();
        let store = scoped_store();
        let key = if saturated {
            "package.rpm"
        } else {
            "package.txt"
        };
        store.seed(key, &vec![0x53; 24 * MIB]);
        store.seed("writer.txt", b"old");
        let mut config = configured(dir.path(), mount.path());
        if saturated {
            config.entry_limit = 4;
            config.read_ahead = 4096;
            config.read_ahead_small = 4096;
            config.read_ahead_large = 4096;
            config.read_ahead_parallel = 4096;
        } else {
            config.entry_limit = 1024;
        }
        let fs = attach(config, store.clone()).await;
        let node = fs.lookup(1, key).await.unwrap();
        let writer = fs.lookup(1, "writer.txt").await.unwrap();
        let pending_flush = if saturated {
            // Population must progress while a real gated upload owns its lane.
            fs.write(writer.ino, 0, b"committed").await.unwrap();
            let (entered, release) = store.pause_put();
            let task = tokio::spawn({
                let fs = fs.clone();
                let ino = writer.ino;
                async move { fs.fsync(ino).await }
            });
            tokio::time::timeout(DEADLINE, entered)
                .await
                .unwrap()
                .unwrap();
            Some((task, release))
        } else {
            None
        };
        let step = if saturated { 4 * MIB } else { READ_BLOCK_BYTES };
        let end = if saturated { 24 * MIB } else { 16 * MIB };
        for offset in (0..end).step_by(step) {
            let block = tokio::time::timeout(DEADLINE, fs.read(node.ino, offset as u64, 128))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(block.as_slice(), &[0x53; 128]);
        }
        if saturated {
            // The healthy writer can drain bounded admission between victims.
            // A burst is not proof of a stalled engine and must not require
            // rejecting otherwise reusable records to let fsync progress.
            let stats = fs.disk_cache_stats().unwrap();
            assert!(
                stats.admitted_demotions > 0,
                "the selected burst must actually exercise optional spill"
            );
            assert_eq!(
                stats.spill_drops[geesefs_rs::disk_cache::SpillDrop::Jobs as usize],
                0,
                "required reads allow bounded spill admission instead of synchronous job rejection"
            );
        } else {
            assert_eq!(fs.effective_budgets().flush_used_bytes, 0);
            assert_eq!(
                fs.disk_cache_stats().unwrap().index_entries,
                0,
                "excluded reads must not populate the persistent tier"
            );
        }
        if let Some((task, release)) = pending_flush {
            release.send(()).unwrap();
            tokio::time::timeout(DEADLINE, task)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
        } else {
            fs.write(writer.ino, 0, b"committed").await.unwrap();
            tokio::time::timeout(DEADLINE, fs.fsync(writer.ino))
                .await
                .unwrap()
                .unwrap();
        }
        assert_eq!(
            store.objects.lock().unwrap()["writer.txt"].bytes,
            b"committed"
        );
        assert_eq!(fs.effective_budgets().flush_used_bytes, 0);
        let budgets = fs.effective_budgets();
        assert_eq!(budgets.payload_limit_bytes, 32 * MIB);
        assert_eq!(budgets.ingress_bytes, 2 * MIB);
        assert_eq!(budgets.flush_bytes, 10 * MIB);
        assert_eq!(budgets.data_shared_bytes, 20 * MIB);
        assert!(budgets.disk_cache_funding_bytes > 0);
        assert_eq!(
            budgets.payload_limit_bytes,
            budgets.data_shared_bytes + budgets.flush_bytes + budgets.ingress_bytes
        );
        fs.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn mixed_disk_coverage_gets_only_the_missing_range_and_persists_the_validated_gap() {
    let dir = tempfile::tempdir().unwrap();
    let mount = tempfile::tempdir().unwrap();
    let store = scoped_store();
    let bytes: Vec<_> = [0x17, 0x62, 0xa4]
        .into_iter()
        .flat_map(|byte| vec![byte; READ_BLOCK_BYTES])
        .collect();
    store.seed("package.rpm", &bytes);
    let mut config = configured(dir.path(), mount.path());
    config.read_ahead = 0;
    config.read_ahead_small = 0;
    config.read_ahead_large = 0;
    let fs = attach(config.clone(), store.clone()).await;
    let node = fs.lookup(1, "package.rpm").await.unwrap();
    for offset in [0, 2 * READ_BLOCK_BYTES] {
        fs.read(node.ino, offset as u64, READ_BLOCK_BYTES)
            .await
            .unwrap();
    }
    fs.drain_disk_cache(DEADLINE).await.unwrap();
    fs.shutdown().await.unwrap();
    drop(fs);
    config.read_ahead = 384;
    config.read_ahead_small = 384;
    config.read_ahead_large = 384;
    config.read_ahead_parallel = 384;
    let fs = attach(config, store.clone()).await;
    let node = fs.lookup(1, "package.rpm").await.unwrap();
    store.read_ranges.lock().unwrap().clear();
    assert_eq!(
        fs.read(node.ino, 0, bytes.len()).await.unwrap().as_slice(),
        bytes
    );
    assert_eq!(
        *store.read_ranges.lock().unwrap(),
        [(READ_BLOCK_BYTES as u64, READ_BLOCK_BYTES)]
    );
    // Source-fill population persists the gap without retaining another RAM copy.
    fs.drain_disk_cache(DEADLINE).await.unwrap();
    fs.shutdown().await.unwrap();
    drop(fs);
    let fs = attach(configured(dir.path(), mount.path()), store.clone()).await;
    let node = fs.lookup(1, "package.rpm").await.unwrap();
    let gets = store.read_calls.load(Ordering::Relaxed);
    fs.read(node.ino, 0, READ_BLOCK_BYTES).await.unwrap();
    let hits = fs.disk_cache_stats().unwrap().hits;
    assert!(
        fs.read_reply_data(node.ino, 0, READ_BLOCK_BYTES, |result| {
            assert_eq!(result.unwrap(), vec![0x17; READ_BLOCK_BYTES]);
        })
        .await
        .is_none()
    );
    assert!(
        fs.disk_cache_stats().unwrap().hits > hits,
        "an independent read must consult L2"
    );
    let hits = fs.disk_cache_stats().unwrap().hits;
    assert_eq!(
        fs.read(node.ino, READ_BLOCK_BYTES as u64, 13)
            .await
            .unwrap()
            .as_slice(),
        &[0x62; 13]
    );
    assert_eq!(
        fs.disk_cache_stats().unwrap().hits,
        hits + 1,
        "independent reads must load their own validated L2 backing"
    );
    assert_eq!(store.read_calls.load(Ordering::Relaxed), gets);
    fs.shutdown().await.unwrap();
}

#[tokio::test]
async fn pending_rename_bypasses_l2_until_retry_publishes_the_new_name() {
    let dir = tempfile::tempdir().unwrap();
    let mount = tempfile::tempdir().unwrap();
    let store = scoped_store();
    store.seed("package.rpm", &vec![0x45; READ_BLOCK_BYTES]);
    let fs = attach(configured(dir.path(), mount.path()), store.clone()).await;
    let node = fs.lookup(1, "package.rpm").await.unwrap();
    fs.read(node.ino, 0, READ_BLOCK_BYTES).await.unwrap();
    fs.drain_disk_cache(DEADLINE).await.unwrap();
    let (entered, release) = store.pause_copy();
    let task = tokio::spawn({
        let fs = fs.clone();
        async move { fs.rename(1, "package.rpm", 1, "moved.deb").await }
    });
    tokio::time::timeout(DEADLINE, entered)
        .await
        .unwrap()
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    drop(release);
    let hits = fs.disk_cache_stats().unwrap().hits;
    let gets = store.read_calls.load(Ordering::Relaxed);
    assert_eq!(
        fs.read(node.ino, 0, 17).await.unwrap().as_slice(),
        &[0x45; 17]
    );
    assert_eq!(store.read_calls.load(Ordering::Relaxed), gets + 1);
    assert_eq!(fs.disk_cache_stats().unwrap().hits, hits);
    fs.rename(1, "package.rpm", 1, "moved.deb").await.unwrap();
    assert_eq!(fs.lookup(1, "moved.deb").await.unwrap().ino, node.ino);
    assert_eq!(
        fs.read(node.ino, 0, 17).await.unwrap().as_slice(),
        &[0x45; 17]
    );
    fs.shutdown().await.unwrap();
}

#[tokio::test]
async fn enabled_same_range_coalescing_independent_progress_and_deadline_release() {
    use std::{future::Future, task::Poll};
    let dir = tempfile::tempdir().unwrap();
    let mount = tempfile::tempdir().unwrap();
    let store = scoped_store();
    store.seed("package.rpm", &vec![0x27; 3 * READ_BLOCK_BYTES]);
    let fs = attach(configured(dir.path(), mount.path()), store.clone()).await;
    let node = fs.lookup(1, "package.rpm").await.unwrap();
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    store
        .read_range_gates
        .lock()
        .unwrap()
        .insert((0, READ_BLOCK_BYTES), gate.clone());
    let leader = tokio::spawn({
        let fs = fs.clone();
        let ino = node.ino;
        async move { fs.read(ino, 0, 128).await }
    });
    tokio::time::timeout(DEADLINE, store.read_started.notified())
        .await
        .unwrap();
    let mut follower = Box::pin(fs.read(node.ino, 1, 127));
    std::future::poll_fn(|cx| {
        assert!(follower.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    assert_eq!(
        store.read_calls.load(Ordering::Relaxed),
        1,
        "same-range follower must not issue another GET"
    );
    let block = tokio::time::timeout(DEADLINE, fs.read(node.ino, READ_BLOCK_BYTES as u64, 128))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(block.as_slice(), &[0x27; 128]);
    drop(block);
    drop(follower);
    gate.add_permits(1);
    assert_eq!(leader.await.unwrap().unwrap().as_slice(), &[0x27; 128]);
    // A request deadline drops its fill owner; a later request must proceed.
    store.read_range_gates.lock().unwrap().insert(
        (2 * READ_BLOCK_BYTES as u64, READ_BLOCK_BYTES),
        Arc::new(tokio::sync::Semaphore::new(0)),
    );
    assert!(
        tokio::time::timeout(
            Duration::from_millis(30),
            fs.read(node.ino, 2 * READ_BLOCK_BYTES as u64, 128)
        )
        .await
        .is_err()
    );
    store.read_range_gates.lock().unwrap().clear();
    assert_eq!(
        tokio::time::timeout(
            DEADLINE,
            fs.read(node.ino, 2 * READ_BLOCK_BYTES as u64, 128)
        )
        .await
        .unwrap()
        .unwrap()
        .as_slice(),
        &[0x27; 128]
    );
    fs.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancelled_shutdown_still_closes_optional_cache_while_dirty_data_remains_retryable() {
    let dir = tempfile::tempdir().unwrap();
    let mount = tempfile::tempdir().unwrap();
    let store = scoped_store();
    store.seed("writer.txt", b"old");
    let fs = attach(configured(dir.path(), mount.path()), store.clone()).await;
    let node = fs.lookup(1, "writer.txt").await.unwrap();
    fs.write(node.ino, 0, b"committed").await.unwrap();
    let (entered, release) = store.pause_put();
    let shutdown = tokio::spawn({
        let fs = fs.clone();
        async move { fs.shutdown().await }
    });
    tokio::time::timeout(DEADLINE, entered)
        .await
        .unwrap()
        .unwrap();
    shutdown.abort();
    assert!(shutdown.await.unwrap_err().is_cancelled());
    drop(release);
    tokio::time::timeout(DEADLINE, async {
        while fs.disk_cache_state() != Some(geesefs_rs::disk_cache::State::Closed) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("cache teardown must not depend on a cancelled writer flush");
    let owner = PreparedDiskCache::prepare(dir.path(), 16 * MIB as u64).unwrap();
    drop(owner);
    fs.shutdown().await.unwrap();
    assert_eq!(
        store.objects.lock().unwrap()["writer.txt"].bytes,
        b"committed"
    );
}
