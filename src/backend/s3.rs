use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};

use aws_sdk_s3::{
    Client,
    config::{BehaviorVersion, Region},
    error::ProvideErrorMetadata,
    primitives::ByteStream,
    types::{CompletedMultipartUpload, CompletedPart},
};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};

use crate::{
    budget::{Block, Budget},
    config::{Config, ListType, SignatureVersion},
    error::{Error, Result},
};

/// One SDK client owns ordinary transport, TLS, credentials, signing and retries.
#[derive(Clone)]
pub struct S3 {
    client: Client,
    bucket: String,
    prefix: String,
    read_timeout: Duration,
    list_mode: Arc<AtomicU8>,
    extension_http: reqwest::Client,
    extension_attempts: u32,
    multipart_age: Option<Duration>,
    copy_threshold: Option<u64>,
    storage_class: String,
    cold_min_size: u64,
    cleanup_slots: Arc<tokio::sync::Semaphore>,
    cleanup_limit: u32,
    copy_parallelism: usize,
    copy_slots: Arc<tokio::sync::Semaphore>,
    signature: SignatureVersion,
    detect_region: bool,
    try_anonymous: bool,
    no_detect: bool,
    regional_aws_endpoint: bool,
}

#[derive(Clone, Debug, Default)]
pub struct ObjectHead {
    pub size: u64,
    pub etag: String,
    pub metadata: HashMap<String, String>,
    pub last_modified: Option<std::time::SystemTime>,
    pub properties: ObjectProperties,
}

