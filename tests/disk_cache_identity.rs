#![forbid(unsafe_code)]

use aws_sdk_s3::{
    Client,
    config::{BehaviorVersion, Credentials, Region},
};
use clap::Parser;
use geesefs_rs::backend::{
    Store,
    cache_scope::{CacheBypass, CacheScope},
    s3::S3,
};
use geesefs_rs::config::Config;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

#[path = "support/disk.rs"]
mod support;
pub use support::{budget, disk_cache, error};

fn options(extra: &[&str]) -> Config {
    let mut args = vec![
        "geesefs-rs",
        "--no-detect",
        "--http-timeout=2s",
        "--sdk-max-retries=0",
    ];
    args.extend_from_slice(extra);
    args.extend(["bucket:prefix", "/tmp/unused-mount"]);
    Config::try_parse_from(args).unwrap()
}

async fn namespace(store: S3) -> [u8; 32] {
    store
        .initialize()
        .await
        .unwrap()
        .persistent_cache_scope()
        .unwrap()
        .namespace()
        .expect("expected eligible namespace")
}

fn backend(endpoint: &str, bucket: &str, prefix: &str) -> S3 {
    backend_in_region(endpoint, bucket, prefix, "us-east-1")
}

fn backend_in_region(endpoint: &str, bucket: &str, prefix: &str, region: &str) -> S3 {
    let config = aws_sdk_s3::Config::builder()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new(region.to_owned()))
        .endpoint_url(endpoint)
        .force_path_style(true)
        .credentials_provider(credentials("access-one"))
        .build();
    S3::new(Client::from_conf(config), bucket.into(), prefix.into()).unwrap()
}

fn credentials(access: &str) -> Credentials {
    Credentials::new(
        access,
        "fixture-secret-never-persist",
        None,
        None,
        "fixture",
    )
}

#[tokio::test]
async fn known_static_identity_survives_reconstruction_and_separates_backends() {
    let mut namespaces = Vec::new();
    for (endpoint, bucket, prefix, access) in [
        ("http://127.0.0.1:9", "bucket", "prefix", "access-one"),
        ("http://127.0.0.1:9", "bucket", "prefix", "access-one"),
        ("http://127.0.0.1:10", "bucket", "prefix", "access-one"),
        ("http://127.0.0.1:9", "other", "prefix", "access-one"),
        ("http://127.0.0.1:9", "bucket", "other", "access-one"),
        ("http://127.0.0.1:9", "bucket", "prefix", "access-two"),
    ] {
        let store = backend(endpoint, bucket, prefix)
            .with_static_credentials(credentials(access))
            .initialize()
            .await
            .unwrap();
        let scope = Store::persistent_cache_scope(&store).unwrap();
        namespaces.push(
            scope
                .namespace()
                .expect("known static credentials must support persistence"),
        );
        let debug = format!("{scope:?}");
        assert!(!debug.contains(access));
        assert!(!debug.contains("fixture-secret"));
    }
    assert_eq!(namespaces[0], namespaces[1]);
    for i in 2..namespaces.len() {
        assert_ne!(namespaces[0], namespaces[i], "context {i} must not alias");
    }
}

#[tokio::test]
async fn opaque_sdk_provider_is_not_anonymous_even_when_it_returns_static_values() {
    let store = backend("http://127.0.0.1:9", "bucket", "prefix")
        .initialize()
        .await
        .unwrap();
    assert_eq!(
        store.persistent_cache_scope().unwrap(),
        CacheScope::Bypass(CacheBypass::OpaqueCredentials)
    );
}

#[tokio::test]
async fn scope_requires_initialization_and_options_invalidate_it() {
    let store = backend("http://127.0.0.1:9", "bucket", "prefix")
        .with_static_credentials(credentials("access-one"));
    assert!(store.persistent_cache_scope().is_err());
    let store = store.initialize().await.unwrap();
    assert!(
        store
            .persistent_cache_scope()
            .unwrap()
            .namespace()
            .is_some()
    );
    let store = store.with_options(&options(&[])).unwrap();
    assert!(store.persistent_cache_scope().is_err());
    assert!(
        store
            .initialize()
            .await
            .unwrap()
            .persistent_cache_scope()
            .unwrap()
            .namespace()
            .is_some()
    );
}

#[tokio::test]
async fn retained_custom_auth_on_reconfiguration_cannot_claim_a_signed_scope() {
    let store = backend("http://127.0.0.1:9", "bucket", "prefix")
        .with_static_credentials(credentials("access-one"))
        .with_options(&options(&["--signature-version=anonymous"]))
        .unwrap()
        .initialize()
        .await
        .unwrap();
    assert!(
        store
            .persistent_cache_scope()
            .unwrap()
            .namespace()
            .is_some()
    );
    let store = store
        .with_options(&options(&["--signature-version=4"]))
        .unwrap()
        .initialize()
        .await
        .unwrap();
    assert_eq!(
        store.persistent_cache_scope().unwrap(),
        CacheScope::Bypass(CacheBypass::OpaqueAuthentication)
    );
}

