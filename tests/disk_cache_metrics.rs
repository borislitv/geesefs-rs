#![forbid(unsafe_code)]
mod support;

use geesefs_rs::{
    config::Config, disk_cache::PreparedDiskCache, filesystem::Filesystem, metrics::render,
    read::READ_BLOCK_BYTES,
};
use std::{
    os::unix::fs::PermissionsExt,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use support::MemoryStore;

const MIB: usize = 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(10);

fn sample(text: &str, name: &str) -> f64 {
    text.lines()
        .find_map(|line| {
            let (key, value) = line.split_once(' ')?;
            (key == name).then(|| value.parse().unwrap())
        })
        .unwrap_or_else(|| panic!("missing {name} in {text}"))
}

fn configured(dir: &std::path::Path, mount: &std::path::Path) -> Config {
    let mut config = MemoryStore::config(32);
    config.mountpoint = mount.into();
    config.disk_cache_dir = Some(dir.into());
    config.disk_cache_max_size = Some(16 * MIB as u64);
    config.disk_cache_regex = Some(r"\.rpm$".parse().unwrap());
    config.metadata_limit = Some(2 * MIB);
    config.entry_limit = 32;
    config.read_ahead = 128;
    config.read_ahead_small = 128;
    config.read_ahead_large = 128;
    config
}

async fn attach(config: Config, store: Arc<MemoryStore>) -> Arc<Filesystem> {
    let disk = config.disk_cache_config().unwrap().unwrap();
    std::fs::set_permissions(&disk.directory, std::fs::Permissions::from_mode(0o700)).unwrap();
    let prepared = PreparedDiskCache::prepare(&disk.directory, disk.max_bytes).unwrap();
    let fs = Filesystem::new(config, store).unwrap();
    fs.attach_disk_cache(prepared).await.unwrap();
    fs
}

#[tokio::test]
async fn disabled_prepared_and_bypass_are_not_fake_ready_or_measured_allocations() {
    let store = MemoryStore::new();
    let fs = Filesystem::new(MemoryStore::config(32), store.clone()).unwrap();
    let text = render(&fs).unwrap();
    assert_eq!(
        sample(&text, "geesefs_disk_cache_state{state=\"disabled\"}"),
        1.
    );
    assert_eq!(
        sample(&text, "geesefs_disk_cache_state{state=\"ready\"}"),
        0.
    );
    assert_eq!(sample(&text, "geesefs_disk_cache_stats_available"), 0.);
    assert_eq!(sample(&text, "geesefs_disk_cache_hits_total"), 0.);
    assert_eq!(sample(&text, "geesefs_disk_cache_hit_bytes_total"), 0.);
    assert_eq!(sample(&text, "geesefs_disk_cache_funding_bytes"), 0.);
    assert_eq!(sample(&text, "geesefs_metadata_limit_enabled"), 0.);
    assert_eq!(sample(&text, "geesefs_metadata_charged_bytes"), 0.);
    assert!(!text.contains("geesefs_disk_cache_allocated_bytes "));
    assert!(!text.contains("geesefs_disk_cache_index_heap_bytes "));
    assert_eq!(
        sample(
            &text,
            "geesefs_disk_cache_opened_file_observation_available"
        ),
        0.
    );
    assert!(
        !text
            .lines()
            .any(|line| line.starts_with("geesefs_disk_cache_opened_file_allocated_bytes "))
    );
    for missing in [
        "demotions",
        "capacity_evictions",
        "read_latency",
        "write_latency",
        "recovery_duration",
        "allocated_bytes",
        "index_heap_bytes",
    ] {
        assert_eq!(
            sample(
                &text,
                &format!(
                    "geesefs_disk_cache_instrumentation_available{{observation=\"{missing}\"}}"
                )
            ),
            0.
        );
    }
    fs.shutdown().await.unwrap();

    let dir = tempfile::tempdir().unwrap();
    let mount = tempfile::tempdir().unwrap();
    let config = configured(dir.path(), mount.path());
    *store.cache_namespace.lock().unwrap() = Some([7; 32]);
    let prepared_fs = Filesystem::new(config.clone(), store.clone()).unwrap();
    let text = render(&prepared_fs).unwrap();
    assert_eq!(
        sample(&text, "geesefs_disk_cache_state{state=\"prepared\"}"),
        1.
    );
    assert_eq!(sample(&text, "geesefs_disk_cache_stats_available"), 0.);
    assert!(
        !text
            .lines()
            .any(|line| line.starts_with("geesefs_disk_cache_hits_total "))
    );
    prepared_fs.shutdown().await.unwrap();
    *store.cache_namespace.lock().unwrap() = None;
    let bypass = attach(config, store).await;
    let text = render(&bypass).unwrap();
    assert_eq!(
        sample(&text, "geesefs_disk_cache_state{state=\"bypass\"}"),
        1.
    );
    assert_eq!(
        sample(
            &text,
            "geesefs_disk_cache_bypass_reason{reason=\"unsupported_backend\"}"
        ),
        1.
    );
    assert_eq!(sample(&text, "geesefs_disk_cache_stats_available"), 0.);
    assert_eq!(sample(&text, "geesefs_disk_cache_funding_bytes"), 0.);
    bypass.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancelled_fill_follower_and_scrapes_do_not_count_another_l2_operation() {
    let dir = tempfile::tempdir().unwrap();
    let mount = tempfile::tempdir().unwrap();
    let store = MemoryStore::new();
    *store.cache_namespace.lock().unwrap() = Some([7; 32]);
    store.seed("package.rpm", &[0x39; READ_BLOCK_BYTES]);
    let mut config = configured(dir.path(), mount.path());
    config.read_ahead = 0;
    config.read_ahead_small = 0;
    config.read_ahead_large = 0;
    let fs = attach(config, store.clone()).await;
    let ino = fs.lookup(1, "package.rpm").await.unwrap().ino;
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *store.read_gate.lock().unwrap() = Some(gate.clone());
    let leader = tokio::spawn({
        let fs = fs.clone();
        async move { fs.read(ino, 11, 37).await }
    });
    tokio::time::timeout(DEADLINE, store.read_started.notified())
        .await
        .unwrap();
    // The real L2 miss has completed; only the source GET is gated. A second
    // caller shares the fill, then cancels without creating another L2 load.
    let mut follower = Box::pin(fs.read(ino, 11, 37));
    assert!(
        std::future::poll_fn(|cx| std::task::Poll::Ready(follower.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    drop(follower);
    for _ in 0..2 {
        let text = render(&fs).unwrap();
        assert_eq!(
            sample(&text, "geesefs_disk_cache_read_operations_total"),
            1.
        );
        assert_eq!(
            sample(&text, "geesefs_disk_cache_read_operation_misses_total"),
            1.
        );
        assert_eq!(
            sample(&text, "geesefs_disk_cache_read_operation_hits_total"),
            0.
        );
        assert_eq!(sample(&text, "geesefs_disk_cache_hit_bytes_total"), 0.);
    }
    gate.add_permits(1);
    assert_eq!(
        tokio::time::timeout(DEADLINE, leader)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .as_slice(),
        &[0x39; 37]
    );
    assert_eq!(fs.read(ino, 11, 37).await.unwrap().as_slice(), &[0x39; 37]);
    let text = render(&fs).unwrap();
    assert_eq!(
        sample(&text, "geesefs_disk_cache_read_operations_total"),
        1.
    );
    assert_eq!(
        sample(&text, "geesefs_disk_cache_read_operation_misses_total"),
        1.
    );
    assert_eq!(sample(&text, "geesefs_disk_cache_hits_total"), 0.);
    assert_eq!(store.read_calls.load(Ordering::Relaxed), 1);
    fs.shutdown().await.unwrap();
}

#[tokio::test]
async fn real_eviction_recovery_and_aligned_l2_read_have_exact_count_and_byte_metrics() {
    let dir = tempfile::tempdir().unwrap();
    let mount = tempfile::tempdir().unwrap();
    let store = MemoryStore::new();
    *store.cache_namespace.lock().unwrap() = Some([7; 32]);
    store.seed("private-package.rpm", &vec![0x39; READ_BLOCK_BYTES]);
    let config = configured(dir.path(), mount.path());
    let fs = attach(config.clone(), store.clone()).await;
    let package = fs.lookup(1, "private-package.rpm").await.unwrap();
    assert_eq!(
        fs.read(package.ino, 0, READ_BLOCK_BYTES)
            .await
            .unwrap()
            .as_slice(),
        &[0x39; READ_BLOCK_BYTES]
    );
    let step = fs
        .effective_budgets()
        .requested_read_ahead_bytes
        .max(READ_BLOCK_BYTES);
    store.seed("pressure.txt", &vec![0x71; 40 * step]);
    let pressure = fs.lookup(1, "pressure.txt").await.unwrap();
    for offset in (0..40 * step).step_by(step) {
        assert_eq!(
            fs.read(pressure.ino, offset as u64, READ_BLOCK_BYTES)
                .await
                .unwrap()
                .as_slice(),
            &[0x71; READ_BLOCK_BYTES]
        );
        fs.drain_disk_cache(DEADLINE).await.unwrap();
    }
    let text = render(&fs).unwrap();
    assert_eq!(
        sample(&text, "geesefs_read_cache_capacity_evictions_total"),
        9.
    );
    assert_eq!(sample(&text, "geesefs_disk_cache_misses_total"), 1.);
    assert_eq!(sample(&text, "geesefs_disk_cache_hits_total"), 0.);
    assert_eq!(sample(&text, "geesefs_disk_cache_hit_bytes_total"), 0.);
    assert!(sample(&text, "geesefs_disk_cache_io_write_bytes_total") > READ_BLOCK_BYTES as f64);
    assert_eq!(sample(&text, "geesefs_disk_cache_index_entries"), 1.);
    assert_eq!(
        sample(&text, "geesefs_disk_cache_admitted_demotions_total"),
        1.
    );
    assert_eq!(
        sample(&text, "geesefs_disk_cache_engine_admitted_demotions_total"),
        1.
    );
    assert_eq!(
        sample(&text, "geesefs_disk_cache_spill_operations_total"),
        1.
    );
    assert_eq!(sample(&text, "geesefs_disk_cache_spill_no_space_total"), 0.);
    assert_eq!(sample(&text, "geesefs_disk_cache_spill_errors_total"), 0.);
    assert_eq!(
        sample(&text, "geesefs_disk_cache_read_operations_total"),
        1.
    );
    assert_eq!(
        sample(&text, "geesefs_disk_cache_read_operation_misses_total"),
        1.
    );
    assert_eq!(
        sample(&text, "geesefs_disk_cache_region_reclaims_started_total"),
        0.
    );
    let spill_total = sample(
        &text,
        "geesefs_disk_cache_spill_operation_duration_seconds_total",
    );
    assert!(spill_total > 0.);
    assert_eq!(
        sample(
            &text,
            "geesefs_disk_cache_spill_operation_duration_seconds_max"
        ),
        spill_total
    );
    fs.shutdown().await.unwrap();
    drop(fs);

    let mut config = config;
    config.read_ahead = 0;
    config.read_ahead_small = 0;
    config.read_ahead_large = 0;
    let fs = attach(config, store.clone()).await;
    let package = fs.lookup(1, "private-package.rpm").await.unwrap();
    let source_gets = store.read_calls.load(Ordering::Relaxed);
    assert_eq!(
        fs.read(package.ino, 11, 37).await.unwrap().as_slice(),
        &[0x39; 37]
    );
    assert_eq!(
        store.read_calls.load(Ordering::Relaxed),
        source_gets,
        "must be a real disk hit, not refetch"
    );
    fs.drain_disk_cache(DEADLINE).await.unwrap();
    let stats = fs.disk_cache_stats().unwrap();
    let budgets = fs.effective_budgets();
    let metadata = fs.metadata_usage();
    let source_metadata_calls = (
        store.head_starts.load(Ordering::Relaxed),
        store.list_calls.load(Ordering::Relaxed),
    );
    let text = render(&fs).unwrap();
    assert_eq!(
        sample(&text, "geesefs_disk_cache_state{state=\"ready\"}"),
        1.
    );
    assert_eq!(sample(&text, "geesefs_disk_cache_stats_available"), 1.);
    assert_eq!(sample(&text, "geesefs_disk_cache_hits_total"), 1.);
    assert_eq!(sample(&text, "geesefs_disk_cache_misses_total"), 0.);
    assert_eq!(
        sample(&text, "geesefs_disk_cache_read_operations_total"),
        1.
    );
    assert_eq!(
        sample(&text, "geesefs_disk_cache_read_operation_hits_total"),
        1.
    );
    assert_eq!(
        sample(&text, "geesefs_disk_cache_read_operation_misses_total"),
        0.
    );
    assert_eq!(
        sample(&text, "geesefs_disk_cache_read_operation_errors_total"),
        0.
    );
    assert_eq!(
        sample(
            &text,
            "geesefs_disk_cache_read_operation_duration_seconds_total"
        ),
        stats.read_operations.total_duration_ns as f64 / 1e9
    );
    assert_eq!(
        sample(
            &text,
            "geesefs_disk_cache_read_operation_duration_seconds_max"
        ),
        stats.read_operations.max_duration_ns as f64 / 1e9
    );
    assert!(
        sample(
            &text,
            "geesefs_disk_cache_read_operation_duration_seconds_total"
        ) > 0.
    );
    for phase in ["startup", "allocation", "recovery"] {
        assert_eq!(
            sample(
                &text,
                &format!("geesefs_disk_cache_open_phase_completions_total{{phase=\"{phase}\"}}")
            ),
            1.
        );
        let total = sample(
            &text,
            &format!("geesefs_disk_cache_open_phase_duration_seconds_total{{phase=\"{phase}\"}}"),
        );
        assert!(total > 0.);
        assert_eq!(
            sample(
                &text,
                &format!("geesefs_disk_cache_open_phase_duration_seconds_max{{phase=\"{phase}\"}}")
            ),
            total
        );
    }
    assert_eq!(
        sample(
            &text,
            "geesefs_disk_cache_opened_file_observation_available"
        ),
        1.
    );
    assert!(sample(&text, "geesefs_disk_cache_opened_file_allocated_bytes") >= (16 * MIB) as f64);
    for observation in [
        "demotions",
        "read_latency",
        "write_latency",
        "recovery_duration",
        "region_reclaims_started",
    ] {
        assert_eq!(
            sample(
                &text,
                &format!(
                    "geesefs_disk_cache_instrumentation_available{{observation=\"{observation}\"}}"
                )
            ),
            1.
        );
    }
    // Requested 11..48 is planned as 0..48: report validated pipeline bytes,
    // not a fabricated count of the 37 user-visible bytes.
    assert_eq!(sample(&text, "geesefs_disk_cache_hit_bytes_total"), 48.);
    // Cross-language contract: every concrete benchmark metric is exported by
    // the real renderer, and missing observations remain explicitly unavailable.
    let python = std::env::var_os("GEESEFS_TEST_PYTHON").unwrap_or_else(|| {
        let local = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".venv-test/bin/python");
        if local.is_file() {
            local.into_os_string()
        } else {
            "python3".into()
        }
    });
    let manifest = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::process::Command::new(python)
            .kill_on_drop(true)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .args(["bench/disk_cache.py", "--example-manifest"])
            .output(),
    )
    .await
    .expect("manifest command deadline")
    .unwrap();
    assert!(
        manifest.status.success(),
        "{}",
        String::from_utf8_lossy(&manifest.stderr)
    );
    let manifest: serde_json::Value = serde_json::from_slice(&manifest.stdout).unwrap();
    for metric in manifest["disk_metrics"].as_object().unwrap().values() {
        sample(&text, metric.as_str().unwrap());
    }
    for observation in manifest["unavailable_disk_observations"]
        .as_array()
        .unwrap()
    {
        assert_eq!(
            sample(
                &text,
                &format!(
                    "geesefs_disk_cache_instrumentation_available{{observation=\"{}\"}}",
                    observation.as_str().unwrap()
                )
            ),
            0.
        );
    }
    assert_eq!(
        sample(&text, "geesefs_disk_cache_managed_capacity_bytes"),
        (16 * MIB) as f64
    );
    assert_eq!(
        sample(&text, "geesefs_disk_cache_funding_bytes"),
        budgets.disk_cache_funding_bytes as f64
    );
    assert_eq!(
        sample(&text, "geesefs_effective_l1_policy_bytes"),
        budgets.read_cache_policy_bytes as f64
    );
    assert_eq!(sample(&text, "geesefs_metadata_limit_enabled"), 1.);
    assert_eq!(
        sample(&text, "geesefs_metadata_limit_bytes"),
        (2 * MIB) as f64
    );
    assert_eq!(
        sample(&text, "geesefs_metadata_charged_bytes"),
        metadata.charged as f64
    );
    assert!(metadata.charged > 0);
    assert_eq!(
        sample(
            &text,
            "geesefs_metadata_instrumentation_available{observation=\"external_overhead_bytes\"}"
        ),
        0.
    );
    let component_sum: f64 = [
        "fixed",
        "recovery",
        "metadata",
        "copy",
        "read_work",
        "submission",
    ]
    .iter()
    .map(|component| {
        sample(
            &text,
            &format!("geesefs_disk_cache_component_funding_bytes{{component=\"{component}\"}}"),
        )
    })
    .sum();
    assert_eq!(
        component_sum, budgets.disk_cache_funding_bytes as f64,
        "funding components must not double-count recovery"
    );
    for (metric, expected) in [
        ("geesefs_disk_cache_active_jobs", stats.active_jobs),
        (
            "geesefs_disk_cache_submission_bytes{scope=\"queued_and_active\"}",
            stats.submission_bytes,
        ),
        (
            "geesefs_disk_cache_submission_items{scope=\"queued_and_active\"}",
            stats.submission_items,
        ),
        ("geesefs_disk_cache_copy_owned_bytes", stats.copied_bytes),
        ("geesefs_disk_cache_index_entries", stats.index_entries),
        ("geesefs_disk_cache_recent_entries", stats.recent_entries),
    ] {
        assert_eq!(sample(&text, metric), expected as f64, "{metric}");
    }
    assert!(text.contains("queued and active"));
    assert!(text.contains("not measured heap"));
    assert!(text.contains("external overhead"));
    assert!(!text.contains("private-package"));
    assert!(!text.contains(&dir.path().to_string_lossy().to_string()));
    let reads = stats.io_read_bytes;
    let writes = stats.io_write_bytes;
    render(&fs).unwrap();
    let after = fs.disk_cache_stats().unwrap();
    assert_eq!(
        (after.io_read_bytes, after.io_write_bytes),
        (reads, writes),
        "scrapes must not read cache storage"
    );
    assert_eq!(fs.metadata_usage(), metadata);
    assert_eq!(
        after.read_operations, stats.read_operations,
        "scrapes cannot record another operation"
    );
    // An L1 hit is not a second completed L2 load or another L2 hit byte.
    assert_eq!(
        fs.read(package.ino, 11, 37).await.unwrap().as_slice(),
        &[0x39; 37]
    );
    let again = render(&fs).unwrap();
    assert_eq!(
        sample(&again, "geesefs_disk_cache_read_operations_total"),
        1.
    );
    assert_eq!(sample(&again, "geesefs_disk_cache_hit_bytes_total"), 48.);
    assert_eq!(
        fs.effective_budgets().data_used_bytes,
        budgets.data_used_bytes
    );
    assert_eq!(store.read_calls.load(Ordering::Relaxed), source_gets);
    assert_eq!(
        (
            store.head_starts.load(Ordering::Relaxed),
            store.list_calls.load(Ordering::Relaxed)
        ),
        source_metadata_calls
    );
    fs.shutdown().await.unwrap();
    let text = render(&fs).unwrap();
    assert_eq!(
        sample(&text, "geesefs_disk_cache_state{state=\"closed\"}"),
        1.
    );
    assert_eq!(sample(&text, "geesefs_disk_cache_hit_bytes_total"), 48.);
}