/// HTTP object properties are distinct from x-amz-meta user attributes. COPY
/// with REPLACE and multipart initiation must carry them explicitly.
#[derive(Clone, Debug, Default)]
pub struct ObjectProperties {
    pub content_type: Option<String>,
    pub content_encoding: Option<String>,
    pub content_disposition: Option<String>,
    pub content_language: Option<String>,
    pub cache_control: Option<String>,
    pub expires: Option<aws_smithy_types::DateTime>,
    pub website_redirect_location: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PatchResult {
    pub etag: String,
    pub last_modified: aws_smithy_types::DateTime,
}

fn backend(operation: &'static str, code: Option<&str>, error: impl std::fmt::Display) -> Error {
    Error::Backend {
        operation,
        code: code.unwrap_or("TransportOrProtocol").into(),
        message: error.to_string(),
    }
}

impl S3 {
    /// Bounded, read-only reference-style startup discovery. Explicit auth modes
    /// and --no-detect skip it; no failure silently downgrades a mounted client.
    pub async fn initialize(self) -> Result<Self> {
        let deadline = self.read_timeout.saturating_mul(4);
        tokio::time::timeout(deadline, self.initialize_inner())
            .await
            .map_err(|_| Error::Timeout("S3 initialization"))?
    }
    async fn initialize_inner(mut self) -> Result<Self> {
        if self.no_detect || self.signature != SignatureVersion::Auto {
            return Ok(self);
        }
        if self.detect_region {
            let anonymous = Client::from_conf(
                self.client
                    .config()
                    .to_builder()
                    .allow_no_auth()
                    .auth_scheme_resolver(super::signature::AnonymousResolver)
                    .build(),
            );
            let (anonymous_ok, region) =
                match anonymous.head_bucket().bucket(&self.bucket).send().await {
                    Ok(output) => (true, output.bucket_region),
                    Err(error) => {
                        if error
                            .raw_response()
                            .is_some_and(|r| r.status().as_u16() == 404)
                        {
                            return Err(backend(
                                "HeadBucket",
                                Some("NoSuchBucket"),
                                "bucket does not exist",
                            ));
                        }
                        (
                            false,
                            error
                                .raw_response()
                                .and_then(|r| r.headers().get("x-amz-bucket-region"))
                                .map(str::to_owned),
                        )
                    }
                };
            if let Some(region) = region {
                if region.is_empty()
                    || region.len() > 128
                    || !region
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                {
                    return Err(Error::InvalidResponse(
                        "invalid bucket region header".into(),
                    ));
                }
                let mut builder = self
                    .client
                    .config()
                    .to_builder()
                    .region(Region::new(region));
                if self.regional_aws_endpoint {
                    builder.set_endpoint_url(None);
                }
                self.client = Client::from_conf(builder.build());
            }
            if anonymous_ok && self.try_anonymous {
                self.client = Client::from_conf(
                    self.client
                        .config()
                        .to_builder()
                        .allow_no_auth()
                        .auth_scheme_resolver(super::signature::AnonymousResolver)
                        .build(),
                );
                self.signature = SignatureVersion::Anonymous;
            }
        }
        for attempt in 0..2 {
            match self
                .client
                .head_object()
                .bucket(&self.bucket)
                .key(self.key("__geesefs_access_probe__")?)
                .send()
                .await
            {
                Ok(_) => return Ok(self),
                Err(error) => {
                    let status = error.raw_response().map(|r| r.status().as_u16());
                    if status == Some(404) {
                        return Ok(self);
                    }
                    if attempt == 0
                        && self.signature == SignatureVersion::Auto
                        && matches!(status, Some(400 | 403 | 429 | 500 | 502 | 503 | 504))
                    {
                        self.client = Client::from_conf(
                            self.client
                                .config()
                                .to_builder()
                                .push_auth_scheme(super::signature::V2Scheme::new(
                                    self.bucket.clone(),
                                ))
                                .build(),
                        );
                        self.signature = SignatureVersion::V2;
                    } else {
                        return Err(backend(
                            "S3 credential probe",
                            error.as_service_error().and_then(|e| e.code()),
                            &error,
                        ));
                    }
                }
            }
        }
        Err(Error::InvalidResponse(
            "S3 initialization did not select authentication".into(),
        ))
    }
    /// One conditional PATCH attempt, never a blind retry of a possibly applied
    /// append. The allocation owner remains charged through request completion.
    pub async fn patch(
        &self,
        key: &str,
        offset: u64,
        body: Block,
        etag: &str,
        append_part_size: Option<u64>,
    ) -> Result<PatchResult> {
        if body.as_slice().is_empty() || etag.is_empty() || append_part_size == Some(0) {
            return Err(Error::Config(
                "PATCH requires nonempty body/ETag and positive append part size".into(),
            ));
        }
        let length = body.as_slice().len();
        let end = offset
            .checked_add(length as u64 - 1)
            .ok_or_else(|| Error::Config("PATCH range overflow".into()))?;
        reqwest::header::HeaderValue::from_str(etag)
            .map_err(|_| Error::Config("invalid PATCH ETag".into()))?;
        let key = self.key(key)?;
        tokio::time::timeout(self.read_timeout, async {
            let etag = etag.to_owned();
            // GET has no upload checksum middleware. Customize its request before
            // SDK presigning rather than signing an empty PUT body as PATCH data.
            let signed = self
                .client
                .get_object()
                .bucket(&self.bucket)
                .key(key)
                .customize()
                .mutate_request(move |r| {
                    r.set_method("PATCH").expect("static HTTP method");
                    r.headers_mut()
                        .insert("content-range", format!("bytes {offset}-{end}/*"));
                    r.headers_mut().insert("content-length", length.to_string());
                    r.headers_mut().insert("if-match", etag.clone());
                    if let Some(size) = append_part_size {
                        r.headers_mut()
                            .insert("x-yc-s3-patch-append-part-size", size.to_string());
                    }
                })
                .presigned(
                    aws_sdk_s3::presigning::PresigningConfig::expires_in(Duration::from_secs(60))
                        .map_err(|_| Error::Config("presigning duration".into()))?,
                )
                .await
                .map_err(|_| backend("PATCH presign", None, "SDK could not presign PATCH"))?;
            let mut request = self
                .extension_http
                .request(reqwest::Method::PATCH, signed.uri());
            for (name, value) in signed.headers() {
                request = request.header(name, value);
            }
            let mut response = request
                .body(bytes::Bytes::from_owner(body))
                .send()
                .await
                .map_err(|e| backend("PATCH", None, e.without_url()))?;
            let status = response.status();
            let mut xml = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|e| backend("PATCH body", None, e.without_url()))?
            {
                if xml.len() + chunk.len() > 65536 {
                    return Err(Error::InvalidResponse(
                        "PATCH response exceeds 64KiB".into(),
                    ));
                }
                xml.extend_from_slice(&chunk);
            }
            if !status.is_success() {
                #[derive(serde::Deserialize)]
                struct ServiceError {
                    #[serde(rename = "Code")]
                    code: String,
                }
                let code = quick_xml::de::from_reader::<_, ServiceError>(xml.as_slice())
                    .map(|e| e.code)
                    .unwrap_or_else(|_| format!("HTTP{}", status.as_u16()));
                if code == "NotImplemented" {
                    return Err(Error::Fs(libc::ENOSYS));
                }
                return Err(backend("PATCH", Some(&code), "extension service error"));
            }
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "PascalCase")]
            struct Response {
                object: Info,
            }
            #[derive(serde::Deserialize)]
            struct Info {
                #[serde(rename = "ETag")]
                etag: String,
                #[serde(rename = "LastModified")]
                modified: String,
            }
            super::extended_list::validate_document(&xml, "PatchObjectResult")?;
            let output: Response = quick_xml::de::from_reader(xml.as_slice())
                .map_err(|_| Error::InvalidResponse("invalid PATCH response XML".into()))?;
            if output.object.etag.is_empty() {
                return Err(Error::InvalidResponse("PATCH response missing ETag".into()));
            }
            let last_modified = aws_smithy_types::DateTime::from_str(
                &output.object.modified,
                aws_smithy_types::date_time::Format::DateTime,
            )
            .map_err(|_| Error::InvalidResponse("PATCH response invalid LastModified".into()))?;
            Ok(PatchResult {
                etag: output.object.etag,
                last_modified,
            })
        })
        .await
        .map_err(|_| Error::Timeout("PATCH presign/request/body"))?
    }
    /// Startup cleanup is confined to this mounted prefix. Unlike the reference's
    /// first-page-only background loop, every page is processed with a deadline.
    /// The caller must only invoke this on a bucket/prefix it owns.
    pub async fn expire_multipart(&self, now: std::time::SystemTime) -> Result<usize> {
        let Some(age) = self.multipart_age else {
            return Ok(0);
        };
        tokio::time::timeout(self.read_timeout, self.expire_multipart_inner(now, age))
            .await
            .map_err(|_| Error::Timeout("multipart expiry"))?
    }
    async fn expire_multipart_inner(
        &self,
        now: std::time::SystemTime,
        age: Duration,
    ) -> Result<usize> {
        let mut marker: Option<(String, String)> = None;
        let mut count = 0;
        loop {
            let page = self
                .client
                .list_multipart_uploads()
                .bucket(&self.bucket)
                .prefix(&self.prefix)
                .max_uploads(1000)
                .set_key_marker(marker.as_ref().map(|m| m.0.clone()))
                .set_upload_id_marker(marker.as_ref().map(|m| m.1.clone()))
                .send()
                .await
                .map_err(|e| {
                    backend(
                        "ListMultipartUploads",
                        e.as_service_error().and_then(|e| e.code()),
                        &e,
                    )
                })?;
            if page.uploads().len() > 1000 {
                return Err(Error::InvalidResponse(
                    "multipart listing entry limit".into(),
                ));
            }
            // Validate confinement before performing any abort from this page.
            for upload in page.uploads() {
                let key = upload
                    .key()
                    .ok_or_else(|| Error::InvalidResponse("multipart entry missing key".into()))?;
                let relative = key.strip_prefix(&self.prefix).ok_or_else(|| {
                    Error::InvalidResponse("multipart entry escapes prefix".into())
                })?;
                self.key(relative)?;
                if upload.upload_id().is_none_or(str::is_empty) || upload.initiated().is_none() {
                    return Err(Error::InvalidResponse(
                        "multipart entry missing identity/time".into(),
                    ));
                }
            }
            for upload in page.uploads() {
                let initiated = std::time::SystemTime::try_from(*upload.initiated().unwrap())
                    .map_err(|_| {
                        Error::InvalidResponse("invalid multipart initiation time".into())
                    })?;
                if now
                    .duration_since(initiated)
                    .is_ok_and(|elapsed| elapsed >= age)
                {
                    let key = upload.key().unwrap().strip_prefix(&self.prefix).unwrap();
                    match self.abort_upload(key, upload.upload_id().unwrap()).await {
                        Ok(()) => count += 1,
                        Err(Error::Backend { ref code, .. }) if code == "NoSuchUpload" => {}
                        Err(error) => return Err(error),
                    }
                }
            }
            if !page.is_truncated().unwrap_or(false) {
                return Ok(count);
            }
            let key = page
                .next_key_marker()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    Error::InvalidResponse("multipart listing missing key marker".into())
                })?;
            let upload = page
                .next_upload_id_marker()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    Error::InvalidResponse("multipart listing missing upload marker".into())
                })?;
            let next = (key.to_owned(), upload.to_owned());
            if !key.starts_with(&self.prefix)
                || marker
                    .as_ref()
                    .is_some_and(|old| old == &next || old.0 > next.0)
            {
                return Err(Error::InvalidResponse(
                    "multipart listing invalid/repeated marker".into(),
                ));
            }
            marker = Some(next);
        }
    }
    pub async fn from_config(config: &Config) -> Result<Self> {
        config.validate()?;
        let mut loader = aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new(config.effective_region().to_owned()))
            .timeout_config(
                aws_sdk_s3::config::timeout::TimeoutConfig::builder()
                    .operation_timeout(config.http_timeout)
                    .operation_attempt_timeout(config.http_timeout)
                    .connect_timeout(config.http_timeout)
                    .read_timeout(config.http_timeout)
                    .build(),
            );
        if let Some(profile) = &config.profile {
            loader = loader.profile_name(profile);
        }
        if config.iam || config.signature_version == SignatureVersion::Anonymous {
            loader = loader.no_credentials();
        }
        let mut profile_files = None;
        if !config.shared_config.is_empty() {
            let mut files = aws_runtime::env_config::file::EnvConfigFiles::builder();
            for path in &config.shared_config {
                files = files.with_file(
                    aws_runtime::env_config::file::EnvConfigFileKind::Config,
                    path,
                );
            }
            let files = files.build();
            loader = loader.profile_files(files.clone());
            profile_files = Some(files);
        }
        if !config.iam
            && config.signature_version != SignatureVersion::Anonymous
            && let Some(profile) = &config.profile
        {
            // Explicit Go session profiles override environment credentials.
            // Do not silently fall through to ambient keys on profile failure.
            let provider_config = aws_config::provider_config::ProviderConfig::without_region()
                .with_region(Some(Region::new(config.effective_region().to_owned())))
                .with_timeout_config(
                    aws_sdk_s3::config::timeout::TimeoutConfig::builder()
                        .operation_timeout(config.http_timeout)
                        .operation_attempt_timeout(config.http_timeout)
                        .connect_timeout(config.http_timeout)
                        .read_timeout(config.http_timeout)
                        .build(),
                )
                .with_retry_config(
                    aws_sdk_s3::config::retry::RetryConfig::standard()
                        .with_max_attempts(config.sdk_max_retries + 1),
                );
            let mut provider = aws_config::profile::ProfileFileCredentialsProvider::builder()
                .configure(&provider_config)
                .profile_name(profile);
            if let Some(files) = profile_files {
                provider = provider.profile_files(files);
            }
            loader = loader.credentials_provider(provider.build());
        }
        let shared = tokio::time::timeout(config.http_timeout, loader.load())
            .await
            .map_err(|_| Error::Timeout("AWS configuration load"))?;
        let client_config = aws_sdk_s3::config::Builder::from(&shared)
            .endpoint_url(&config.endpoint)
            .force_path_style(!config.subdomain)
            .timeout_config(
                aws_sdk_s3::config::timeout::TimeoutConfig::builder()
                    .operation_timeout(config.http_timeout)
                    .operation_attempt_timeout(config.http_timeout)
                    .build(),
            )
            .build();
        let (bucket, prefix) = config.bucket_and_prefix();
        Self::new(
            Client::from_conf(client_config),
            bucket.into(),
            prefix.into(),
        )?
        .with_options(config)
    }

    /// Accept an explicitly configured client for deterministic disposable tests.
    pub fn new(client: Client, bucket: String, prefix: String) -> Result<Self> {
        if bucket.is_empty() || bucket.contains('/') || prefix.split('/').any(|p| p == "..") {
            return Err(Error::Config("invalid bucket or prefix".into()));
        }
        let prefix = if prefix.is_empty() {
            prefix
        } else {
            format!("{}/", prefix.trim_end_matches('/'))
        };
        let read_timeout = client
            .config()
            .timeout_config()
            .and_then(|t| t.operation_timeout())
            .unwrap_or(Duration::from_secs(30));
        let client = if client
            .config()
            .timeout_config()
            .and_then(|t| t.operation_timeout())
            .is_none()
        {
            let timeout = client
                .config()
                .timeout_config()
                .cloned()
                .unwrap_or_else(|| aws_sdk_s3::config::timeout::TimeoutConfig::builder().build())
                .into_builder()
                .operation_timeout(read_timeout)
                .build();
            Client::from_conf(client.config().to_builder().timeout_config(timeout).build())
        } else {
            client
        };
        let extension_attempts = client
            .config()
            .retry_config()
            .map_or(3, |r| r.max_attempts());
        Ok(Self {
            client,
            bucket,
            prefix,
            read_timeout,
            list_mode: Arc::new(AtomicU8::new(1)),
            multipart_age: None,
            copy_threshold: None,
            storage_class: "STANDARD".into(),
            cold_min_size: 0,
            cleanup_slots: Arc::new(tokio::sync::Semaphore::new(64)),
            cleanup_limit: 64,
            copy_parallelism: 8,
            copy_slots: Arc::new(tokio::sync::Semaphore::new(16)),
            signature: SignatureVersion::V4,
            detect_region: false,
            try_anonymous: false,
            no_detect: true,
            regional_aws_endpoint: false,
            extension_attempts,
            extension_http: reqwest::Client::builder()
                .timeout(read_timeout)
                .gzip(true)
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| Error::Config("extension HTTP client initialization failed".into()))?,
        })
    }

    /// Also used with an explicit disposable SDK client by contract tests.
    pub fn with_options(mut self, config: &Config) -> Result<Self> {
        config.validate()?;
        let mut builder = self
            .client
            .config()
            .to_builder()
            .force_path_style(!config.subdomain)
            .interceptor(super::options::RequestOptions::new(config)?)
            .retry_config(
                aws_sdk_s3::config::retry::RetryConfig::standard()
                    .with_max_attempts(config.sdk_max_retries + 1),
            )
            .timeout_config(
                aws_sdk_s3::config::timeout::TimeoutConfig::builder()
                    .operation_timeout(config.http_timeout)
                    .operation_attempt_timeout(config.http_timeout)
                    .build(),
            );
        if config.no_verify_ssl {
            builder = builder.http_client(super::transport::InsecureTransport::new(
                config.http_timeout,
            )?);
        }
        if config.no_checksum {
            builder = builder
                .request_checksum_calculation(
                    aws_sdk_s3::config::RequestChecksumCalculation::WhenRequired,
                )
                .response_checksum_validation(
                    aws_sdk_s3::config::ResponseChecksumValidation::WhenRequired,
                );
        }
        if config.iam {
            builder = builder
                .push_auth_scheme(super::auth::IamScheme::new(
                    super::auth::IamProvider::new(config)?,
                    config.iam_header.clone(),
                ))
                .auth_scheme_resolver(super::auth::IamResolver)
                .identity_cache(aws_sdk_s3::config::IdentityCache::no_cache());
        } else if config.signature_version == SignatureVersion::V2 {
            builder =
                builder.push_auth_scheme(super::signature::V2Scheme::new(self.bucket.clone()));
        } else if config.signature_version == SignatureVersion::Anonymous {
            builder = builder
                .allow_no_auth()
                .auth_scheme_resolver(super::signature::AnonymousResolver);
        }
        self.client = Client::from_conf(builder.build());
        self.read_timeout = config.http_timeout;
        self.extension_attempts = config.sdk_max_retries + 1;
        self.extension_http = reqwest::Client::builder()
            .danger_accept_invalid_certs(config.no_verify_ssl)
            .timeout(config.http_timeout)
            .redirect(reqwest::redirect::Policy::none())
            .gzip(true)
            .build()
            .map_err(|_| Error::Config("extension HTTP client initialization failed".into()))?;
        self.multipart_age = (!config.no_expire_multipart).then_some(config.multipart_age);
        self.copy_threshold = Some(config.multipart_copy_threshold * 1024 * 1024);
        self.storage_class = config.storage_class.clone();
        self.cold_min_size = config.cold_min_size;
        self.cleanup_slots = Arc::new(tokio::sync::Semaphore::new(config.max_requests));
        self.cleanup_limit = config.max_requests as u32;
        self.copy_parallelism = config.max_parallel_parts.min(config.max_requests);
        self.copy_slots = Arc::new(tokio::sync::Semaphore::new(config.max_parallel_copy));
        self.signature = config.signature_version;
        self.no_detect = config.no_detect || config.iam;
        self.regional_aws_endpoint = reqwest::Url::parse(&config.endpoint)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned))
            .is_some_and(|host| {
                (host == "s3.amazonaws.com" || host.starts_with("s3.") || host.starts_with("s3-"))
                    && (host.ends_with(".amazonaws.com") || host.ends_with(".amazonaws.com.cn"))
            });
        self.detect_region = config.region.is_none();
        self.try_anonymous = config.profile.is_none()
            && std::env::var_os("AWS_ACCESS_KEY_ID").is_none_or(|key| key.is_empty());
        Ok(self.with_list_type(config.effective_list_type()))
    }

    pub fn with_list_type(self, mode: ListType) -> Self {
        self.list_mode.store(
            match mode {
                ListType::V1 => 1,
                ListType::V2 => 2,
                ListType::Extended => 3,
            },
            Ordering::Relaxed,
        );
        self
    }

    pub async fn list(
        &self,
        relative_prefix: &str,
        delimiter: Option<&str>,
        token: Option<&str>,
    ) -> Result<super::extended_list::ListPage> {
        use super::extended_list::{ListPage, ListedObject, MAX_PAGE_ENTRIES};
        let prefix = self.key(relative_prefix)?;
        let mut mode = self.list_mode.load(Ordering::Relaxed);
        if mode == 3 {
            match self.list_extended(&prefix, delimiter, token).await {
                Ok(mut page) => {
                    self.relativize_page(&mut page)?;
                    return Ok(page);
                }
                Err(Error::Backend { ref code, .. })
                    if super::extended_list::permits_standard_fallback(code) && token.is_none() =>
                {
                    self.list_mode.store(1, Ordering::Relaxed);
                    mode = 1;
                }
                Err(error) => return Err(error),
            }
        }
        let (objects, prefixes, truncated, mut next) = if mode == 2 {
            let page = self
                .client
                .list_objects_v2()
                .bucket(&self.bucket)
                .prefix(&prefix)
                .set_delimiter(delimiter.map(str::to_owned))
                .set_continuation_token(token.map(str::to_owned))
                .max_keys(1000)
                .send()
                .await
                .map_err(|e| {
                    backend(
                        "ListObjectsV2",
                        e.as_service_error().and_then(|e| e.code()),
                        &e,
                    )
                })?;
            (
                page.contents.unwrap_or_default(),
                page.common_prefixes.unwrap_or_default(),
                page.is_truncated.unwrap_or(false),
                page.next_continuation_token,
            )
        } else {
            let page = self
                .client
                .list_objects()
                .bucket(&self.bucket)
                .prefix(&prefix)
                .set_delimiter(delimiter.map(str::to_owned))
                .set_marker(token.map(str::to_owned))
                .max_keys(1000)
                .send()
                .await
                .map_err(|e| {
                    backend(
                        "ListObjects",
                        e.as_service_error().and_then(|e| e.code()),
                        &e,
                    )
                })?;
            (
                page.contents.unwrap_or_default(),
                page.common_prefixes.unwrap_or_default(),
                page.is_truncated.unwrap_or(false),
                page.next_marker,
            )
        };
        if objects.len() + prefixes.len() > MAX_PAGE_ENTRIES {
            return Err(Error::InvalidResponse("listing entry limit".into()));
        }
        if truncated && next.is_none() && mode == 1 {
            next = objects
                .iter()
                .filter_map(|o| o.key.clone())
                .chain(prefixes.iter().filter_map(|p| p.prefix.clone()))
                .max();
        }
        if truncated
            && next
                .as_deref()
                .is_none_or(|s| s.is_empty() || Some(s) == token)
        {
            return Err(Error::InvalidResponse(
                "missing/repeated listing token".into(),
            ));
        }
        let mut page = ListPage {
            objects: Vec::with_capacity(objects.len()),
            common_prefixes: Vec::with_capacity(prefixes.len()),
            next_token: if truncated { next } else { None },
        };
        let mut previous: Option<String> = None;
        for object in objects {
            let key = object
                .key
                .ok_or_else(|| Error::InvalidResponse("listing missing key".into()))?;
            if !key.starts_with(&prefix) || previous.as_ref().is_some_and(|p| *p >= key) {
                return Err(Error::InvalidResponse(
                    "unordered/unconfined listing".into(),
                ));
            }
            previous = Some(key.clone());
            let size = object
                .size
                .and_then(|s| u64::try_from(s).ok())
                .ok_or_else(|| Error::InvalidResponse("listing invalid size".into()))?;
            page.objects.push(ListedObject {
                key,
                size,
                etag: object.e_tag,
                last_modified: object.last_modified,
                metadata: None,
            });
        }
        for common in prefixes {
            let key = common
                .prefix
                .ok_or_else(|| Error::InvalidResponse("listing missing prefix".into()))?;
            if !key.starts_with(&prefix) || !key.ends_with('/') {
                return Err(Error::InvalidResponse("unconfined prefix".into()));
            }
            page.common_prefixes.push(key);
        }
        self.relativize_page(&mut page)?;
        Ok(page)
    }

    fn relativize_page(&self, page: &mut super::extended_list::ListPage) -> Result<()> {
        for key in page
            .objects
            .iter_mut()
            .map(|o| &mut o.key)
            .chain(page.common_prefixes.iter_mut())
        {
            *key = key
                .strip_prefix(&self.prefix)
                .ok_or_else(|| Error::InvalidResponse("listing escapes mounted prefix".into()))?
                .to_owned();
        }
        Ok(())
    }

    async fn list_extended(
        &self,
        prefix: &str,
        delimiter: Option<&str>,
        token: Option<&str>,
    ) -> Result<super::extended_list::ListPage> {
        tokio::time::timeout(
            self.read_timeout,
            self.list_extended_inner(prefix, delimiter, token),
        )
        .await
        .map_err(|_| Error::Timeout("ext-v1 presign/retries/body"))?
    }
    async fn list_extended_inner(
        &self,
        prefix: &str,
        delimiter: Option<&str>,
        token: Option<&str>,
    ) -> Result<super::extended_list::ListPage> {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct ServiceError {
            code: String,
        }
        let prefix_owned = prefix.to_owned();
        let delimiter_owned = delimiter.map(str::to_owned);
        let token_owned = token.map(str::to_owned);
        // The SDK does not expose LIST presigning. Reuse its supported GET
        // endpoint resolution/auth/signing pipeline, customizing only the URI
        // before signing. No request is made for the placeholder key.
        let signed = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key("__geesefs_extension_placeholder__")
            .customize()
            .mutate_request(move |r| {
                let mut uri = reqwest::Url::parse(r.uri()).expect("SDK produces valid HTTP URI");
                let path = uri
                    .path()
                    .strip_suffix("/__geesefs_extension_placeholder__")
                    .expect("SDK placeholder path suffix")
                    .to_owned();
                uri.set_path(&path);
                {
                    let mut query = uri.query_pairs_mut();
                    query
                        .clear()
                        .append_pair("list-type", "ext-v1")
                        .append_pair("max-keys", "1000")
                        .append_pair("prefix", &prefix_owned);
                    if let Some(delimiter) = &delimiter_owned {
                        query.append_pair("delimiter", delimiter);
                    }
                    if let Some(token) = &token_owned {
                        query.append_pair("continuation-token", token);
                    }
                }
                r.set_uri(uri.as_str())
                    .expect("URL serialization produces valid HTTP URI");
            })
            .presigned(
                aws_sdk_s3::presigning::PresigningConfig::expires_in(Duration::from_secs(60))
                    .map_err(|_| Error::Config("presigning duration".into()))?,
            )
            .await
            .map_err(|_| backend("ext-v1 presign", None, "SDK could not presign extension"))?;
        // The presigned URL is a credential. Never include it in logs/errors.
        for attempt in 0..self.extension_attempts {
            let mut request = self.extension_http.get(signed.uri());
            for (name, value) in signed.headers() {
                request = request.header(name, value);
            }
            let response = request.send().await;
            let mut response = match response {
                Ok(response) => response,
                Err(error)
                    if attempt + 1 < self.extension_attempts
                        && (error.is_connect() || error.is_timeout()) =>
                {
                    tokio::time::sleep(Duration::from_millis(30 * (1 << attempt.min(8)))).await;
                    continue;
                }
                Err(error) => return Err(backend("ext-v1", None, error.without_url())),
            };
            let status = response.status();
            let mut xml = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|e| backend("ext-v1 body", None, e.without_url()))?
            {
                if xml.len() + chunk.len() > super::extended_list::MAX_PAGE_BYTES {
                    return Err(Error::InvalidResponse(
                        "extension page exceeds byte limit".into(),
                    ));
                }
                xml.extend_from_slice(&chunk);
            }
            if status.is_success() {
                return super::extended_list::decode_page(&xml, prefix, token);
            }
            let code = quick_xml::de::from_reader::<_, ServiceError>(xml.as_slice())
                .map(|e| e.code)
                .unwrap_or_else(|_| format!("HTTP{}", status.as_u16()));
            if attempt + 1 < self.extension_attempts
                && (status.is_server_error() || status.as_u16() == 429)
                && !super::extended_list::permits_standard_fallback(&code)
            {
                tokio::time::sleep(Duration::from_millis(30 * (1 << attempt.min(8)))).await;
                continue;
            }
            return Err(backend("ext-v1", Some(&code), "extension service error"));
        }
        Err(Error::Timeout("ext-v1 retries"))
    }

    fn key(&self, relative: &str) -> Result<String> {
        if relative.starts_with('/') || relative.split('/').any(|p| p == ".." || p == ".") {
            return Err(Error::Config("object key escapes mount prefix".into()));
        }
        Ok(format!("{}{relative}", self.prefix))
    }

    fn copy_source(&self, relative: &str) -> Result<String> {
        // SDK expects the source header already URL encoded; keep '/' separators.
        let source = format!("{}/{}", self.bucket, self.key(relative)?);
        Ok(source
            .split('/')
            .map(|part| utf8_percent_encode(part, NON_ALPHANUMERIC).to_string())
            .collect::<Vec<_>>()
            .join("/"))
    }

    pub async fn head(&self, key: &str) -> Result<ObjectHead> {
        Ok(self.head_details(key).await?.0)
    }
    async fn head_details(&self, key: &str) -> Result<(ObjectHead, Option<String>)> {
        let response = self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(self.key(key)?)
            .send()
            .await
            .map_err(|e| backend("HEAD", e.as_service_error().and_then(|e| e.code()), &e))?;
        let size = response
            .content_length()
            .and_then(|s| u64::try_from(s).ok())
            .ok_or_else(|| Error::InvalidResponse("HEAD missing valid Content-Length".into()))?;
        let etag = response
            .e_tag()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::InvalidResponse("HEAD missing ETag".into()))?
            .to_owned();
        Ok((
            ObjectHead {
                size,
                etag,
                metadata: response.metadata.unwrap_or_default(),
                last_modified: response
                    .last_modified
                    .and_then(|d| std::time::SystemTime::try_from(d).ok()),
                properties: ObjectProperties {
                    content_type: response.content_type,
                    content_encoding: response.content_encoding,
                    content_disposition: response.content_disposition,
                    content_language: response.content_language,
                    cache_control: response.cache_control,
                    expires: response
                        .expires_string
                        .as_deref()
                        .map(|s| {
                            aws_smithy_types::DateTime::from_str(
                                s,
                                aws_smithy_types::date_time::Format::HttpDate,
                            )
                        })
                        .transpose()
                        .map_err(|_| {
                            Error::InvalidResponse("invalid Expires object header".into())
                        })?,
                    website_redirect_location: response.website_redirect_location,
                },
            },
            response
                .storage_class
                .map(|class| class.as_str().to_owned()),
        ))
    }

    /// Read exactly the requested intersection with a known object generation.
    /// Admission happens before GET. Never collect the body or decode stored gzip.
    /// SDK/HTTP transport buffers are additional overhead, not charged payload bytes.
    pub async fn read_range(
        &self,
        key: &str,
        offset: u64,
        length: usize,
        head: &ObjectHead,
        budget: &Budget,
    ) -> Result<Block> {
        tokio::time::timeout(
            self.read_timeout,
            self.read_range_inner(key, offset, length, head, budget),
        )
        .await
        .map_err(|_| Error::Timeout("GET admission/response/body"))?
    }

    async fn read_range_inner(
        &self,
        key: &str,
        offset: u64,
        length: usize,
        head: &ObjectHead,
        budget: &Budget,
    ) -> Result<Block> {
        let length = u64::try_from(length)
            .map_err(|_| Error::Config("read length overflow".into()))?
            .min(head.size.saturating_sub(offset));
        let mut block = budget.reserve(length as usize).await?.zeroed();
        if length == 0 {
            return Ok(block);
        }
        if head.etag.is_empty() {
            return Err(Error::Config("conditional read requires ETag".into()));
        }
        let end = offset
            .checked_add(length - 1)
            .ok_or_else(|| Error::Config("range overflow".into()))?;
        let mut response = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(self.key(key)?)
            .range(format!("bytes={offset}-{end}"))
            .if_match(&head.etag)
            .send()
            .await
            .map_err(|e| backend("GET", e.as_service_error().and_then(|e| e.code()), &e))?;
        let expected_range = format!("bytes {offset}-{end}/{}", head.size);
        if response.content_length() != Some(length as i64)
            || response.content_range() != Some(expected_range.as_str())
        {
            return Err(Error::InvalidResponse(
                "GET range/length differs from requested generation".into(),
            ));
        }
        let mut filled = 0;
        while let Some(chunk) = response
            .body
            .try_next()
            .await
            .map_err(|e| backend("GET body", None, e))?
        {
            if chunk.len() > length as usize - filled {
                return Err(Error::InvalidResponse("GET body longer than Range".into()));
            }
            block.as_mut_slice()[filled..filled + chunk.len()].copy_from_slice(&chunk);
            filled += chunk.len();
        }
        if filled != length as usize {
            return Err(Error::InvalidResponse("short GET body".into()));
        }
        Ok(block)
    }

    /// Streaming input must be independently bounded by its allocation owner.
    pub async fn put(
        &self,
        key: &str,
        body: ByteStream,
        metadata: HashMap<String, String>,
    ) -> Result<String> {
        let response = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(self.key(key)?)
            .body(body)
            .set_metadata(Some(metadata))
            .send()
            .await
            .map_err(|e| backend("PUT", e.as_service_error().and_then(|e| e.code()), &e))?;
        response
            .e_tag()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| Error::InvalidResponse("PUT missing ETag".into()))
    }

    pub async fn begin_upload(
        &self,
        key: &str,
        metadata: HashMap<String, String>,
        size: u64,
    ) -> Result<String> {
        let storage = if self.storage_class == "STANDARD_IA" && size < self.cold_min_size {
            "STANDARD"
        } else {
            &self.storage_class
        };
        self.begin_upload_storage(
            key,
            metadata,
            Some(storage.into()),
            &ObjectProperties::default(),
        )
        .await
    }
    async fn begin_upload_storage(
        &self,
        key: &str,
        metadata: HashMap<String, String>,
        storage: Option<String>,
        properties: &ObjectProperties,
    ) -> Result<String> {
        let response = self
            .client
            .create_multipart_upload()
            .bucket(&self.bucket)
            .key(self.key(key)?)
            .set_metadata(Some(metadata))
            .set_content_type(properties.content_type.clone())
            .set_content_encoding(properties.content_encoding.clone())
            .set_content_disposition(properties.content_disposition.clone())
            .set_content_language(properties.content_language.clone())
            .set_cache_control(properties.cache_control.clone())
            .set_expires(properties.expires)
            .set_website_redirect_location(properties.website_redirect_location.clone())
            .set_storage_class(storage.map(|s| aws_sdk_s3::types::StorageClass::from(s.as_str())))
            .send()
            .await
            .map_err(|e| {
                backend(
                    "CreateMultipartUpload",
                    e.as_service_error().and_then(|e| e.code()),
                    &e,
                )
            })?;
        response
            .upload_id()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| Error::InvalidResponse("missing upload ID".into()))
    }

    pub async fn upload_part(
        &self,
        key: &str,
        upload: &str,
        part: i32,
        body: ByteStream,
    ) -> Result<CompletedPart> {
        validate_part(part)?;
        let response = self
            .client
            .upload_part()
            .bucket(&self.bucket)
            .key(self.key(key)?)
            .upload_id(upload)
            .part_number(part)
            .body(body)
            .send()
            .await
            .map_err(|e| {
                backend(
                    "UploadPart",
                    e.as_service_error().and_then(|e| e.code()),
                    &e,
                )
            })?;
        let etag = response
            .e_tag()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::InvalidResponse("UploadPart missing ETag".into()))?;
        Ok(CompletedPart::builder()
            .part_number(part)
            .e_tag(etag)
            .build())
    }

    pub async fn copy_part(
        &self,
        destination: &str,
        upload: &str,
        part: i32,
        source: &str,
        range: std::ops::RangeInclusive<u64>,
        etag: &str,
    ) -> Result<CompletedPart> {
        validate_part(part)?;
        let _slot =
            tokio::time::timeout(self.read_timeout, self.copy_slots.clone().acquire_owned())
                .await
                .map_err(|_| Error::Timeout("copy part admission"))?
                .map_err(|_| Error::Cancelled)?;
        if range.is_empty() || etag.is_empty() {
            return Err(Error::Config(
                "copy requires ordered range and source ETag".into(),
            ));
        }
        let response = self
            .client
            .upload_part_copy()
            .bucket(&self.bucket)
            .key(self.key(destination)?)
            .upload_id(upload)
            .part_number(part)
            .copy_source(self.copy_source(source)?)
            .copy_source_if_match(etag)
            .copy_source_range(format!("bytes={}-{}", range.start(), range.end()))
            .send()
            .await
            .map_err(|e| {
                backend(
                    "UploadPartCopy",
                    e.as_service_error().and_then(|e| e.code()),
                    &e,
                )
            })?;
        let etag = response
            .copy_part_result()
            .and_then(|r| r.e_tag())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::InvalidResponse("UploadPartCopy missing ETag".into()))?;
        Ok(CompletedPart::builder()
            .part_number(part)
            .e_tag(etag)
            .build())
    }

    /// Keep the upload/generation pending on any error, including embedded errors
    /// in HTTP 200 responses (decoded by the SDK). Caller decides recovery/abort.
    pub async fn complete_upload(
        &self,
        key: &str,
        upload: &str,
        parts: Vec<CompletedPart>,
    ) -> Result<String> {
        if parts.is_empty() {
            return Err(Error::Config("multipart completion needs parts".into()));
        }
        let mut previous = 0;
        for part in &parts {
            let number = part
                .part_number()
                .ok_or_else(|| Error::Config("missing part number".into()))?;
            validate_part(number)?;
            if number <= previous || part.e_tag().is_none_or(str::is_empty) {
                return Err(Error::Config("unordered parts or missing ETag".into()));
            }
            previous = number;
        }
        let response = self
            .client
            .complete_multipart_upload()
            .bucket(&self.bucket)
            .key(self.key(key)?)
            .upload_id(upload)
            .multipart_upload(
                CompletedMultipartUpload::builder()
                    .set_parts(Some(parts))
                    .build(),
            )
            .send()
            .await
            .map_err(|e| {
                backend(
                    "CompleteMultipartUpload",
                    e.as_service_error().and_then(|e| e.code()),
                    &e,
                )
            })?;
        if response.e_tag().is_none_or(str::is_empty) {
            return Err(Error::InvalidResponse("completion missing ETag".into()));
        }
        Ok(response.e_tag().expect("validated ETag").into())
    }

    pub async fn abort_upload(&self, key: &str, upload: &str) -> Result<()> {
        self.client
            .abort_multipart_upload()
            .bucket(&self.bucket)
            .key(self.key(key)?)
            .upload_id(upload)
            .send()
            .await
            .map_err(|e| {
                backend(
                    "AbortMultipartUpload",
                    e.as_service_error().and_then(|e| e.code()),
                    &e,
                )
            })?;
        Ok(())
    }

    pub async fn copy(&self, source: &str, destination: &str, etag: &str) -> Result<String> {
        if etag.is_empty() {
            return Err(Error::Config("copy requires source ETag".into()));
        }
        let mut storage = None;
        if let Some(threshold) = self.copy_threshold {
            let (head, source_storage) = self.head_details(source).await?;
            if head.etag != etag {
                return Err(Error::Fs(libc::ESTALE));
            }
            storage = Some(source_storage.unwrap_or_else(|| {
                if self.storage_class == "STANDARD_IA" && head.size < self.cold_min_size {
                    "STANDARD".into()
                } else {
                    self.storage_class.clone()
                }
            }));
            if head.size > threshold {
                return self
                    .copy_multipart(source, destination, &head, threshold, storage)
                    .await;
            }
        }
        let response = self
            .client
            .copy_object()
            .bucket(&self.bucket)
            .key(self.key(destination)?)
            .copy_source(self.copy_source(source)?)
            .copy_source_if_match(etag)
            .set_storage_class(storage.map(|s| aws_sdk_s3::types::StorageClass::from(s.as_str())))
            .send()
            .await
            .map_err(|e| {
                backend(
                    "CopyObject",
                    e.as_service_error().and_then(|e| e.code()),
                    &e,
                )
            })?;
        if response
            .copy_object_result()
            .and_then(|r| r.e_tag())
            .is_none_or(str::is_empty)
        {
            return Err(Error::InvalidResponse("copy missing ETag".into()));
        }
        Ok(response
            .copy_object_result()
            .and_then(|r| r.e_tag())
            .expect("validated copy ETag")
            .into())
    }

    async fn copy_multipart(
        &self,
        source: &str,
        destination: &str,
        head: &ObjectHead,
        threshold: u64,
        storage: Option<String>,
    ) -> Result<String> {
        const MAX_PART: u64 = 5 * 1024 * 1024 * 1024;
        let part_size = threshold
            .max(5 * 1024 * 1024)
            .max(head.size.div_ceil(10000));
        if part_size > MAX_PART {
            return Err(Error::Fs(libc::EFBIG));
        }
        let permit = tokio::time::timeout(
            self.read_timeout,
            self.cleanup_slots.clone().acquire_owned(),
        )
        .await
        .map_err(|_| Error::Timeout("multipart copy admission"))?
        .map_err(|_| Error::Cancelled)?;
        let upload = self
            .begin_upload_storage(
                destination,
                head.metadata.clone(),
                storage,
                &head.properties,
            )
            .await?;
        let mut guard = CopyUpload {
            store: self.clone(),
            key: destination.into(),
            upload: upload.clone(),
            permit: Some(permit),
            tasks: Some(tokio::task::JoinSet::new()),
            armed: true,
        };
        let tasks = guard.tasks.as_mut().expect("owned copy tasks");
        let result = async {
            let mut parts = std::collections::BTreeMap::new();
            let (mut offset, mut number) = (0, 0);
            while offset < head.size || !tasks.is_empty() {
                while offset < head.size && tasks.len() < self.copy_parallelism {
                    let start = offset;
                    let end = (offset + part_size).min(head.size) - 1;
                    offset = end + 1;
                    number += 1;
                    let (store, source, destination, upload, etag) = (
                        self.clone(),
                        source.to_owned(),
                        destination.to_owned(),
                        upload.clone(),
                        head.etag.clone(),
                    );
                    let part = number;
                    tasks.spawn(async move {
                        store
                            .copy_part(&destination, &upload, part, &source, start..=end, &etag)
                            .await
                    });
                }
                match tasks.join_next().await.expect("copy jobs") {
                    Ok(Ok(part)) => {
                        parts.insert(
                            part.part_number().ok_or_else(|| {
                                Error::InvalidResponse("missing copied part number".into())
                            })?,
                            part,
                        );
                    }
                    other => {
                        return Err(match other {
                            Ok(Err(error)) => error,
                            _ => Error::Cancelled,
                        });
                    }
                }
            }
            self.complete_upload(destination, &upload, parts.into_values().collect())
                .await
        }
        .await;
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
        if result.is_err() {
            let _ = self.abort_upload(destination, &upload).await;
        }
        guard.armed = false;
        result
    }

    pub async fn replace_metadata(
        &self,
        key: &str,
        etag: &str,
        metadata: HashMap<String, String>,
    ) -> Result<ObjectHead> {
        let (mut head, storage) = self.head_details(key).await?;
        if head.etag != etag {
            return Err(Error::Fs(libc::ESTALE));
        }
        head.metadata = metadata.clone();
        let threshold = self.copy_threshold.unwrap_or(128 * 1024 * 1024);
        let committed = if head.size > threshold {
            self.copy_multipart(key, key, &head, threshold, storage)
                .await?
        } else {
            let response = self
                .client
                .copy_object()
                .bucket(&self.bucket)
                .key(self.key(key)?)
                .copy_source(self.copy_source(key)?)
                .copy_source_if_match(etag)
                .metadata_directive(aws_sdk_s3::types::MetadataDirective::Replace)
                .set_content_type(head.properties.content_type.clone())
                .set_content_encoding(head.properties.content_encoding.clone())
                .set_content_disposition(head.properties.content_disposition.clone())
                .set_content_language(head.properties.content_language.clone())
                .set_cache_control(head.properties.cache_control.clone())
                .set_expires(head.properties.expires)
                .set_website_redirect_location(head.properties.website_redirect_location.clone())
                .set_metadata(Some(metadata))
                .set_storage_class(
                    storage.map(|s| aws_sdk_s3::types::StorageClass::from(s.as_str())),
                )
                .send()
                .await
                .map_err(|e| {
                    backend(
                        "replace metadata",
                        e.as_service_error().and_then(|e| e.code()),
                        &e,
                    )
                })?;
            response
                .copy_object_result()
                .and_then(|r| r.e_tag())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| Error::InvalidResponse("metadata copy missing ETag".into()))?
                .to_owned()
        };
        let updated = self.head(key).await?;
        if updated.etag != committed || updated.size != head.size {
            return Err(Error::Fs(libc::ESTALE));
        }
        Ok(updated)
    }

    /// Call after requests/background work have stopped, before runtime teardown.
    pub async fn drain_cleanup(&self) -> Result<()> {
        let _idle = tokio::time::timeout(
            self.read_timeout,
            self.cleanup_slots.acquire_many(self.cleanup_limit),
        )
        .await
        .map_err(|_| Error::Timeout("multipart copy cleanup"))?
        .map_err(|_| Error::Cancelled)?;
        Ok(())
    }

    pub async fn delete(&self, key: &str) -> Result<()> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(self.key(key)?)
            .send()
            .await
            .map_err(|e| backend("DELETE", e.as_service_error().and_then(|e| e.code()), &e))?;
        Ok(())
    }
}

struct CopyUpload {
    store: S3,
    key: String,
    upload: String,
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
    tasks: Option<tokio::task::JoinSet<Result<CompletedPart>>>,
    armed: bool,
}
impl Drop for CopyUpload {
    fn drop(&mut self) {
        if self.armed
            && let Ok(runtime) = tokio::runtime::Handle::try_current()
        {
            let store = self.store.clone();
            let key = self.key.clone();
            let upload = self.upload.clone();
            let permit = self.permit.take();
            let mut tasks = self.tasks.take().expect("owned copy tasks");
            tasks.abort_all();
            runtime.spawn(async move {
                let _permit = permit;
                while tasks.join_next().await.is_some() {}
                let _ = store.abort_upload(&key, &upload).await;
            });
        }
    }
}

fn validate_part(part: i32) -> Result<()> {
    if !(1..=10000).contains(&part) {
        return Err(Error::Config("part number outside 1..=10000".into()));
    }
    Ok(())
}
