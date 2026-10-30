//! Narrow SDK transport adapter for explicit insecure-TLS mode. Reqwest owns
//! HTTP/TLS and streaming bodies; ordinary verified traffic uses SDK transport.
use crate::error::{Error, Result};
use aws_smithy_runtime_api::client::{
    http::{
        HttpClient, HttpConnector, HttpConnectorFuture, HttpConnectorSettings, SharedHttpConnector,
    },
    orchestrator::{HttpRequest, HttpResponse},
    result::ConnectorError,
    runtime_components::RuntimeComponents,
};
use aws_smithy_types::body::SdkBody;
#[derive(Clone, Debug)]
pub struct InsecureTransport(reqwest::Client);
impl InsecureTransport {
    pub fn new(timeout: std::time::Duration) -> Result<Self> {
        reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .timeout(timeout)
            .connect_timeout(timeout)
            .read_timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .gzip(false)
            .build()
            .map(Self)
            .map_err(|_| Error::Config("insecure S3 transport initialization failed".into()))
    }
}
impl HttpClient for InsecureTransport {
    fn http_connector(
        &self,
        _: &HttpConnectorSettings,
        _: &RuntimeComponents,
    ) -> SharedHttpConnector {
        SharedHttpConnector::new(self.clone())
    }
}
impl HttpConnector for InsecureTransport {
    fn call(&self, request: HttpRequest) -> HttpConnectorFuture {
        let client = self.0.clone();
        HttpConnectorFuture::new(async move {
            let request = request
                .try_into_http1x()
                .map_err(|e| ConnectorError::user(Box::new(e)))?
                .map(reqwest::Body::wrap);
            let request = reqwest::Request::try_from(request).map_err(connection_error)?;
            let response: http::Response<reqwest::Body> = client
                .execute(request)
                .await
                .map_err(connection_error)?
                .into();
            HttpResponse::try_from(response.map(SdkBody::from_body_1_x))
                .map_err(|e| ConnectorError::other(Box::new(e), None))
        })
    }
}
fn connection_error(error: reqwest::Error) -> ConnectorError {
    let error = error.without_url();
    if error.is_timeout() {
        ConnectorError::timeout(Box::new(error))
    } else if error.is_connect() {
        ConnectorError::io(Box::new(error)).never_connected()
    } else if error.is_builder() || error.is_request() {
        ConnectorError::user(Box::new(error))
    } else {
        ConnectorError::other(Box::new(error), None)
    }
}
