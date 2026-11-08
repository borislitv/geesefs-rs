#![forbid(unsafe_code)]
use std::{ffi::OsString, time::Duration};
use tokio::process::Command;

use clap::Parser;
use geesefs_rs::config::{Config, ListType, normalize_fstab};

async fn cli(arguments: &[&str]) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_geesefs-rs"));
    command.env_clear().kill_on_drop(true).args(arguments);
    // Keep production credentials absent while allowing instrumented child
    // executions to contribute their actual entrypoint coverage.
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", profile);
    }
    tokio::time::timeout(Duration::from_secs(5), command.output())
        .await
        .expect("CLI must exit within five seconds")
        .unwrap()
}

#[test]
fn reference_cli_defaults_and_prefix() {
    let c = Config::try_parse_from(["geesefs-rs", "bucket:packages", "/tmp/mount"]).unwrap();
    c.validate().unwrap();
    assert_eq!(c.endpoint, "https://storage.yandexcloud.net");
    assert_eq!(c.effective_region(), "us-east-1");
    assert_eq!((c.dir_mode, c.file_mode), (0o755, 0o644));
    assert_eq!((c.memory_limit, c.entry_limit), (1000, 100000));
    assert_eq!(c.metrics_listen, None);
    assert_eq!(
        c.diagnostic_log_level,
        geesefs_rs::config::DiagnosticLogLevel::Warn
    );
    assert_eq!(c.symlink_attr, "--symlink-target");
    assert_eq!(c.bucket_and_prefix(), ("bucket", "packages"));
    assert_eq!(c.effective_list_type(), ListType::Extended);
    assert_eq!(c.sdk_max_retries, 2, "three total attempts by default");
}

#[test]
fn accepts_an_explicit_metrics_listener_without_contacting_backend() {
    let config = Config::try_parse_from([
        "geesefs-rs",
        "--metrics-listen",
        "127.0.0.1:9469",
        "bucket",
        "/tmp/mount",
    ])
    .expect("an explicit metrics listener must be accepted");
    config.validate().unwrap();
    assert_eq!(config.metrics_listen.unwrap().to_string(), "127.0.0.1:9469");
}

#[test]
fn diagnostic_log_levels_are_explicit_and_bounded() {
    for level in ["off", "error", "warn", "info", "debug"] {
        Config::try_parse_from([
            "geesefs-rs",
            "--diagnostic-log-level",
            level,
            "bucket",
            "/tmp/mount",
        ])
        .unwrap_or_else(|error| panic!("valid log level {level}: {error}"));
    }
    assert!(
        Config::try_parse_from([
            "geesefs-rs",
            "--diagnostic-log-level",
            "request-path",
            "bucket",
            "/tmp/mount",
        ])
        .is_err()
    );
}

#[test]
fn metrics_listener_requires_a_numeric_socket_address() {
    for address in ["localhost:9469", "127.0.0.1", "127.0.0.1:65536"] {
        assert!(
            Config::try_parse_from([
                "geesefs-rs",
                "--metrics-listen",
                address,
                "bucket",
                "/tmp/mount",
            ])
            .is_err(),
            "{address}"
        );
    }
    let config = Config::try_parse_from([
        "geesefs-rs",
        "--metrics-listen",
        "[::1]:9469",
        "bucket",
        "/tmp/mount",
    ])
    .unwrap();
    assert!(config.metrics_listen.unwrap().is_ipv6());
}

