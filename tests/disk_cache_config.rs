#![forbid(unsafe_code)]

use clap::Parser;
use geesefs_rs::cache_policy::CachePolicy;
use geesefs_rs::config::Config;
use std::os::unix::fs::{PermissionsExt, symlink};

struct Fixture {
    root: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("cache")).unwrap();
        std::fs::create_dir(root.path().join("mount")).unwrap();
        Self { root }
    }

    fn config(&self) -> Config {
        Config::try_parse_from(self.args()).unwrap()
    }

    fn args(&self) -> Vec<String> {
        vec![
            "geesefs-rs".into(),
            "bucket:packages/nested".into(),
            self.root.path().join("mount").to_str().unwrap().into(),
            "--disk-cache-dir".into(),
            self.root.path().join("cache").to_str().unwrap().into(),
            "--disk-cache-max-size".into(),
            "1 TiB".into(),
            "--disk-cache-regex".into(),
            r"\.(deb|rpm)$".into(),
        ]
    }
}

#[test]
fn accepts_documented_flags_without_touching_cache_contents() {
    let root = tempfile::tempdir().unwrap();
    let cache = root.path().join("cache");
    let mount = root.path().join("mount");
    std::fs::create_dir(&cache).unwrap();
    std::fs::create_dir(&mount).unwrap();
    let config = Config::try_parse_from([
        "geesefs-rs",
        "bucket:packages",
        mount.to_str().unwrap(),
        "--disk-cache-dir",
        cache.to_str().unwrap(),
        "--disk-cache-max-size",
        "1 TiB",
        "--disk-cache-regex",
        r"\.(deb|rpm)$",
    ])
    .expect("the documented three flags must parse");
    config.validate().unwrap();
    let disk = config.disk_cache_config().unwrap().unwrap();
    assert_eq!(disk.max_bytes, 1_099_511_627_776);
    assert_eq!(disk.directory, std::fs::canonicalize(&cache).unwrap());
    assert!(disk.policy.matches("/pool/package.deb"));
    assert_eq!(std::fs::read_dir(cache).unwrap().count(), 0);
}

#[test]
fn disabled_configuration_needs_no_local_paths_and_retains_defaults() {
    let config = Config::try_parse_from(["geesefs-rs", "bucket", "/does/not/exist"]).unwrap();
    config.validate().unwrap();
    assert!(config.disk_cache_config().unwrap().is_none());
    assert_eq!(config.memory_limit, 1000);
    assert_eq!(config.read_ahead, 5120);
}

#[test]
fn every_partial_option_set_is_rejected() {
    let fixture = Fixture::new();
    let args = fixture.args();
    for mask in 1..7 {
        let mut partial = args[..3].to_vec();
        for bit in 0..3 {
            if mask & (1 << bit) != 0 {
                partial.extend_from_slice(&args[3 + bit * 2..5 + bit * 2]);
            }
        }
        assert!(Config::try_parse_from(partial).is_err(), "mask={mask}");
    }
    // Programmatic callers must not bypass the complete-set invariant either.
    let mut config = fixture.config();
    config.disk_cache_regex = None;
    assert!(config.validate().is_err());
}

#[test]
fn rejects_malformed_zero_and_saturated_sizes() {
    let fixture = Fixture::new();
    for size in [
        "",
        "potato",
        "-1",
        "0",
        "0 GiB",
        "0.1 B",
        "1 XB",
        "18446744073709551615",
        "18446744073709551616 B",
        "18446744073709551614 TiB",
        "999999999999999999999999999999999999999 TiB",
        "1e6",
        "NaN",
        "inf",
    ] {
        let mut args = fixture.args();
        args[6] = size.into();
        assert!(Config::try_parse_from(args).is_err(), "size={size:?}");
    }
    for (size, expected) in [
        ("1 B", 1),
        ("1.5 KiB", 1536),
        ("1 GB", 1_000_000_000),
        ("18446744073709551614 B", u64::MAX - 1),
    ] {
        let mut args = fixture.args();
        args[6] = size.into();
        assert_eq!(
            Config::try_parse_from(args)
                .unwrap()
                .disk_cache_config()
                .unwrap()
                .unwrap()
                .max_bytes,
            expected
        );
    }
}

