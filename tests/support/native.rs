use geesefs_rs::{filesystem::Filesystem, fuse::Fuse};
use std::{path::Path, sync::Arc};

/// Mount only a disposable fixture, and join its actual session on success/panic.
pub fn mounted(fs: Arc<Filesystem>, check: impl FnOnce(&Path)) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let adapter = Fuse::new(fs, runtime.handle().clone());
    let notifier = adapter.notifier.clone();
    let shutdown_error = adapter.shutdown_error.clone();
    let mut options = fuser::Config::default();
    let threads = std::env::var("GEESEFS_TEST_FUSE_THREADS")
        .map(|value| value.parse::<usize>().expect("native fixture loop count"))
        .unwrap_or(1);
    options.n_threads = Some(threads);
    options.clone_fd = threads > 1;
    options
        .mount_options
        .push(fuser::MountOption::DefaultPermissions);
    let session = fuser::Session::new(adapter, directory.path(), &options)
        .expect("explicitly requested native FUSE mount must be available");
    *notifier.lock().unwrap() = Some(session.notifier());
    let session = session.spawn().unwrap();
    let checked =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| check(directory.path())));
    session
        .umount_and_join()
        .expect("owned FUSE session must stop cleanly");
    if let Err(panic) = checked {
        std::panic::resume_unwind(panic);
    }
    assert!(shutdown_error.lock().unwrap().is_none());
}