#[tokio::test]
async fn encryption_and_requester_contexts_do_not_alias() {
    use base64::Engine;
    let key_one = base64::engine::general_purpose::STANDARD.encode([1u8; 32]);
    let key_two = base64::engine::general_purpose::STANDARD.encode([2u8; 32]);
    let mut scopes = Vec::new();
    for extra in [
        vec![],
        vec!["--sse"],
        vec!["--sse-kms=key-one"],
        vec!["--sse-kms=key-two"],
        vec!["--sse-c", &key_one],
        vec!["--sse-c", &key_two],
        vec!["--requester-pays"],
    ] {
        scopes.push(
            namespace(
                backend("http://127.0.0.1:9", "bucket", "prefix")
                    .with_static_credentials(credentials("access-one"))
                    .with_options(&options(&extra))
                    .unwrap(),
            )
            .await,
        );
    }
    for i in 0..scopes.len() {
        for j in 0..i {
            assert_ne!(scopes[i], scopes[j], "encryption/access contexts {i}/{j}");
        }
    }
    let bare = namespace(
        backend("http://127.0.0.1:9", "bucket", "prefix")
            .with_static_credentials(credentials("access-one")),
    )
    .await;
    assert_eq!(
        scopes[0], bare,
        "default options must retain the same context"
    );
    // The configured MD5 is a wire selector, not our identity boundary.
    let mut changed_md5 = options(&["--sse-c", &key_one]);
    changed_md5.sse_c.as_mut().unwrap().digest = "not-an-identity".into();
    assert_eq!(
        scopes[4],
        namespace(
            backend("http://127.0.0.1:9", "bucket", "prefix")
                .with_static_credentials(credentials("access-one"))
                .with_options(&changed_md5)
                .unwrap()
        )
        .await
    );
}

#[derive(Clone, Debug)]
struct CountingProvider(Arc<AtomicUsize>);
impl aws_smithy_runtime_api::client::identity::ResolveIdentity for CountingProvider {
    fn resolve_identity<'a>(
        &'a self,
        _: &'a aws_smithy_runtime_api::client::runtime_components::RuntimeComponents,
        _: &'a aws_smithy_types::config_bag::ConfigBag,
    ) -> aws_smithy_runtime_api::client::identity::IdentityFuture<'a> {
        self.0.fetch_add(1, Ordering::SeqCst);
        aws_smithy_runtime_api::client::identity::IdentityFuture::ready(Ok(credentials(
            "opaque-access",
        )
        .into()))
    }
}

impl aws_smithy_runtime_api::client::auth::AuthScheme for CountingProvider {
    fn scheme_id(&self) -> aws_smithy_runtime_api::client::auth::AuthSchemeId {
        aws_smithy_runtime_api::client::auth::AuthSchemeId::new("sigv4")
    }
    fn identity_resolver(
        &self,
        _: &dyn aws_smithy_runtime_api::client::runtime_components::GetIdentityResolver,
    ) -> Option<aws_smithy_runtime_api::client::identity::SharedIdentityResolver> {
        Some(aws_smithy_runtime_api::client::identity::SharedIdentityResolver::new(self.clone()))
    }
    fn signer(&self) -> &dyn aws_smithy_runtime_api::client::auth::Sign {
        self
    }
}
impl aws_smithy_runtime_api::client::auth::Sign for CountingProvider {
    fn sign_http_request(
        &self,
        _: &mut aws_smithy_runtime_api::client::orchestrator::HttpRequest,
        _: &aws_smithy_runtime_api::client::identity::Identity,
        _: aws_smithy_runtime_api::client::auth::AuthSchemeEndpointConfig<'_>,
        _: &aws_smithy_runtime_api::client::runtime_components::RuntimeComponents,
        _: &aws_smithy_types::config_bag::ConfigBag,
    ) -> Result<(), aws_smithy_runtime_api::box_error::BoxError> {
        Ok(())
    }
}

