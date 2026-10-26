mod support;

#[test]
fn synchronous_fuser_destroy_flushes_without_an_entered_runtime() {
    let store = support::MemoryStore::new();
    let fs = store.filesystem(32);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let node = fs
            .create(1, "pending", libc::S_IFREG | 0o644, None, 0)
            .await
            .unwrap();
        fs.write(node.ino, 0, b"durable on unmount").await.unwrap();
    });
    let mut adapter = geesefs_rs::fuse::Fuse::new(fs, runtime.handle().clone());
    fuser::Filesystem::destroy(&mut adapter);
    assert!(adapter.shutdown_error.lock().unwrap().is_none());
    assert_eq!(store.bytes("pending"), b"durable on unmount");
}
