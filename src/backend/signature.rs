//! SDK authentication adapters; canonicalization and crypto remain crate-owned.
use aws_smithy_runtime_api::{
    box_error::BoxError,
    client::{
        auth::{
            AuthScheme, AuthSchemeEndpointConfig, AuthSchemeId, AuthSchemeOption,
            AuthSchemeOptionsFuture, Sign,
        },
        identity::{Identity, SharedIdentityResolver},
        orchestrator::HttpRequest,
        runtime_components::{GetIdentityResolver, RuntimeComponents},
    },
};
use aws_smithy_types::config_bag::ConfigBag;
const SIGV4: AuthSchemeId = AuthSchemeId::new("sigv4");
#[derive(Debug)]
pub struct V2Scheme {
    signer: V2Signer,
}
impl V2Scheme {
    pub fn new(bucket: String) -> Self {
        Self {
            signer: V2Signer { bucket },
        }
    }
}
impl AuthScheme for V2Scheme {
    fn scheme_id(&self) -> AuthSchemeId {
        SIGV4
    }
    fn identity_resolver(
        &self,
        resolvers: &dyn GetIdentityResolver,
    ) -> Option<SharedIdentityResolver> {
        resolvers.identity_resolver(SIGV4)
    }
    fn signer(&self) -> &dyn Sign {
        &self.signer
    }
}
#[derive(Debug)]
struct V2Signer {
    bucket: String,
}
impl Sign for V2Signer {
    fn sign_http_request(
        &self,
        r: &mut HttpRequest,
        identity: &Identity,
        _: AuthSchemeEndpointConfig<'_>,
        runtime: &RuntimeComponents,
        _: &ConfigBag,
    ) -> std::result::Result<(), BoxError> {
        let credentials = identity
            .data::<aws_sdk_s3::config::Credentials>()
            .ok_or("SigV2 credentials type mismatch")?;
        if r.headers().get("x-amz-date").is_none() && r.headers().get("date").is_none() {
            let now = runtime
                .time_source()
                .map_or_else(std::time::SystemTime::now, |clock| clock.now());
            r.headers_mut().try_insert(
                "x-amz-date",
                aws_smithy_types::DateTime::from(now)
                    .fmt(aws_smithy_types::date_time::Format::HttpDate)?,
            )?;
        }
        if let Some(token) = credentials.session_token() {
            r.headers_mut()
                .try_insert("x-amz-security-token", token.to_owned())?;
        }
        r.headers_mut().remove("authorization");
        // The reference replaces its SDK user-agent handler. Retain the normal
        // User-Agent but omit this duplicate telemetry header from legacy auth.
        r.headers_mut().remove("x-amz-user-agent");
        let uri = reqwest::Url::parse(r.uri())?;
        // Go's subresources are already encoded by the SDK URI serializer.
        // Preserve its encoded uploadId rules, not a second URL encoder.
        const SUBRESOURCES: &[&str] = &[
            "acl",
            "delete",
            "lifecycle",
            "location",
            "logging",
            "notification",
            "partNumber",
            "policy",
            "requestPayment",
            "torrent",
            "uploadId",
            "uploads",
            "versionId",
            "versioning",
            "versions",
            "website",
        ];
        let mut query: Vec<(String, String)> = uri
            .query()
            .into_iter()
            .flat_map(|s| s.split('&'))
            .map(|p| p.split_once('=').unwrap_or((p, "")))
            .filter(|(name, _)| SUBRESOURCES.contains(name))
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect();
        query.sort();
        let headers: Vec<_> = r.headers().iter().collect();
        let virtual_bucket = uri
            .host_str()
            .filter(|host| host.starts_with(&format!("{}.", self.bucket)))
            .map(|_| self.bucket.as_str());
        let canonical = s3s_sigv2::create_string_to_sign(
            s3s_sigv2::Mode::HeaderAuth,
            r.method(),
            uri.path(),
            Some(&query),
            &headers,
            virtual_bucket,
        );
        let signature = s3s_sigv2::calculate_signature(credentials.secret_access_key(), &canonical);
        r.headers_mut().try_insert(
            "authorization",
            format!("AWS {}:{signature}", credentials.access_key_id()),
        )?;
        Ok(())
    }
}
#[derive(Debug)]
pub struct AnonymousResolver;
impl aws_sdk_s3::config::auth::ResolveAuthScheme for AnonymousResolver {
    fn resolve_auth_scheme<'a>(
        &'a self,
        _: &'a aws_sdk_s3::config::auth::Params,
        _: &'a ConfigBag,
        _: &'a RuntimeComponents,
    ) -> AuthSchemeOptionsFuture<'a> {
        AuthSchemeOptionsFuture::ready(Ok(vec![AuthSchemeOption::from(AuthSchemeId::new(
            "noAuth",
        ))]))
    }
}
