use std::{ffi::OsString, path::PathBuf, time::Duration};

use clap::{Parser, ValueEnum};

use crate::error::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum ListType {
    #[value(name = "1")]
    V1,
    #[value(name = "2")]
    V2,
    #[value(name = "ext-v1")]
    Extended,
}

/// Only options with implemented configuration semantics are parsed.
#[derive(Clone, Debug, Parser)]
#[command(version, about = "GeeseFS Rust rewrite (NOT accepted for production)")]
pub struct Config {
    pub bucket_prefix: String,
    pub mountpoint: PathBuf,
    #[arg(long, default_value = "https://storage.yandexcloud.net")]
    pub endpoint: String,
    #[arg(long, help = "S3 region; defaults to us-east-1 before autodetection")]
    pub region: Option<String>,
    #[arg(long)]
    pub no_detect: bool,
    #[arg(long, value_enum, default_value = "auto")]
    pub signature_version: SignatureVersion,
    #[arg(long)]
    pub profile: Option<String>,
    #[arg(long, value_delimiter = ',')]
    pub shared_config: Vec<PathBuf>,
    #[arg(long)]
    pub iam: bool,
    #[arg(long, default_value = "gcp", value_enum)]
    pub iam_flavor: IamFlavor,
    #[arg(long)]
    pub iam_url: Option<String>,
    #[arg(long, default_value = "X-YaCloud-SubjectToken")]
    pub iam_header: String,
    #[arg(long)]
    pub requester_pays: bool,
    #[arg(long)]
    pub sse: bool,
    #[arg(long)]
    pub sse_kms: Option<String>,
    #[arg(long)]
    pub sse_c: Option<CustomerKey>,
    #[arg(long, default_value = "STANDARD")]
    pub storage_class: String,
    #[arg(long, default_value_t = 0)]
    pub cold_min_size: u64,
    #[arg(long)]
    pub use_content_type: bool,
    #[arg(long)]
    pub acl: Option<String>,
    #[arg(long)]
    pub no_checksum: bool,
    #[arg(
        long,
        help = "Disable S3 TLS certificate verification (insecure; explicit opt-in)"
    )]
    pub no_verify_ssl: bool,
    #[arg(long, default_value = "30s", value_parser = parse_duration)]
    pub http_timeout: Duration,
    #[arg(long, default_value = "5m", value_parser = parse_duration)]
    pub request_timeout: Duration,
    #[arg(long, default_value = "5m", value_parser = parse_duration)]
    pub shutdown_timeout: Duration,
    #[arg(long, default_value_t = 3)]
    pub sdk_max_retries: u32,
    #[arg(long)]
    pub no_expire_multipart: bool,
    #[arg(long, default_value = "48h", value_parser = parse_duration)]
    pub multipart_age: Duration,
    #[arg(long, default_value_t = 128)]
    pub multipart_copy_threshold: u64,
    #[arg(long)]
    pub subdomain: bool,
    #[arg(long, value_enum)]
    pub list_type: Option<ListType>,
    #[arg(long, default_value_t = 1000)]
    pub memory_limit: usize,
    #[arg(long)]
    pub use_enomem: bool,
    #[arg(long)]
    pub no_implicit_dir: bool,
    #[arg(long)]
    pub no_dir_object: bool,
    #[arg(long,default_value="1m",value_parser=parse_duration)]
    pub stat_cache_ttl: Duration,
    #[arg(long, default_value_t = 0)]
    pub negative_lookup_cache_size: usize,
    #[arg(long, default_value_t = 100000)]
    pub entry_limit: u64,
    #[arg(long, default_value = "0755", value_parser = parse_mode)]
    pub dir_mode: u16,
    #[arg(long, default_value = "0644", value_parser = parse_mode)]
    pub file_mode: u16,
    #[arg(long, default_value = "uid")]
    pub uid_attr: String,
    #[arg(long, default_value = "gid")]
    pub gid_attr: String,
    #[arg(long, default_value = "mode")]
    pub mode_attr: String,
    #[arg(long, default_value = "rdev")]
    pub rdev_attr: String,
    #[arg(long, default_value = "mtime")]
    pub mtime_attr: String,
    #[arg(long, default_value = "--symlink-target", allow_hyphen_values = true)]
    pub symlink_attr: String,
    #[arg(short = 'o', value_delimiter = ',')]
    pub mount_options: Vec<String>,
    #[arg(short = 'f', long = "f")]
    pub foreground: bool,
    #[arg(long)]
    pub enable_perms: bool,
    #[arg(long)]
    pub enable_mtime: bool,
    #[arg(long)]
    pub enable_specials: bool,
    #[arg(long)]
    pub no_specials: bool,
    #[arg(long)]
    pub disable_xattr: bool,
    #[arg(long)]
    pub emulate_hardlinks_as_symlinks: bool,
    #[arg(long)]
    pub fsync_on_close: bool,
    #[arg(long)]
    pub enable_patch: bool,
    #[arg(long)]
    pub drop_patch_conflicts: bool,
    #[arg(long)]
    pub prefer_patch_uploads: bool,
    #[arg(long = "single-part", default_value_t = 5)]
    pub single_part: u64,
    #[arg(long = "part-sizes", default_value = "5:1000,25:1000,125")]
    pub part_sizes: crate::parts::PartSchedule,
    #[arg(long)]
    pub read_only_direct_io: bool,
    #[arg(long, default_value_t = 16)]
    pub max_flushers: usize,
    #[arg(long, default_value_t = 8)]
    pub max_parallel_parts: usize,
    #[arg(long, default_value_t = 16)]
    pub max_parallel_copy: usize,
    #[arg(long, default_value_t = 64)]
    pub max_requests: usize,
    #[arg(long)]
    pub uid: Option<u32>,
    #[arg(long)]
    pub gid: Option<u32>,
    #[arg(long, default_value_t = 5120)]
    pub read_ahead: usize,
    #[arg(long, default_value_t = 128)]
    pub read_ahead_small: usize,
    #[arg(long, default_value_t = 102400)]
    pub read_ahead_large: usize,
    #[arg(long, default_value_t = 20480)]
    pub read_ahead_parallel: usize,
    #[arg(long, default_value_t = 20480)]
    pub large_read_cutoff: u64,
    #[arg(long, default_value_t = 128)]
    pub small_read_cutoff: usize,
    #[arg(long, default_value_t = 4)]
    pub small_read_count: usize,
    /// Private allocator evidence for the benchmark runner, created exclusively.
    #[arg(long)]
    pub stats_file: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum IamFlavor {
    Gcp,
    #[value(name = "imdsv1")]
    Imds,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum SignatureVersion {
    Auto,
    #[value(name = "4")]
    V4,
    #[value(name = "2")]
    V2,
    Anonymous,
}

/// Never expose an encryption key through derived Config diagnostics.
#[derive(Clone)]
pub struct CustomerKey {
    pub encoded: String,
    pub digest: String,
}
impl std::fmt::Debug for CustomerKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CustomerKey(REDACTED)")
    }
}
impl std::str::FromStr for CustomerKey {
    type Err = String;
    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        use base64::Engine;
        use md5::{Digest, Md5};
        let key = base64::engine::general_purpose::STANDARD
            .decode(value)
            .map_err(|_| "sse-c must be base64".to_owned())?;
        if key.len() != 32 {
            return Err("sse-c requires a 256-bit key".into());
        }
        Ok(Self {
            encoded: value.into(),
            digest: base64::engine::general_purpose::STANDARD.encode(Md5::digest(&key)),
        })
    }
}
fn parse_duration(value: &str) -> std::result::Result<Duration, String> {
    humantime::parse_duration(value).map_err(|_| "invalid finite duration".into())
}

