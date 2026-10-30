use crate::config::{Config, CustomerKey};
use aws_smithy_runtime_api::{
    box_error::BoxError,
    client::{
        interceptors::{Intercept, context::BeforeTransmitInterceptorContextMut},
        orchestrator::Metadata,
        runtime_components::RuntimeComponents,
    },
};
use aws_smithy_types::config_bag::ConfigBag;

/// Header adapter for options missing from callers' generic Store interface.
/// Runs before SDK signing; the SDK still owns wire serialization and SigV4.
#[derive(Debug)]
pub struct RequestOptions {
    requester: bool,
    sse: bool,
    kms: Option<String>,
    customer: Option<CustomerKey>,
    storage: String,
    cold_min: u64,
    acl: Option<String>,
    mime: Option<super::content_type::ContentTypes>,
    no_checksum: bool,
}
impl RequestOptions {
    pub fn new(c: &Config) -> crate::error::Result<Self> {
        Ok(Self {
            requester: c.requester_pays,
            sse: c.sse,
            kms: c.sse_kms.clone(),
            customer: c.sse_c.clone(),
            storage: c.storage_class.clone(),
            cold_min: c.cold_min_size,
            acl: c.acl.clone(),
            mime: c
                .use_content_type
                .then(super::content_type::ContentTypes::system)
                .transpose()?,
            no_checksum: c.no_checksum,
        })
    }
}
impl Intercept for RequestOptions {
    fn name(&self) -> &'static str {
        "GeeseFSRequestOptions"
    }
    fn modify_before_signing(
        &self,
        context: &mut BeforeTransmitInterceptorContextMut<'_>,
        _: &RuntimeComponents,
        cfg: &mut ConfigBag,
    ) -> std::result::Result<(), BoxError> {
        let operation = cfg
            .load::<Metadata>()
            .map(|m| m.name().to_owned())
            .unwrap_or_default();
        let r = context.request_mut();
        let preserve_content_type = operation == "CopyObject"
            || operation == "CreateMultipartUpload" && r.headers().get("content-type").is_some();
        if self.requester {
            r.headers_mut()
                .try_insert("x-amz-request-payer", "requester")?;
        }
        let creates = matches!(
            operation.as_str(),
            "PutObject" | "CopyObject" | "CreateMultipartUpload"
        );
        let reads = matches!(operation.as_str(), "HeadObject" | "GetObject");
        let parts = matches!(
            operation.as_str(),
            "UploadPart" | "UploadPartCopy" | "CompleteMultipartUpload"
        );
        if creates {
            if self.kms.is_some() || self.sse {
                r.headers_mut().try_insert(
                    "x-amz-server-side-encryption",
                    if self.kms.is_some() {
                        "aws:kms"
                    } else {
                        "AES256"
                    },
                )?;
                if let Some(kms) = self.kms.as_ref().filter(|s| !s.is_empty()) {
                    r.headers_mut()
                        .try_insert("x-amz-server-side-encryption-aws-kms-key-id", kms.clone())?;
                }
            }
            let size = r
                .headers()
                .get("content-length")
                .and_then(|s| s.parse::<u64>().ok());
            let storage = if operation == "PutObject"
                && self.storage == "STANDARD_IA"
                && size.is_some_and(|n| n < self.cold_min)
            {
                "STANDARD"
            } else {
                &self.storage
            };
            if r.headers().get("x-amz-storage-class").is_none() {
                r.headers_mut()
                    .try_insert("x-amz-storage-class", storage.to_owned())?;
            }
            if let Some(acl) = &self.acl {
                r.headers_mut().try_insert("x-amz-acl", acl.clone())?;
            }
            if let Some(types) = &self.mime
                && !preserve_content_type
            {
                let url = reqwest::Url::parse(r.uri())?;
                let path = percent_encoding::percent_decode_str(url.path()).decode_utf8_lossy();
                if let Some(mime) = types.guess(path.as_ref()) {
                    r.headers_mut()
                        .try_insert("content-type", mime.to_owned())?;
                }
            }
        }
        if let Some(customer) = self.customer.as_ref().filter(|_| creates || reads || parts) {
            for (name, value) in [
                ("x-amz-server-side-encryption-customer-algorithm", "AES256"),
                (
                    "x-amz-server-side-encryption-customer-key",
                    &customer.encoded,
                ),
                (
                    "x-amz-server-side-encryption-customer-key-md5",
                    &customer.digest,
                ),
            ] {
                r.headers_mut().try_insert(name, value.to_owned())?;
            }
            if matches!(operation.as_str(), "CopyObject" | "UploadPartCopy") {
                for (name, value) in [
                    (
                        "x-amz-copy-source-server-side-encryption-customer-algorithm",
                        "AES256",
                    ),
                    (
                        "x-amz-copy-source-server-side-encryption-customer-key",
                        &customer.encoded,
                    ),
                    (
                        "x-amz-copy-source-server-side-encryption-customer-key-md5",
                        &customer.digest,
                    ),
                ] {
                    r.headers_mut().try_insert(name, value.to_owned())?;
                }
            }
        }
        if self.no_checksum {
            cfg.interceptor_state()
                .store_put(aws_runtime::auth::PayloadSigningOverride::UnsignedPayload);
        }
        Ok(())
    }
}
