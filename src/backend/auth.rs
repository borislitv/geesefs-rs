//! Public SDK authentication-scheme adapter for the reference metadata-token API.
//! There is no cryptographic signer here: IAM transmits a configured token header.
use crate::{
    config::{Config, IamFlavor},
    error::{Error, Result},
};
use aws_smithy_runtime_api::{
    box_error::BoxError,
    client::{
        auth::{
            AuthScheme, AuthSchemeEndpointConfig, AuthSchemeId, AuthSchemeOption,
            AuthSchemeOptionsFuture, Sign,
        },
        identity::{Identity, IdentityFuture, ResolveIdentity, SharedIdentityResolver},
        orchestrator::HttpRequest,
        runtime_components::{GetIdentityResolver, RuntimeComponents},
    },
};
use aws_smithy_types::config_bag::ConfigBag;
use std::{
    sync::Arc,
    time::{Duration, SystemTime},
};

// Replace the SDK's selected scheme using its documented override API. Keeping
// its endpoint scheme ID preserves SDK endpoint/addressing resolution; only the
// identity resolver and header-only signer are replaced for IAM-configured clients.
const SCHEME: AuthSchemeId = AuthSchemeId::new("sigv4");
#[derive(Clone)]
struct Token(String);
impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("IAM token (REDACTED)")
    }
}
struct Cached {
    token: Token,
    expires: SystemTime,
    refresh: SystemTime,
}
pub struct IamProvider {
    client: reqwest::Client,
    url: String,
    flavor: IamFlavor,
    cache: tokio::sync::Mutex<Option<Cached>>,
}
impl std::fmt::Debug for IamProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("IamProvider(REDACTED)")
    }
}
impl IamProvider {
    pub fn new(config: &Config) -> Result<Arc<Self>> {
        let url=config.iam_url.clone().unwrap_or_else(|| match config.iam_flavor { IamFlavor::Gcp=>"http://169.254.169.254/computeMetadata/v1/instance/service-accounts/default/token",IamFlavor::Imds=>"http://169.254.169.254/latest/meta-data/iam/security-credentials/default" }.into());
        Ok(Arc::new(Self {
            client: reqwest::Client::builder()
                .no_proxy()
                .timeout(config.http_timeout)
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| Error::Config("IAM HTTP client initialization failed".into()))?,
            url,
            flavor: config.iam_flavor,
            cache: tokio::sync::Mutex::new(None),
        }))
    }
    async fn identity(&self, runtime: &RuntimeComponents) -> Result<Identity> {
        let mut cache = self.cache.lock().await;
        let now = runtime
            .time_source()
            .map_or_else(SystemTime::now, |time| time.now());
        if let Some(cached) = cache.as_ref().filter(|c| now < c.refresh) {
            return Ok(Identity::new(cached.token.clone(), Some(cached.expires)));
        }
        let mut request = self.client.get(&self.url);
        if self.flavor == IamFlavor::Gcp {
            request = request.header("Metadata-Flavor", "Google");
        }
        let mut response = request
            .send()
            .await
            .map_err(|_| Error::InvalidResponse("IAM metadata request failed".into()))?;
        if !response.status().is_success() {
            return Err(Error::InvalidResponse(format!(
                "IAM metadata HTTP {}",
                response.status().as_u16()
            )));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| Error::InvalidResponse("IAM metadata body failed".into()))?
        {
            if bytes.len() + chunk.len() > 65536 {
                return Err(Error::InvalidResponse("IAM metadata exceeds 64KiB".into()));
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|_| Error::InvalidResponse("invalid IAM metadata JSON".into()))?;
        let (token, expires) = match self.flavor {
            IamFlavor::Gcp => {
                let token = value.get("access_token").and_then(|v| v.as_str());
                let ttl = value
                    .get("expires_in")
                    .and_then(|v| v.as_u64())
                    .filter(|n| *n > 0)
                    .and_then(|n| now.checked_add(Duration::from_secs(n)));
                (token, ttl)
            }
            IamFlavor::Imds => {
                if value.get("Code").and_then(|v| v.as_str()) != Some("Success") {
                    return Err(Error::InvalidResponse(
                        "IAM metadata did not report Success".into(),
                    ));
                }
                let expiry = value
                    .get("Expiration")
                    .and_then(|v| v.as_str())
                    .and_then(|s| {
                        aws_smithy_types::DateTime::from_str(
                            s,
                            aws_smithy_types::date_time::Format::DateTime,
                        )
                        .ok()
                    })
                    .and_then(|t| SystemTime::try_from(t).ok());
                (value.get("Token").and_then(|v| v.as_str()), expiry)
            }
        };
        let token = token
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::InvalidResponse("IAM metadata has no token".into()))?;
        reqwest::header::HeaderValue::from_str(token)
            .map_err(|_| Error::InvalidResponse("invalid IAM token header".into()))?;
        let expires = expires
            .filter(|t| {
                *t > runtime
                    .time_source()
                    .map_or_else(SystemTime::now, |time| time.now())
            })
            .ok_or_else(|| Error::InvalidResponse("IAM metadata has no future expiry".into()))?;
        let ttl = expires
            .duration_since(now)
            .map_err(|_| Error::InvalidResponse("expired IAM token".into()))?;
        let margin = if ttl > Duration::from_secs(300) {
            Duration::from_secs(300)
        } else if ttl > Duration::from_secs(30) {
            Duration::from_secs(30)
        } else {
            ttl / 2
        };
        let token = Token(token.to_owned());
        *cache = Some(Cached {
            token: token.clone(),
            expires,
            refresh: expires - margin,
        });
        Ok(Identity::new(token, Some(expires)))
    }
}
impl ResolveIdentity for IamProvider {
    fn resolve_identity<'a>(
        &'a self,
        runtime: &'a RuntimeComponents,
        _: &'a ConfigBag,
    ) -> IdentityFuture<'a> {
        IdentityFuture::new(async move {
            self.identity(runtime)
                .await
                .map_err(|e| Box::new(e) as BoxError)
        })
    }
}
#[derive(Debug)]
pub struct IamScheme {
    resolver: SharedIdentityResolver,
    signer: TokenHeader,
}
impl IamScheme {
    pub fn new(provider: Arc<IamProvider>, header: String) -> Self {
        Self {
            resolver: SharedIdentityResolver::new(Provider(provider)),
            signer: TokenHeader(header),
        }
    }
}
#[derive(Debug)]
struct Provider(Arc<IamProvider>);
impl ResolveIdentity for Provider {
    fn resolve_identity<'a>(
        &'a self,
        r: &'a RuntimeComponents,
        c: &'a ConfigBag,
    ) -> IdentityFuture<'a> {
        self.0.resolve_identity(r, c)
    }
}
#[derive(Debug)]
struct TokenHeader(String);
impl Sign for TokenHeader {
    fn sign_http_request(
        &self,
        request: &mut HttpRequest,
        identity: &Identity,
        _: AuthSchemeEndpointConfig<'_>,
        _: &RuntimeComponents,
        _: &ConfigBag,
    ) -> std::result::Result<(), BoxError> {
        let token = identity
            .data::<Token>()
            .ok_or("IAM identity type mismatch")?;
        request
            .headers_mut()
            .try_insert(self.0.clone(), token.0.clone())?;
        Ok(())
    }
}
impl AuthScheme for IamScheme {
    fn scheme_id(&self) -> AuthSchemeId {
        SCHEME
    }
    fn identity_resolver(&self, _: &dyn GetIdentityResolver) -> Option<SharedIdentityResolver> {
        Some(self.resolver.clone())
    }
    fn signer(&self) -> &dyn Sign {
        &self.signer
    }
}
#[derive(Debug)]
pub struct IamResolver;
impl aws_sdk_s3::config::auth::ResolveAuthScheme for IamResolver {
    fn resolve_auth_scheme<'a>(
        &'a self,
        _: &'a aws_sdk_s3::config::auth::Params,
        _: &'a ConfigBag,
        _: &'a RuntimeComponents,
    ) -> AuthSchemeOptionsFuture<'a> {
        AuthSchemeOptionsFuture::ready(Ok(vec![AuthSchemeOption::from(SCHEME)]))
    }
}