#[test]
fn exact_fstab_shape_normalizes_reference_flags() {
    let args = [
        "geesefs-rs",
        "bucket",
        "/tmp/mount",
        "-o",
        "ro,--memory-limit=64,--f,nodev",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    let normalized = normalize_fstab(args);
    let c = Config::try_parse_from(normalized).unwrap();
    assert_eq!(c.memory_limit, 64);
    assert!(c.foreground);
    assert_eq!(c.mount_options, ["ro", "nodev"]);
    c.validate().unwrap();
}

#[test]
fn allow_other_survives_reference_fstab_normalization() {
    let arguments = [
        "geesefs-rs",
        "bucket",
        "/tmp/mount",
        "-o",
        "ro,allow_other,--f,--memory-limit=64",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    let config = Config::try_parse_from(normalize_fstab(arguments)).unwrap();
    config.validate().unwrap();
    assert!(config.foreground);
    assert_eq!(config.memory_limit, 64);
    assert_eq!(config.mount_options, ["ro", "allow_other"]);
}

#[tokio::test]
async fn rejects_unsupported_options_in_actual_binary() {
    for flag in [
        "--drop-patch-conflict-typo",
        "--signature-v2",
        "--ignore-fsync",
        "--cluster",
        "--typo",
    ] {
        let output = cli(&[flag, "bucket", "/tmp/mount"]).await;
        assert_eq!(output.status.code(), Some(2), "flag {flag}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("unexpected argument"));
    }
}

#[test]
fn validates_boundary_values_without_contacting_backend() {
    for args in [
        vec!["geesefs-rs", "--memory-limit=0", "bucket", "/tmp/mount"],
        vec![
            "geesefs-rs",
            "--endpoint=ftp://invalid",
            "bucket",
            "/tmp/mount",
        ],
        vec!["geesefs-rs", "--entry-limit=0", "bucket", "/tmp/mount"],
        vec!["geesefs-rs", "--max-flushers=0", "bucket", "/tmp/mount"],
        vec!["geesefs-rs", "--max-flushers=4097", "bucket", "/tmp/mount"],
        vec!["geesefs-rs", "bucket:../escape", "/tmp/mount"],
        vec!["geesefs-rs", "-o", "allow_others", "bucket", "/tmp/mount"],
    ] {
        assert!(Config::try_parse_from(args).unwrap().validate().is_err());
    }
    assert!(
        Config::try_parse_from(["geesefs-rs", "--memory-limit=-1", "bucket", "/tmp/mount"])
            .is_err()
    );
    assert!(
        Config::try_parse_from(["geesefs-rs", "--list-type=3", "bucket", "/tmp/mount"]).is_err()
    );
}

#[test]
fn entry_limit_must_fit_owned_handle_admission() {
    let maximum = tokio::sync::Semaphore::MAX_PERMITS as u64;
    for (limit, valid) in [(maximum, true), (maximum + 1, false), (u64::MAX, false)] {
        let config = Config::try_parse_from([
            "geesefs-rs",
            &format!("--entry-limit={limit}"),
            "bucket",
            "/tmp/mount",
        ])
        .unwrap();
        assert_eq!(config.validate().is_ok(), valid, "entry limit {limit}");
    }
}

#[test]
fn permission_integer_bases_and_non_yandex_list_default() {
    let c = Config::try_parse_from([
        "geesefs-rs",
        "--dir-mode=493",
        "--file-mode=0x1a4",
        "--endpoint=http://127.0.0.1",
        "bucket",
        "/tmp/mount",
    ])
    .unwrap();
    assert_eq!((c.dir_mode, c.file_mode), (0o755, 0o644));
    assert_eq!(c.effective_list_type(), ListType::V1);
}

#[tokio::test]
async fn missing_foreground_mode_is_an_explicit_failure() {
    let output = cli(&["bucket", "/tmp/mount"]).await;
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("specify -f"));
}

#[tokio::test]
async fn startup_rejects_invalid_configuration_before_backend_or_mount_activity() {
    let directory = tempfile::tempdir().unwrap();
    let mount = directory.path().to_str().unwrap();
    for (flag, message) in [
        ("--memory-limit=0".to_owned(), "invalid memory-limit"),
        (
            format!(
                "--entry-limit={}",
                tokio::sync::Semaphore::MAX_PERMITS as u64 + 1
            ),
            "entry-limit must be",
        ),
        (
            "--request-timeout=0s".to_owned(),
            "request-timeout and shutdown-timeout",
        ),
        (
            "--read-only-direct-io".to_owned(),
            "read-only-direct-io requires -o ro",
        ),
    ] {
        let output = cli(&[
            "-f",
            "--no-detect",
            "--endpoint=http://127.0.0.1:1",
            &flag,
            "bucket",
            mount,
        ])
        .await;
        assert_eq!(output.status.code(), Some(1), "{flag}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(message),
            "{flag}: {:?}",
            output.stderr
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
    let missing = directory.path().join("absent");
    let output = cli(&[
        "-f",
        "--no-detect",
        "--endpoint=http://127.0.0.1:1",
        "bucket",
        missing.to_str().unwrap(),
    ])
    .await;
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("mountpoint must be an existing directory")
    );
    assert!(!missing.exists());
}
#[test]
fn backend_flags_have_reference_defaults_and_reject_invalid_combinations() {
    use std::time::Duration;
    let c = Config::try_parse_from(["geesefs-rs", "-f", "bucket", "/tmp/mount"]).unwrap();
    assert_eq!(c.http_timeout, Duration::from_secs(30));
    assert_eq!(c.multipart_age, Duration::from_secs(48 * 3600));
    assert_eq!(c.multipart_copy_threshold, 128);
    assert_eq!(c.sdk_max_retries, 2);
    assert_eq!(c.storage_class, "STANDARD");
    assert_eq!(c.iam_header, "X-YaCloud-SubjectToken");
    for flags in [
        vec!["--region="],
        vec!["--storage-class="],
        vec!["--profile="],
        vec!["--iam", "--signature-version=4"],
        vec!["--iam-header=Host"],
        vec!["--http-timeout=0s"],
        vec!["--http-timeout=25h"],
        vec!["--multipart-copy-threshold=0"],
        vec!["--sdk-max-retries=101"],
        vec!["--iam-url=http://127.0.0.1"],
        vec![
            "--sse",
            "--sse-c=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        ],
        vec![
            "--sse-kms=key",
            "--sse-c=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        ],
        vec!["--acl=invalid\nheader"],
    ] {
        let mut args = vec!["geesefs-rs", "-f"];
        args.extend(flags);
        args.extend(["bucket", "/tmp/mount"]);
        assert!(Config::try_parse_from(args).unwrap().validate().is_err());
    }
    for flag in [
        "--sse-c=not-base64",
        "--sse-c=YWJj",
        "--http-timeout=forever",
        "--iam-flavor=unknown",
    ] {
        assert!(
            Config::try_parse_from(["geesefs-rs", "-f", flag, "bucket", "/tmp/mount"]).is_err()
        );
    }
    let c = Config::try_parse_from([
        "geesefs-rs",
        "-f",
        "--http-timeout=1m30s",
        "--sse-kms=",
        "bucket",
        "/tmp/mount",
    ])
    .unwrap();
    c.validate().unwrap();
    assert_eq!(c.http_timeout, Duration::from_secs(90));
    assert_eq!(c.sse_kms, Some(String::new()));
}

#[test]
fn writer_and_core_options_are_parsed_and_reject_dependent_flags_without_patch() {
    let defaults = Config::try_parse_from(["geesefs-rs", "-f", "bucket", "/tmp/mount"]).unwrap();
    assert!(
        !defaults.enable_patch
            && !defaults.drop_patch_conflicts
            && !defaults.prefer_patch_uploads
            && !defaults.use_enomem
            && !defaults.no_implicit_dir
            && !defaults.no_dir_object
    );
    assert_eq!(defaults.stat_cache_ttl, Duration::from_secs(60));
    let configured = Config::try_parse_from([
        "geesefs-rs",
        "-f",
        "--enable-patch",
        "--drop-patch-conflicts",
        "--prefer-patch-uploads",
        "--use-enomem",
        "--no-implicit-dir",
        "--no-dir-object",
        "--stat-cache-ttl=0s",
        "--memory-limit=32768",
        "bucket",
        "/tmp/mount",
    ])
    .unwrap();
    configured.validate().unwrap();
    assert!(
        configured.enable_patch
            && configured.drop_patch_conflicts
            && configured.prefer_patch_uploads
            && configured.use_enomem
            && configured.no_implicit_dir
            && configured.no_dir_object
    );
    assert_eq!(configured.stat_cache_ttl, Duration::ZERO);
    for flag in ["--drop-patch-conflicts", "--prefer-patch-uploads"] {
        assert!(
            Config::try_parse_from(["geesefs-rs", "-f", flag, "bucket", "/tmp/mount"])
                .unwrap()
                .validate()
                .is_err()
        );
    }
}