fn parse_mode(s: &str) -> std::result::Result<u16, String> {
    // Go's flag integer parser accepts octal with a leading zero, decimal otherwise.
    let value = if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u16::from_str_radix(hex, 16)
    } else if s.starts_with('0') && s.len() > 1 {
        u16::from_str_radix(&s[1..], 8)
    } else {
        s.parse()
    }
    .map_err(|_| "invalid file mode".to_string())?;
    if value > 0o7777 {
        return Err("file mode exceeds permission bits".into());
    }
    Ok(value)
}

/// Reference MessageMountFlags normalization, only for the exact fstab shape.
pub fn normalize_fstab(args: Vec<OsString>) -> Vec<OsString> {
    if args.len() != 5 || args[3] != "-o" {
        return args;
    }
    let Some(options) = args[4].to_str() else {
        return args;
    };
    let mut result = vec![args[0].clone()];
    let mut mount = Vec::new();
    for option in options.split(',') {
        if option.starts_with('-') {
            result.push(OsString::from(option));
        } else {
            mount.push(option);
        }
    }
    if !mount.is_empty() {
        result.extend([OsString::from("-o"), OsString::from(mount.join(","))]);
    }
    result.extend([args[1].clone(), args[2].clone()]);
    result
}

impl Config {
    pub fn effective_region(&self) -> &str {
        self.region.as_deref().unwrap_or("us-east-1")
    }
    pub fn specials_enabled(&self) -> bool {
        !self.no_specials && (self.enable_specials || self.endpoint.contains("yandex"))
    }
    pub fn validate(&self) -> Result<()> {
        for timeout in [self.request_timeout, self.shutdown_timeout] {
            if timeout.is_zero() || timeout > Duration::from_secs(86400) {
                return Err(Error::Config(
                    "request-timeout and shutdown-timeout must be in (0,24h]".into(),
                ));
            }
        }
        if self.single_part > 5120 {
            return Err(Error::Config("single-part must not exceed 5120 MiB".into()));
        }
        if (self.drop_patch_conflicts || self.prefer_patch_uploads) && !self.enable_patch {
            return Err(Error::Config(
                "drop-patch-conflicts/prefer-patch-uploads require enable-patch".into(),
            ));
        }
        if self.region.as_ref().is_some_and(String::is_empty) {
            return Err(Error::Config("region cannot be empty".into()));
        }
        if self.iam && self.signature_version != SignatureVersion::Auto {
            return Err(Error::Config(
                "--iam cannot be combined with an explicit signature-version".into(),
            ));
        }
        if self.storage_class.is_empty() || self.profile.as_ref().is_some_and(String::is_empty) {
            return Err(Error::Config(
                "storage-class/profile cannot be empty".into(),
            ));
        }
        if ["host", "content-length", "transfer-encoding", "connection"]
            .contains(&self.iam_header.to_ascii_lowercase().as_str())
        {
            return Err(Error::Config(
                "iam-header cannot replace HTTP framing/routing headers".into(),
            ));
        }
        if self.multipart_copy_threshold == 0 || self.multipart_copy_threshold > 5120 {
            return Err(Error::Config(
                "multipart-copy-threshold must be in 1..=5120 MiB".into(),
            ));
        }
        let endpoint = reqwest::Url::parse(&self.endpoint)
            .map_err(|_| Error::Config("invalid endpoint URL".into()))?;
        if !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
        {
            return Err(Error::Config(
                "endpoint cannot contain credentials, query or fragment".into(),
            ));
        }
        if self.http_timeout.is_zero()
            || self.http_timeout > Duration::from_secs(86400)
            || self.sdk_max_retries > 100
        {
            return Err(Error::Config(
                "http-timeout must be in (0,24h], sdk-max-retries <=100".into(),
            ));
        }
        if self.sse_c.is_some() && (self.sse || self.sse_kms.is_some()) {
            return Err(Error::Config(
                "sse-c cannot be combined with sse/sse-kms".into(),
            ));
        }
        for value in [
            Some(self.storage_class.as_str()),
            self.acl.as_deref(),
            self.sse_kms.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            reqwest::header::HeaderValue::from_str(value)
                .map_err(|_| Error::Config("invalid S3 request header value".into()))?;
        }
        reqwest::header::HeaderName::from_bytes(self.iam_header.as_bytes())
            .map_err(|_| Error::Config("invalid iam-header".into()))?;
        if self.iam {
            if let Some(url) = &self.iam_url {
                let url = reqwest::Url::parse(url)
                    .map_err(|_| Error::Config("invalid iam-url".into()))?;
                if !matches!(url.scheme(), "http" | "https")
                    || url.host_str().is_none()
                    || !url.username().is_empty()
                    || url.password().is_some()
                {
                    return Err(Error::Config(
                        "iam-url must be an HTTP(S) URL without credentials".into(),
                    ));
                }
            }
        } else if self.iam_url.is_some()
            || self.iam_flavor != IamFlavor::Gcp
            || self.iam_header != "X-YaCloud-SubjectToken"
        {
            return Err(Error::Config("iam-url/flavor/header require --iam".into()));
        }
        let (bucket, prefix) = self.bucket_and_prefix();
        if bucket.is_empty() || bucket.contains('/') || prefix.split('/').any(|p| p == "..") {
            return Err(Error::Config("invalid bucket[:prefix]".into()));
        }
        if !(self.endpoint.starts_with("https://") || self.endpoint.starts_with("http://")) {
            return Err(Error::Config("endpoint must use http or https".into()));
        }
        if self.memory_limit == 0 || self.memory_limit.checked_mul(1024 * 1024).is_none() {
            return Err(Error::Config("invalid memory-limit in MiB".into()));
        }
        if self.entry_limit == 0 {
            return Err(Error::Config("entry-limit must be positive".into()));
        }
        if self.max_flushers == 0
            || self.max_flushers > 4096
            || self.max_requests == 0
            || self.max_requests > 4096
            || self.max_parallel_parts == 0
            || self.max_parallel_parts > 4096
            || self.max_parallel_copy == 0
            || self.max_parallel_copy > 4096
        {
            return Err(Error::Config(
                "max-flushers and max-requests must be in 1..=4096".into(),
            ));
        }
        if self.small_read_count == 0
            || self.small_read_count > 64
            || [
                self.read_ahead,
                self.read_ahead_small,
                self.read_ahead_large,
                self.small_read_cutoff,
                self.read_ahead_parallel,
            ]
            .iter()
            .any(|n| n.checked_mul(1024).is_none())
            || self.large_read_cutoff.checked_mul(1024).is_none()
        {
            return Err(Error::Config(
                "read windows must fit bytes and small-read-count must be 1..=64".into(),
            ));
        }
        if self.read_ahead_parallel == 0 {
            return Err(Error::Config(
                "read-ahead-parallel must be positive KiB".into(),
            ));
        }
        if self.read_only_direct_io && !self.mount_options.iter().any(|o| o == "ro") {
            return Err(Error::Config("read-only-direct-io requires -o ro".into()));
        }
        for option in &self.mount_options {
            if !matches!(
                option.as_str(),
                "ro" | "rw" | "nosuid" | "nodev" | "noexec" | "default_permissions"
            ) {
                return Err(Error::Unsupported(format!("mount option {option}")));
            }
        }
        Ok(())
    }

    pub fn bucket_and_prefix(&self) -> (&str, &str) {
        self.bucket_prefix
            .split_once(':')
            .unwrap_or((&self.bucket_prefix, ""))
    }

    pub fn effective_list_type(&self) -> ListType {
        self.list_type.unwrap_or_else(|| {
            if self.endpoint.contains("yandex") {
                ListType::Extended
            } else {
                ListType::V1
            }
        })
    }
}
