//! Concurrent parent listing must not undo a move's admitted preparation.
mod support;

use geesefs_rs::{config::ListType, error::Error, filesystem::Filesystem};
use std::{sync::atomic::Ordering, time::Duration};
use support::MemoryStore;

#[tokio::test]
async fn rejected_move_before_copy_does_not_strand_child_durability() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let mut rejected_before_copy = 0;
        // Exercise byte-pressure boundaries without deriving expected costs from
        // WriterState, namespace marker layouts or private admission helpers.
        for spare in [0, 192, 224, 256, 288, 320, 512] {
            let store = MemoryStore::new();
            store.seed("old/", b"");
            store.seed("old/child", b"base");
            store.seed("pressure", b"");
            let mut config = MemoryStore::config(16);
            config.metadata_limit = Some(256 * 1024);
            config.list_type = Some(ListType::V2);
            config.stat_cache_ttl = Duration::ZERO;
            // A missing destination must not consume the LIST barrier during
            // implicit-directory discovery, before rename's own preflight.
            config.no_implicit_dir = true;
            let fs = Filesystem::new(config, store.clone()).unwrap();
            let source = fs.lookup(1, "old").await.unwrap();
            let child = fs.lookup(source.ino, "child").await.unwrap();
            let pressure_node = fs.lookup(1, "pressure").await.unwrap();
            let (entered, release) = store.pause_list();
            let moving = tokio::spawn({
                let fs = fs.clone();
                async move { fs.rename(1, "old", 1, "new").await }
            });
            entered.await.unwrap();
            assert_eq!(
                store.list_requests.lock().unwrap().last().cloned(),
                Some(("old/".into(), None)),
                "barrier must follow rename preflight, at subtree discovery"
            );
            assert_eq!(store.copy_calls.load(Ordering::SeqCst), 0);
            assert_eq!(store.delete_calls.load(Ordering::SeqCst), 0);

            // S3 permits an object and a same-named prefix simultaneously. An
            // external creator publishes the colliding object during the move.
            // MemoryStore correctly returns that object as authoritative Contents
            // alongside the prefix; a bare prefix alone has unknown metadata and
            // would not exercise listed_node's clean-state replacement branch.
            store.seed("old", b"");
            let listing = fs.directory_remainder(1, "", "", 16).await.unwrap();

            // Hold an independently owned canonical metadata version against the
            // filesystem's real admission budget. Calibrate its fixed cost from
            // an observed successful public operation, not production layouts.
            let template = pressure_node.state.lock().unwrap().metadata.clone();
            let before_probe = fs.metadata_usage().charged;
            let mut probe = template.clone();
            probe.try_insert("hold", b"").unwrap();
            let fixed = fs.metadata_usage().charged - before_probe;
            drop(probe);
            let usage = fs.metadata_usage();
            let available = usage.limit.unwrap() - usage.charged;
            assert!(available > fixed + spare);
            let mut pressure = template.clone();
            pressure.try_insert("hold", &vec![b'x'; available - fixed - spare]).unwrap();
            assert_eq!(
                fs.metadata_usage().limit.unwrap() - fs.metadata_usage().charged,
                spare,
                "public pressure owner must leave the requested byte margin"
            );

            release.send(()).unwrap();
            let result = moving.await.unwrap();
            let copies = store.copy_calls.load(Ordering::SeqCst);
            let deletes = store.delete_calls.load(Ordering::SeqCst);
            drop(pressure);
            drop(template);
            drop(listing);
            if result.is_err() && copies == 0 && deletes == 0 {
                rejected_before_copy += 1;
                assert!(matches!(result, Err(Error::Fs(libc::ENOMEM))), "spare={spare}: {result:?}");
                assert_eq!(store.bytes("old/child"), b"base");
                assert!(!store.objects.lock().unwrap().contains_key("new/child"));
                fs.write(child.ino, 0, b"NEXT").await.unwrap();
                let durable = fs.fsync(child.ino).await;
                assert!(
                    durable.is_ok(),
                    "spare={spare}: rejected move sent no COPY/DELETE, but stranded the child: {durable:?}"
                );
                assert_eq!(store.bytes("old/child"), b"NEXT");
            }
        }
        assert!(rejected_before_copy > 0, "fixture must exercise admission rejection before backend mutation");
    }).await.expect("rename/listing/admission regression deadline");
}
