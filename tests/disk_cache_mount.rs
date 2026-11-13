//! Native acceptance is opt-in and never skipped successfully after invocation.
//! Ordinary tests exercise fail-closed preflight/CLI without mounting anything.
use std::{path::PathBuf, time::Duration};

fn python() -> PathBuf {
    std::env::var_os("GEESEFS_TEST_PYTHON").map_or_else(
        || {
            let local = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".venv-test/bin/python");
            if local.is_file() {
                local
            } else {
                PathBuf::from("python3")
            }
        },
        PathBuf::from,
    )
}

#[tokio::test]
async fn preflight_reports_capability_gaps_without_claiming_native_or_runtime_acceptance() {
    let output = tempfile::tempdir().unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(30),
        tokio::process::Command::new(python())
            .kill_on_drop(true)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .args([
                "bench/disk_cache.py",
                "--preflight",
                "--deadline=20",
                "--output",
            ])
            .arg(output.path())
            .output(),
    )
    .await
    .expect("preflight deadline")
    .unwrap();
    let report: serde_json::Value = serde_json::from_slice(
        &std::fs::read(output.path().join("report.json"))
            .expect("preflight must preserve a report"),
    )
    .unwrap();
    assert_eq!(report["production_accepted"], false);
    assert_eq!(report["qualified"], false);
    assert_eq!(report["native_passed"], false);
    assert_eq!(report["runs"], serde_json::json!([]));
    assert_eq!(
        result.status.success(),
        report["errors"].as_array().unwrap().is_empty()
    );
    assert!(
        report["capabilities"]["tools"]["rsync"].is_string()
            || report["capabilities"]["tools"]["rsync"].is_null()
    );
}

#[tokio::test]
async fn invalid_metadata_limit_fails_before_native_mount_or_credential_resolution() {
    let output = tempfile::tempdir().unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::process::Command::new(env!("CARGO_BIN_EXE_geesefs-rs"))
            .kill_on_drop(true)
            .env_clear()
            .args([
                "--metadata-limit=0",
                "--endpoint=http://127.0.0.1:9",
                "bucket",
            ])
            .arg(output.path())
            .output(),
    )
    .await
    .expect("invalid CLI deadline")
    .unwrap();
    assert!(!result.status.success());
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(
        error.contains("metadata-limit") && error.contains("positive"),
        "{error}"
    );
    assert!(
        !std::fs::read_dir(output.path())
            .unwrap()
            .any(|entry| entry.is_ok())
    );
}

#[tokio::test]
#[ignore = "requires frozen baseline/candidate receipts, native FUSE/nginx/rsync and delegated cgroup"]
async fn native_l2_package_nginx_rsync_pressure_restart_and_fsync() {
    let required = |name: &str| {
        std::env::var_os(name).unwrap_or_else(|| panic!("required native handoff: {name}"))
    };
    let result = tokio::time::timeout(
        Duration::from_secs(290),
        tokio::process::Command::new(python())
            .kill_on_drop(true)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .args([
                "bench/disk_cache.py",
                "--execute",
                "--native-only",
                "--deadline=180",
            ])
            .arg("--baseline")
            .arg(required("GEESEFS_DISK_CACHE_BASELINE"))
            .arg("--candidate")
            .arg(required("GEESEFS_DISK_CACHE_CANDIDATE"))
            .arg("--manifest")
            .arg(required("GEESEFS_DISK_CACHE_MANIFEST"))
            .arg("--cgroup-root")
            .arg(required("GEESEFS_BENCH_CGROUP_ROOT"))
            .arg("--output")
            .arg(required("GEESEFS_DISK_CACHE_OUTPUT"))
            .output(),
    )
    .await
    .expect("native matrix deadline")
    .unwrap();
    assert!(
        result.status.success(),
        "native verification failed, not skipped: {}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
