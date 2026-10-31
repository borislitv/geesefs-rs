#![forbid(unsafe_code)]
mod support;

use geesefs_rs::filesystem::Filesystem;
use std::sync::{Arc, atomic::Ordering};
use std::time::Duration;
use support::MemoryStore;

const MIB: usize = 1024 * 1024;
const WINDOW: usize = 128 * 1024;
const READ: usize = 4096;

fn fixture() -> (Arc<MemoryStore>, Arc<Filesystem>, Vec<u8>) {
    let store = MemoryStore::new();
    let bytes: Vec<u8> = (0..24 * MIB)
        .map(|at| (at ^ (at >> 8) ^ (at >> 16)) as u8)
        .collect();
    store.seed("file", &bytes);
    let mut config = MemoryStore::config(16);
    config.read_ahead = WINDOW / 1024;
    config.read_ahead_small = WINDOW / 1024;
    config.read_ahead_large = WINDOW / 1024;
    config.read_ahead_parallel = WINDOW / 1024;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    (store, fs, bytes)
}

async fn revisit(fs: &Filesystem, ino: u64, bytes: &[u8]) {
    fs.open(ino, false).unwrap();
    for number in 0..96 {
        let offset = number * 2 * WINDOW;
        let mut replied = false;
        let error = fs
            .read_reply_data(ino, offset as u64, READ, |result| {
                assert_eq!(result.unwrap(), &bytes[offset..offset + READ]);
                replied = true;
            })
            .await;
        assert!(error.is_none());
        assert!(replied);
        let budgets = fs.effective_budgets();
        assert_eq!(budgets.payload_limit_bytes, 16 * MIB);
        assert!(
            budgets.data_used_bytes + budgets.ingress_used_bytes + budgets.flush_used_bytes
                <= 16 * MIB
        );
    }
    fs.release(ino);
}

#[tokio::test]
async fn idle_flush_capacity_retains_clean_data_without_increasing_the_payload_limit() {
    let (store, fs, bytes) = fixture();
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    revisit(&fs, ino, &bytes).await;
    assert_eq!(store.read_calls.load(Ordering::Relaxed), 96);
    assert_eq!(
        store
            .read_ranges
            .lock()
            .unwrap()
            .iter()
            .map(|(_, size)| size)
            .sum::<usize>(),
        12 * MIB
    );
    revisit(&fs, ino, &bytes).await;
    assert_eq!(
        store.read_calls.load(Ordering::Relaxed),
        96,
        "a 12 MiB clean working set must reuse idle flush capacity within the unchanged 16 MiB payload limit"
    );
    assert_eq!(store.put_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn an_owned_cached_reply_cannot_pin_flush_capacity_or_change_after_fsync() {
    for callback in [false, true] {
        let (store, fs, bytes) = fixture();
        let ino = fs.lookup(1, "file").await.unwrap().ino;
        revisit(&fs, ino, &bytes).await;
        assert!(fs.effective_budgets().flush_used_bytes > 0);
        let pinned = if callback {
            let mut reply = None;
            let error = fs
                .read_reply(ino, 0, READ, |result| reply = Some(result.unwrap()))
                .await;
            assert!(error.is_none());
            reply.unwrap()
        } else {
            fs.read(ino, 0, READ).await.unwrap()
        };
        assert_eq!(pinned.as_slice(), &bytes[..READ]);
        let written = fs
            .create(1, "written", libc::S_IFREG | 0o644, None, 0)
            .await
            .unwrap();
        for number in 0..4 {
            fs.write(written.ino, (number * MIB) as u64, &vec![b'w'; MIB])
                .await
                .unwrap();
        }
        fs.truncate(written.ino, (6 * MIB) as u64).unwrap();
        tokio::time::timeout(Duration::from_secs(5), fs.fsync(written.ino))
            .await
            .expect("an owned cache reply must not pin the upload reserve")
            .unwrap();
        let committed = store.bytes("written");
        assert_eq!(committed.len(), 6 * MIB);
        assert!(committed[..4 * MIB].iter().all(|&byte| byte == b'w'));
        assert!(committed[4 * MIB..].iter().all(|&byte| byte == 0));
        assert_eq!(
            fs.effective_budgets().flush_used_bytes,
            0,
            "an owned reply must not retain a revocable flush allocation"
        );
        assert_eq!(pinned.as_slice(), &bytes[..READ]);
        fs.write(ino, 0, b"new generation").await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), fs.fsync(ino))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(pinned.as_slice(), &bytes[..READ]);
        assert_eq!(&store.bytes("file")[..14], b"new generation");
    }
}

