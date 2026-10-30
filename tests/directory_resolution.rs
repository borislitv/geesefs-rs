#![forbid(unsafe_code)]
mod support;
use geesefs_rs::filesystem::Filesystem;
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use support::MemoryStore;

async fn pending<F: Future>(mut future: Pin<&mut F>) {
    std::future::poll_fn(|cx| {
        assert!(future.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
}
async fn started(store: &MemoryStore, count: usize) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while store.head_starts.load(Ordering::SeqCst) < count {
            store.head_started.notified().await;
        }
    })
    .await
    .expect("selected HEADs must run concurrently before either response is released");
}
async fn fixture() -> (
    Arc<MemoryStore>,
    Arc<Filesystem>,
    u64,
    u64,
    Arc<tokio::sync::Semaphore>,
) {
    let store = MemoryStore::new();
    store.standard_listing.store(true, Ordering::Relaxed);
    for prefix in ["a", "b"] {
        store.seed(&format!("{prefix}/"), b"");
        store.seed(&format!("{prefix}/plain"), b"regular");
        store.seed(&format!("{prefix}/link"), b"");
        store.seed(&format!("{prefix}/fifo"), b"");
        let mut objects = store.objects.lock().unwrap();
        objects
            .get_mut(&format!("{prefix}/link"))
            .unwrap()
            .metadata
            .insert("--symlink-target".into(), "target".into());
        objects
            .get_mut(&format!("{prefix}/fifo"))
            .unwrap()
            .metadata
            .insert("mode".into(), format!("{}", libc::S_IFIFO | 0o600));
    }
    let mut config = MemoryStore::config(32);
    config.max_requests = 2;
    config.enable_specials = true;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let a = fs.lookup(1, "a").await.unwrap().ino;
    let b = fs.lookup(1, "b").await.unwrap().ino;
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *store.head_gate.lock().unwrap() = Some(gate.clone());
    store.head_starts.store(0, Ordering::SeqCst);
    store.peak_heads.store(0, Ordering::SeqCst);
    (store, fs, a, b, gate)
}
fn assert_types(entries: &[geesefs_rs::filesystem::DirEntry]) {
    assert_eq!(
        entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.kind))
            .collect::<Vec<_>>(),
        [
            ("fifo", fuser::FileType::NamedPipe),
            ("link", fuser::FileType::Symlink),
            ("plain", fuser::FileType::RegularFile)
        ]
    );
}

#[tokio::test]
async fn directory_heads_overlap_but_share_one_global_bound_across_readdir_calls() {
    let (store, fs, a, b, gate) = fixture().await;
    let scan = tokio::spawn({
        let fs = fs.clone();
        async move { fs.directory_chunk(a, "", 8).await }
    });
    started(&store, 2).await;
    assert_eq!(fs.effective_budgets().max_metadata_resolution_jobs, 2);
    assert_eq!(fs.effective_budgets().metadata_resolution_jobs_in_use, 2);
    let mut other = Box::pin(fs.directory_chunk(b, "", 8));
    pending(other.as_mut()).await;
    assert_eq!(
        store.head_starts.load(Ordering::SeqCst),
        2,
        "second page must not create another page-sized group of jobs"
    );
    assert_eq!(store.active_heads.load(Ordering::SeqCst), 2);
    gate.add_permits(6);
    let (first, second) =
        tokio::time::timeout(Duration::from_secs(3), async { tokio::join!(scan, other) })
            .await
            .unwrap();
    assert_types(&first.unwrap().unwrap());
    assert_types(&second.unwrap());
    assert_eq!(store.head_starts.load(Ordering::SeqCst), 6);
    assert_eq!(store.peak_heads.load(Ordering::SeqCst), 2);
    assert_eq!(store.active_heads.load(Ordering::SeqCst), 0);
    assert_eq!(fs.effective_budgets().metadata_resolution_jobs_in_use, 0);
}

#[tokio::test]
async fn cancelled_or_failed_resolution_releases_all_global_jobs_and_retry_keeps_types() {
    for failure in [false, true] {
        let (store, fs, a, _b, gate) = fixture().await;
        let mut scan = Box::pin(fs.directory_chunk(a, "", 8));
        pending(scan.as_mut()).await;
        started(&store, 2).await;
        if failure {
            store.fail_head_once.store(true, Ordering::Relaxed);
            gate.add_permits(1);
            assert!(
                tokio::time::timeout(Duration::from_secs(3), scan)
                    .await
                    .unwrap()
                    .is_err()
            );
        } else {
            drop(scan);
        }
        assert_eq!(
            store.active_heads.load(Ordering::SeqCst),
            0,
            "no detached HEAD jobs may survive cancellation/error"
        );
        assert_eq!(fs.effective_budgets().metadata_resolution_jobs_in_use, 0);
        gate.add_permits(3);
        let entries = tokio::time::timeout(Duration::from_secs(3), fs.directory_chunk(a, "", 8))
            .await
            .unwrap()
            .unwrap();
        assert_types(&entries);
        assert_eq!(store.active_heads.load(Ordering::SeqCst), 0);
        assert!(store.peak_heads.load(Ordering::SeqCst) <= 2);
    }
}