#[tokio::test]
async fn anonymous_and_opaque_scopes_never_resolve_credentials_or_send_requests() {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", server.server_addr());
    let calls = Arc::new(AtomicUsize::new(0));
    let sdk = aws_sdk_s3::Config::builder()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new("us-east-1"))
        .endpoint_url(&endpoint)
        .push_auth_scheme(CountingProvider(calls.clone()))
        .build();
    let client = Client::from_conf(sdk);
    client
        .head_object()
        .bucket("bucket")
        .key("calibration")
        .presigned(
            aws_sdk_s3::presigning::PresigningConfig::expires_in(Duration::from_secs(60)).unwrap(),
        )
        .await
        .unwrap();
    assert!(
        calls.swap(0, Ordering::SeqCst) > 0,
        "calibration must exercise identity resolution"
    );
    let opaque = S3::new(client, "bucket".into(), "prefix".into()).unwrap();
    let anonymous = options(&["--signature-version=anonymous"]);
    let first = namespace(opaque.clone().with_options(&anonymous).unwrap()).await;
    let second = namespace(opaque.clone().with_options(&anonymous).unwrap()).await;
    assert_eq!(first, second);
    let signed = namespace(
        opaque
            .clone()
            .with_static_credentials(credentials("access-one")),
    )
    .await;
    assert_ne!(first, signed);
    let initialized = opaque.initialize().await.unwrap();
    for _ in 0..3 {
        assert_eq!(
            initialized.persistent_cache_scope().unwrap(),
            CacheScope::Bypass(CacheBypass::OpaqueCredentials)
        );
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "cache scope must not invoke credential providers"
    );
    assert!(
        server.try_recv().unwrap().is_none(),
        "cache scope must not issue HTTP requests"
    );
}

#[tokio::test]
async fn fixed_temporary_credentials_are_explicitly_bypassed() {
    for credentials in [
        Credentials::new("access", "secret", Some("token".into()), None, "fixture"),
        Credentials::new(
            "access",
            "secret",
            None,
            Some(std::time::SystemTime::now() + Duration::from_secs(600)),
            "fixture",
        ),
    ] {
        let store = backend("http://127.0.0.1:9", "bucket", "prefix")
            .with_static_credentials(credentials)
            .initialize()
            .await
            .unwrap();
        assert_eq!(
            store.persistent_cache_scope().unwrap(),
            CacheScope::Bypass(CacheBypass::TemporaryCredentials)
        );
    }
}

#[tokio::test]
async fn iam_cache_policy_does_not_contact_metadata_or_s3() {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", server.server_addr());
    let store = backend(&endpoint, "bucket", "prefix")
        .with_static_credentials(credentials("access-one"))
        .with_options(&options(&["--iam", "--iam-url", &endpoint]))
        .unwrap()
        .initialize()
        .await
        .unwrap();
    assert_eq!(
        store.persistent_cache_scope().unwrap(),
        CacheScope::Bypass(CacheBypass::OpaqueAuthentication)
    );
    assert!(server.try_recv().unwrap().is_none());
}

#[tokio::test]
async fn unresolved_endpoint_only_bypasses_optional_cache() {
    let client = Client::from_conf(
        aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .build(),
    );
    // No region/override: SDK endpoint resolution cannot produce a namespace,
    // but no-detect startup must retain its existing lack of remote probes.
    let store = S3::new(client, "bucket".into(), "prefix".into())
        .unwrap()
        .with_static_credentials(credentials("access-one"))
        .initialize()
        .await
        .unwrap();
    assert_eq!(
        store.persistent_cache_scope().unwrap(),
        CacheScope::Bypass(CacheBypass::UnresolvedEndpoint)
    );
}

#[tokio::test]
async fn discovery_uses_effective_region_and_v2_authentication() {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", server.server_addr());
    let task = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for status in [200, 403, 404] {
            let request = server
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .expect("discovery request");
            requests.push((
                request.url().to_owned(),
                request
                    .headers()
                    .iter()
                    .find(|header| header.field.equiv("authorization"))
                    .map(|header| header.value.to_string()),
            ));
            let mut response = tiny_http::Response::empty(status);
            if status == 200 {
                response.add_header(
                    tiny_http::Header::from_bytes("x-amz-bucket-region", "us-west-2").unwrap(),
                );
            }
            request.respond(response).unwrap();
        }
        assert!(server.try_recv().unwrap().is_none());
        requests
    });
    let mut config = options(&["--profile=fixture"]);
    config.no_detect = false;
    config.region = None;
    let discovered = namespace(
        backend(&endpoint, "bucket", "prefix")
            .with_static_credentials(credentials("access-one"))
            .with_options(&config)
            .unwrap(),
    )
    .await;
    let requests = task.join().unwrap();
    assert_eq!(
        requests.len(),
        3,
        "only the normal discovery probes may use HTTP"
    );
    assert!(requests[0].1.is_none());
    assert!(
        requests[1]
            .1
            .as_deref()
            .unwrap()
            .contains("/us-west-2/s3/aws4_request")
    );
    assert!(
        requests[2]
            .1
            .as_deref()
            .unwrap()
            .starts_with("AWS access-one:")
    );
    let explicit = namespace(
        backend_in_region(&endpoint, "bucket", "prefix", "us-west-2")
            .with_static_credentials(credentials("access-one"))
            .with_options(&options(&["--signature-version=2"]))
            .unwrap(),
    )
    .await;
    assert_eq!(
        discovered, explicit,
        "namespace must follow selected region and auth, not initial config"
    );
    let wrong_region = namespace(
        backend(&endpoint, "bucket", "prefix")
            .with_static_credentials(credentials("access-one"))
            .with_options(&options(&["--signature-version=2"]))
            .unwrap(),
    )
    .await;
    assert_ne!(discovered, wrong_region);
}