#[tokio::test]
async fn concurrent_flushes_exclude_new_loans_until_the_last_upload_finishes() {
    let (store, fs, bytes) = fixture();
    let ino = fs.lookup(1, "file").await.unwrap().ino;
    revisit(&fs, ino, &bytes).await;
    assert!(fs.effective_budgets().flush_used_bytes > 0);
    let first = fs
        .create(1, "first", libc::S_IFREG | 0o644, None, 0)
        .await
        .unwrap()
        .ino;
    fs.write(first, 0, b"first").await.unwrap();
    let (entered, release) = store.pause_put();
    let flushing = fs.clone();
    let task = tokio::spawn(async move { flushing.fsync(first).await });
    tokio::time::timeout(Duration::from_secs(5), entered)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        fs.effective_budgets().flush_used_bytes,
        0,
        "upload admission must reclaim all clean cache loans"
    );
    let second = fs
        .create(1, "second", libc::S_IFREG | 0o644, None, 0)
        .await
        .unwrap()
        .ino;
    fs.write(second, 0, b"second").await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), fs.fsync(second))
        .await
        .expect("independent flushes must remain concurrent")
        .unwrap();
    assert_eq!(store.bytes("second"), b"second");
    assert!(!task.is_finished());
    revisit(&fs, ino, &bytes).await;
    assert_eq!(
        fs.effective_budgets().flush_used_bytes,
        0,
        "ending one upload must not lend the reserve while another upload is active"
    );
    release.send(()).unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(store.bytes("first"), b"first");
    revisit(&fs, ino, &bytes).await;
    assert!(
        fs.effective_budgets().flush_used_bytes > 0,
        "completed uploads must make unused reserve available again"
    );
}

#[tokio::test]
async fn cancelled_and_failed_uploads_release_activity_without_losing_dirty_data() {
    for cancel in [false, true] {
        let (store, fs, bytes) = fixture();
        let ino = fs.lookup(1, "file").await.unwrap().ino;
        revisit(&fs, ino, &bytes).await;
        assert!(fs.effective_budgets().flush_used_bytes > 0);
        let written = fs
            .create(1, "written", libc::S_IFREG | 0o644, None, 0)
            .await
            .unwrap()
            .ino;
        fs.write(written, 0, b"retained").await.unwrap();
        if cancel {
            let (entered, release) = store.pause_put();
            let flushing = fs.clone();
            let task = tokio::spawn(async move { flushing.fsync(written).await });
            tokio::time::timeout(Duration::from_secs(5), entered)
                .await
                .unwrap()
                .unwrap();
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
            drop(release);
        } else {
            store.fail_put.store(true, Ordering::Relaxed);
            assert!(fs.fsync(written).await.is_err());
            store.fail_put.store(false, Ordering::Relaxed);
        }
        assert!(!store.objects.lock().unwrap().contains_key("written"));
        assert_eq!(
            fs.read(written, 0, 8).await.unwrap().as_slice(),
            b"retained"
        );
        revisit(&fs, ino, &bytes).await;
        assert!(
            fs.effective_budgets().flush_used_bytes > 0,
            "a failed or cancelled upload must not leave cache lending disabled"
        );
        tokio::time::timeout(Duration::from_secs(5), fs.fsync(written))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(store.bytes("written"), b"retained");
        assert_eq!(fs.effective_budgets().flush_used_bytes, 0);
    }
}
