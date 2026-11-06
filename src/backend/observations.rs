//! Supported SDK hooks count transport attempts, not presigning or logical replies.
use aws_runtime::auth::{HttpSignatureType, SigV4OperationSigningConfig};
use aws_smithy_runtime_api::{
    box_error::BoxError,
    client::{
        interceptors::{
            Intercept,
            context::{
                BeforeSerializationInterceptorContextRef, BeforeTransmitInterceptorContextRef,
            },
        },
        orchestrator::Metadata,
        runtime_components::RuntimeComponents,
    },
};
use aws_smithy_types::config_bag::{ConfigBag, Storable, StoreReplace};

#[derive(Debug)]
pub(super) struct Observations(pub(crate) crate::metrics::Diagnostics);
impl Observations {
    pub(super) const NAME: &'static str = "GeeseFSObservations";
}
#[derive(Debug)]
struct Transmits(u64);
impl Storable for Transmits {
    type Storer = StoreReplace<Self>;
}

impl Intercept for Observations {
    fn name(&self) -> &'static str {
        Self::NAME
    }
    fn read_before_execution(
        &self,
        _: &BeforeSerializationInterceptorContextRef<'_>,
        cfg: &mut ConfigBag,
    ) -> Result<(), BoxError> {
        cfg.interceptor_state().store_put(Transmits(0));
        Ok(())
    }
    fn read_before_transmit(
        &self,
        _: &BeforeTransmitInterceptorContextRef<'_>,
        _: &RuntimeComponents,
        cfg: &mut ConfigBag,
    ) -> Result<(), BoxError> {
        // Presigning runs this hook too, then stops before calling the transport.
        // Use the public signing configuration, not the SDK's private marker or
        // credential-bearing URL/header inspection. Our sent SDK operations use
        // header signing; query signing is only used to construct extension URLs.
        if cfg
            .load::<SigV4OperationSigningConfig>()
            .is_some_and(|config| {
                config.signing_options.signature_type == HttpSignatureType::HttpRequestQueryParams
            })
        {
            return Ok(());
        }
        let operation = match cfg.load::<Metadata>().map(|metadata| metadata.name()) {
            Some("GetObject") => "get",
            Some("HeadObject" | "HeadBucket") => "head",
            Some("ListObjects" | "ListObjectsV2") => "list",
            Some(
                "PutObject"
                | "CopyObject"
                | "CreateMultipartUpload"
                | "UploadPart"
                | "UploadPartCopy"
                | "CompleteMultipartUpload"
                | "AbortMultipartUpload"
                | "DeleteObject",
            ) => "write",
            _ => "other",
        };
        let previous = cfg
            .load::<Transmits>()
            .expect("before-execution observer initializes attempts")
            .0;
        self.0.attempt(operation, "sdk", previous != 0);
        cfg.interceptor_state().store_put(Transmits(previous + 1));
        Ok(())
    }
}
