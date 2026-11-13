//! Public-interface representation controls; not mounted/S3 performance acceptance.
mod support;

use geesefs_rs::{config::ListType, filesystem::Filesystem, read};
use std::{sync::atomic::Ordering, time::Duration};
use support::MemoryStore;

#[global_allocator]
#[cfg(not(feature = "system-allocator"))]
static ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[global_allocator]
#[cfg(feature = "system-allocator")]
static ALLOCATOR: &stats_alloc::StatsAlloc<std::alloc::System> = &stats_alloc::INSTRUMENTED_SYSTEM;

#[tokio::test]
async fn refreshed_clean_inode_can_become_a_retryable_sparse_writer() {
    tokio::time::timeout(Duration::from_secs(20), async {
        for limit in [None, Some(256 * 1024)] {
            let store = MemoryStore::new();
            store.seed("file", b"abcdef");
            store.seed("untouched", b"keeper");
            let mut config = MemoryStore::config(16);
            config.metadata_limit = limit;
            config.list_type = Some(ListType::V2);
            config.stat_cache_ttl = Duration::ZERO;
            let fs = Filesystem::new(config.clone(), store.clone()).unwrap();
            let node = fs.lookup(1, "file").await.unwrap();
            let untouched = fs.lookup(1, "untouched").await.unwrap();
            fs.reference(&node);
            fs.open(node.ino, false).unwrap();
            let old = read::snapshot(&node).unwrap();

            store.seed("file", b"UVWXYZ");
            let refreshed = fs.refresh_inode_if_expired(node.ino).await.unwrap();
            assert_eq!(refreshed.ino, node.ino);
            assert_eq!(
                fs.read(node.ino, 0, 20).await.unwrap().as_slice(),
                b"UVWXYZ"
            );
            assert_eq!(old.size(), 6);
            assert_ne!(
                old.remote().unwrap().etag,
                read::snapshot(&node).unwrap().remote().unwrap().etag
            );
            fs.fsync(node.ino).await.unwrap();
            assert_eq!(store.put_calls.load(Ordering::SeqCst), 0);

            fs.write(node.ino, 2, b"12").await.unwrap();
            fs.truncate(node.ino, 4).unwrap();
            fs.truncate(node.ino, 7).unwrap();
            let expected = b"UV12\0\0\0";
            assert_eq!(fs.read(node.ino, 0, 20).await.unwrap().as_slice(), expected);
            assert_eq!(
                fs.read(untouched.ino, 0, 20).await.unwrap().as_slice(),
                b"keeper"
            );
            fs.fsync(untouched.ino).await.unwrap();
            assert_eq!(store.put_calls.load(Ordering::SeqCst), 0);
            store.fail_put.store(true, Ordering::SeqCst);
            assert!(fs.fsync(node.ino).await.is_err());
            // Expiry must not replace the dirty generation after failed durability.
            fs.refresh_inode_if_expired(node.ino).await.unwrap();
            assert_eq!(fs.read(node.ino, 0, 20).await.unwrap().as_slice(), expected);
            assert_eq!(store.bytes("file"), b"UVWXYZ");
            store.fail_put.store(false, Ordering::SeqCst);
            fs.fsync(node.ino).await.unwrap();
            assert_eq!(store.bytes("file"), expected);
            fs.rename(1, "file", 1, "renamed").await.unwrap();
            assert_eq!(fs.lookup(1, "renamed").await.unwrap().ino, node.ino);
            assert!(!store.objects.lock().unwrap().contains_key("file"));
            let fresh = Filesystem::new(config, store.clone()).unwrap();
            let persisted = fresh.lookup(1, "renamed").await.unwrap();
            assert_eq!(
                fresh.read(persisted.ino, 0, 20).await.unwrap().as_slice(),
                expected
            );
            assert_eq!(old.key(), "file");
            fs.release(node.ino);
            fs.forget(node.ino, 1);
        }
    })
    .await
    .expect("representation regression deadline");
}

fn process_sample() -> serde_json::Value {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let kib = |name: &str| -> u64 {
        status
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap()
    };
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap();
    let fields: Vec<_> = stat[stat.rfind(')').unwrap() + 2..]
        .split_whitespace()
        .collect();
    let cgroup = [
        "memory.current",
        "memory.peak",
        "memory.stat",
        "memory.events",
        "cpu.stat",
    ]
    .into_iter()
    .map(|name| {
        (
            name.to_owned(),
            std::fs::read_to_string(format!("/sys/fs/cgroup/{name}")).ok(),
        )
    })
    .collect::<std::collections::BTreeMap<_, _>>();
    serde_json::json!({
        "rss_kib": kib("VmRSS:"), "peak_rss_kib": kib("VmHWM:"),
        "user_ticks": fields[11].parse::<u64>().unwrap(),
        "system_ticks": fields[12].parse::<u64>().unwrap(),
        "allocator": geesefs_rs::metrics::allocation_stats(),
        "cgroup": cgroup
    })
}