#[tokio::test]
async fn app_static_environment_is_eligible_without_remote_credential_calls() {
    const CHILD: &str = "GEESEFS_CACHE_IDENTITY_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let config = options(&["--endpoint=http://127.0.0.1:9", "--region=us-east-1"]);
        let actual = namespace(S3::from_config(&config).await.unwrap()).await;
        let expected = namespace(
            backend("http://127.0.0.1:9", "bucket", "prefix")
                .with_static_credentials(credentials("access-one"))
                .with_options(&config)
                .unwrap(),
        )
        .await;
        assert_eq!(actual, expected);
        let across_processes: String = actual.iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(
            across_processes,
            std::env::var("GEESEFS_EXPECTED_SCOPE").unwrap(),
            "namespace must not contain process-local state"
        );
        let work = tempfile::tempdir().unwrap();
        let file = work.path().join("config");
        let marker = work.path().join("must-not-execute");
        for contents in [
            "[profile fixture]\naws_access_key_id = profile-access\naws_secret_access_key = profile-secret\n".to_owned(),
            format!("[profile fixture]\ncredential_process = /usr/bin/touch {}\n", marker.display()),
        ] {
            std::fs::write(&file, contents).unwrap();
            let config = options(&["--endpoint=http://127.0.0.1:9", "--region=us-east-1", "--profile=fixture", "--shared-config", file.to_str().unwrap()]);
            let profile = S3::from_config(&config).await.unwrap().initialize().await.unwrap();
            assert_eq!(profile.persistent_cache_scope().unwrap(), CacheScope::Bypass(CacheBypass::OpaqueCredentials));
            assert!(!marker.exists(), "identity must not resolve profile processes");
        }
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let parent_scope = namespace(
        backend("http://127.0.0.1:9", "bucket", "prefix")
            .with_static_credentials(credentials("access-one"))
            .with_options(&options(&[
                "--endpoint=http://127.0.0.1:9",
                "--region=us-east-1",
            ]))
            .unwrap(),
    )
    .await;
    let expected: String = parent_scope
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
    command
        .arg("--exact")
        .arg("app_static_environment_is_eligible_without_remote_credential_calls")
        .arg("--nocapture")
        .env_clear()
        .env(CHILD, "1")
        .env("GEESEFS_EXPECTED_SCOPE", expected)
        .env("HOME", home.path())
        .env("AWS_ACCESS_KEY_ID", "access-one")
        .env("AWS_SECRET_ACCESS_KEY", "fixture-secret-never-persist")
        .env("AWS_EC2_METADATA_DISABLED", "true")
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(15), command.output())
        .await
        .expect("static environment child deadline")
        .unwrap();
    assert!(
        output.status.success(),
        "child failed: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn backend_namespaces_reject_same_etag_payloads_and_wrong_size_or_ranges() {
    use disk_cache::{Demotion, DiskKey};
    let dir = support::directory();
    let resources = budget::Budget::new(32 * support::MIB).unwrap();
    let output = budget::Budget::new(support::MIB).unwrap();
    let cache = support::open(dir.path(), &resources).await;
    let first = namespace(
        backend("http://127.0.0.1:9", "bucket", "prefix")
            .with_static_credentials(credentials("access-one")),
    )
    .await;
    let other = namespace(
        backend("http://127.0.0.1:9", "bucket", "other")
            .with_static_credentials(credentials("access-one")),
    )
    .await;
    let key = DiskKey::new(&first, b"package.rpm", b"same-opaque-etag", 10, 2, 3).unwrap();
    assert_eq!(
        cache.try_demote(key.clone(), support::block(b"abc")),
        Demotion::Admitted
    );
    cache.drain(support::DEADLINE).await.unwrap();
    assert_eq!(
        cache.read_block(&key, &output).await.unwrap().as_slice(),
        b"abc"
    );
    for (scope, size, offset, len) in [
        (&other, 10, 2, 3),
        (&first, 11, 2, 3),
        (&first, 10, 3, 3),
        (&first, 10, 2, 2),
    ] {
        let wrong = DiskKey::new(
            scope,
            b"package.rpm",
            b"same-opaque-etag",
            size,
            offset,
            len,
        )
        .unwrap();
        assert!(cache.read_block(&wrong, &output).await.is_none());
    }
    cache.close(support::DEADLINE).await.unwrap();
}
