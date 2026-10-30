#![forbid(unsafe_code)]
use std::{ffi::OsString, time::Duration};
use tokio::process::Command;

use clap::Parser;
use geesefs_rs::config::{Config, ListType, normalize_fstab};

#[test]
fn reference_cli_defaults_and_prefix() {
    let c = Config::try_parse_from(["geesefs-rs", "bucket:packages", "/tmp/mount"]).unwrap();
    c.validate().unwrap();
    assert_eq!(c.endpoint, "https://storage.yandexcloud.net");
    assert_eq!(c.effective_region(), "us-east-1");
    assert_eq!((c.dir_mode, c.file_mode), (0o755, 0o644));
    assert_eq!((c.memory_limit, c.entry_limit), (1000, 100000));
    assert_eq!(c.symlink_attr, "--symlink-target");
    assert_eq!(c.bucket_and_prefix(), ("bucket", "packages"));
    assert_eq!(c.effective_list_type(), ListType::Extended);
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

#[tokio::test]
async fn rejects_unsupported_options_in_actual_binary() {
    for flag in [
        "--drop-patch-conflict-typo",
        "--signature-v2",
        "--ignore-fsync",
        "--cluster",
        "--typo",
    ] {
        let output = tokio::time::timeout(
            Duration::from_secs(5),
            Command::new(env!("CARGO_BIN_EXE_geesefs-rs"))
                .env_clear()
                .kill_on_drop(true)
                .args([flag, "bucket", "/tmp/mount"])
                .output(),
        )
        .await
        .expect("CLI must exit within five seconds")
        .unwrap();
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
        vec!["geesefs-rs", "-o", "allow_other", "bucket", "/tmp/mount"],
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
    let output = tokio::time::timeout(
        Duration::from_secs(5),
        Command::new(env!("CARGO_BIN_EXE_geesefs-rs"))
            .env_clear()
            .kill_on_drop(true)
            .args(["bucket", "/tmp/mount"])
            .output(),
    )
    .await
    .expect("CLI must exit within five seconds")
    .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("specify -f"));
}
#[test]
fn backend_flags_have_reference_defaults_and_reject_invalid_combinations() {
    use std::time::Duration;
    let c = Config::try_parse_from(["geesefs-rs", "-f", "bucket", "/tmp/mount"]).unwrap();
    assert_eq!(c.http_timeout, Duration::from_secs(30));
    assert_eq!(c.multipart_age, Duration::from_secs(48 * 3600));
    assert_eq!(c.multipart_copy_threshold, 128);
    assert_eq!(c.sdk_max_retries, 3);
    assert_eq!(c.storage_class, "STANDARD");
    assert_eq!(c.iam_header, "X-YaCloud-SubjectToken");
    for flags in [
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