#[tokio::test]
#[ignore = "explicit isolated Linux diagnostic; run each mode in a fresh process"]
async fn clean_refresh_and_writer_residency_diagnostic() {
    let entries: usize = std::env::var("GEESEFS_REPRESENTATION_ENTRIES")
        .unwrap_or_else(|_| "100000".into())
        .parse()
        .unwrap();
    assert!((1_000..=500_000).contains(&entries));
    let enabled = match std::env::var("GEESEFS_REPRESENTATION_BUDGET").as_deref() {
        Ok("enabled") => true,
        Ok("disabled") | Err(_) => false,
        other => panic!("expected enabled or disabled, got {other:?}"),
    };
    tokio::time::timeout(Duration::from_secs(300), async {
        let store = MemoryStore::new();
        for index in 0..entries {
            store.seed(&format!("file-{index:08}"), b"");
        }
        let mut config = MemoryStore::config(16);
        config.entry_limit = (entries + 1) as u64;
        config.metadata_limit = enabled.then_some(2 * 1024 * 1024 * 1024);
        config.list_type = Some(ListType::V2);
        config.stat_cache_ttl = Duration::ZERO;
        let fs = Filesystem::new(config, store.clone()).unwrap();
        let baseline = process_sample();
        let started = std::time::Instant::now();
        for index in 0..entries {
            let node = fs.lookup(1, &format!("file-{index:08}")).await.unwrap();
            assert_eq!(node.attr().size, 0);
            // Do not retain a second namespace worth of fake-backend HEAD logs.
            store.head_keys.lock().unwrap().clear();
        }
        fs.flush_all().await.unwrap();
        let clean_ns = started.elapsed().as_nanos();
        let clean = process_sample();
        let clean_charge = fs.metadata_usage();
        assert_eq!(fs.namespace.lock().unwrap().nodes.len(), entries + 1);
        assert_eq!(fs.namespace.lock().unwrap().evictions, 0);

        let started = std::time::Instant::now();
        for index in 0..entries {
            let ino = fs.namespace.lock().unwrap().named(&format!("file-{index:08}")).unwrap().ino;
            assert_eq!(fs.refresh_inode_if_expired(ino).await.unwrap().attr().size, 0);
            store.head_keys.lock().unwrap().clear();
        }
        let refresh_ns = started.elapsed().as_nanos();
        let refreshed = process_sample();
        assert_eq!(fs.metadata_usage(), clean_charge, "equal remote refresh must not leak charges");

        // Exactly 10% become sparse writers, without any allocated payload bytes.
        for index in (0..entries).step_by(10) {
            let node = fs.namespace.lock().unwrap().named(&format!("file-{index:08}")).unwrap();
            fs.truncate(node.ino, 1).unwrap();
            assert_eq!(node.attr().size, 1);
        }
        let writers = process_sample();
        assert_eq!(store.put_calls.load(Ordering::SeqCst), 0);
        assert_eq!(store.delete_calls.load(Ordering::SeqCst), 0);
        assert_eq!(store.copy_calls.load(Ordering::SeqCst), 0);
        assert_eq!(store.read_calls.load(Ordering::SeqCst), 0);
        println!("namespace_representation_json={}", serde_json::json!({
            "entries": entries, "metadata_enabled": enabled,
            "allocator": if cfg!(feature = "system-allocator") { "system" } else { "jemalloc" },
            "baseline_with_seeded_backend": baseline, "clean": clean,
            "refreshed": refreshed, "ten_percent_sparse_writers": writers,
            "populate_wall_ns": clean_ns, "refresh_wall_ns": refresh_ns,
            "clean_admission": clean_charge, "writer_admission": fs.metadata_usage(),
            "layout_bytes": {
                "inode": std::mem::size_of::<geesefs_rs::inode::Inode>(),
                "state": std::mem::size_of::<geesefs_rs::inode::State>(),
                "metadata_budget": std::mem::size_of::<geesefs_rs::metadata_budget::MetadataBudget>(),
                "metadata_charge": std::mem::size_of::<geesefs_rs::metadata_budget::MetadataCharge>()
            },
            "scope": "isolated synthetic flat namespace; allocator/process/container scopes reported separately; not mounted performance acceptance"
        }));
    }).await.expect("representation diagnostic deadline");
}