#[test]
fn regex_policy_uses_literal_case_sensitive_mount_relative_paths() {
    let policy: CachePolicy = r"\.(deb|rpm)$".parse().unwrap();
    for path in ["/a.deb", "/pool/a.rpm", "/é/包.deb"] {
        assert!(policy.matches(path), "{path}");
    }
    for path in [
        "a.deb",
        "/a.debx",
        "/a.DEB",
        "/a.Rpm",
        "/aXdeb",
        "/Release",
        "/Packages.gz",
        "/a.deb/metadata",
        "/a.deb/",
        "",
    ] {
        assert!(!policy.matches(path), "{path}");
    }
    let insensitive: CachePolicy = r"(?i)\.(deb|rpm)$".parse().unwrap();
    assert!(insensitive.matches("/a.DEB"));
    assert!(insensitive.matches("/a.RpM"));
    let literal: CachePolicy = r"^/pool/a\+b\[1\]\.deb$".parse().unwrap();
    assert!(literal.matches("/pool/a+b[1].deb"));
    assert!(!literal.matches("/pool/ab1.deb"));
    let prefix: CachePolicy = r"^/pool/.*\.deb$".parse().unwrap();
    let fixture = Fixture::new();
    let mut config = fixture.config();
    config.disk_cache_regex = Some(prefix);
    assert_eq!(config.bucket_and_prefix(), ("bucket", "packages/nested"));
    let policy = config.disk_cache_config().unwrap().unwrap().policy;
    assert!(policy.matches("/pool/a.deb"));
    assert!(!policy.matches("/packages/nested/pool/a.deb"));
}

#[test]
fn rejects_invalid_pcre_and_bounded_regex_compilation() {
    let fixture = Fixture::new();
    for pattern in ["[", r"(deb)\1", r"(?=deb)", r"(?<=/)deb", r"\p{L}{10000}"] {
        let mut args = fixture.args();
        args[8] = pattern.into();
        assert!(Config::try_parse_from(args).is_err(), "pattern={pattern}");
    }
    assert!(
        "a".repeat(CachePolicy::MAX_PATTERN_BYTES + 1)
            .parse::<CachePolicy>()
            .is_err()
    );
}

#[test]
fn local_directory_must_exist_be_private_and_not_overlap_mount() {
    let fixture = Fixture::new();
    let mut config = fixture.config();
    let root = fixture.root.path();
    for path in [root.join("absent"), root.join("mount"), root.to_owned()] {
        config.disk_cache_dir = Some(path);
        assert!(config.validate().is_err());
    }
    std::fs::create_dir(root.join("mount/inside")).unwrap();
    config.disk_cache_dir = Some(root.join("mount/inside"));
    assert!(config.validate().is_err());
    std::fs::write(root.join("file"), b"not a directory").unwrap();
    config.disk_cache_dir = Some(root.join("file"));
    assert!(config.validate().is_err());
    config.disk_cache_dir = Some(root.join("cache"));
    for mode in [0o777, 0o770, 0o500, 0o600] {
        std::fs::set_permissions(root.join("cache"), std::fs::Permissions::from_mode(mode))
            .unwrap();
        assert!(config.validate().is_err(), "mode={mode:o}");
    }
    std::fs::set_permissions(root.join("cache"), std::fs::Permissions::from_mode(0o700)).unwrap();
    config.validate().unwrap();
}

#[test]
fn canonical_paths_handle_symlinks_and_component_boundaries_consistently() {
    let fixture = Fixture::new();
    let root = fixture.root.path();
    let mut config = fixture.config();
    symlink(root.join("cache"), root.join("cache-alias")).unwrap();
    config.disk_cache_dir = Some(root.join("cache-alias"));
    assert_eq!(
        config.disk_cache_config().unwrap().unwrap().directory,
        root.join("cache")
    );
    symlink(root.join("mount"), root.join("mount-alias")).unwrap();
    config.disk_cache_dir = Some(root.join("mount-alias"));
    assert!(config.validate().is_err());
    config.disk_cache_dir = Some(root.join("cache"));
    config.mountpoint = root.join("cache-alias");
    assert!(config.validate().is_err());
    std::fs::create_dir(root.join("cache/sibling")).unwrap();
    config.mountpoint = root.join("cache-alias/sibling");
    assert!(config.validate().is_err());
    std::fs::create_dir(root.join("cache-sibling")).unwrap();
    config.mountpoint = root.join("cache-sibling");
    config.validate().unwrap();
    config.disk_cache_dir = Some(root.join("cache/../cache"));
    config.validate().unwrap();
}

#[test]
fn fstab_normalization_preserves_disk_flags_and_existing_mount_options() {
    use geesefs_rs::config::normalize_fstab;
    let fixture = Fixture::new();
    let root = fixture.root.path();
    let options = format!(
        "ro,--f,--disk-cache-dir={},--disk-cache-max-size=1 TiB,--disk-cache-regex=\\.(deb|rpm)$",
        root.join("cache").display()
    );
    let args = [
        "geesefs-rs",
        "bucket:prefix",
        root.join("mount").to_str().unwrap(),
        "-o",
        &options,
    ]
    .into_iter()
    .map(std::ffi::OsString::from)
    .collect();
    let config = Config::try_parse_from(normalize_fstab(args)).unwrap();
    config.validate().unwrap();
    assert_eq!(config.mount_options, ["ro"]);
    assert!(config.foreground);
    assert!(
        config
            .disk_cache_config()
            .unwrap()
            .unwrap()
            .policy
            .matches("/a.deb")
    );
}
