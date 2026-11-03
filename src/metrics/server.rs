use std::{convert::Infallible, io, net::SocketAddr, sync::Arc, time::Duration};

use bytes::Bytes;
use http_body_util::Full;
use hyper::{
    Method, Request, Response, StatusCode, body::Incoming, server::conn::http1, service::service_fn,
};
use hyper_util::rt::TokioIo;
use tokio::{
    net::TcpListener,
    task::{JoinHandle, JoinSet},
};

use crate::filesystem::Filesystem;

pub struct MetricsServer {
    address: SocketAddr,
    task: JoinHandle<()>,
}

impl MetricsServer {
    /// Bind before mounting; an occupied/invalid address is a startup error.
    pub async fn bind(address: SocketAddr, fs: Arc<Filesystem>) -> io::Result<Self> {
        let listener = TcpListener::bind(address).await?;
        let address = listener.local_addr()?;
        let task = tokio::spawn(serve(listener, fs));
        Ok(Self { address, task })
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    pub async fn shutdown(mut self) {
        self.task.abort();
        // Await destruction of the listener and its connection JoinSet.
        let _ = (&mut self.task).await;
    }
}

impl Drop for MetricsServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve(listener: TcpListener, fs: Arc<Filesystem>) {
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            _ = connections.join_next(), if !connections.is_empty() => {},
            accepted = listener.accept(), if connections.len() < 2 => {
                let (stream, _) = match accepted {
                    Ok(value) => value,
                    Err(error) => {
                        eprintln!("metrics listener failed: {error}");
                        break;
                    }
                };
                let fs = fs.clone();
                connections.spawn(async move {
                    let service = service_fn(move |request| {
                        let response = response(request, &fs);
                        std::future::ready(Ok::<_, Infallible>(response))
                    });
                    let _ = tokio::time::timeout(Duration::from_secs(5),
                        http1::Builder::new().max_headers(16)
                            .max_buf_size(8192).serve_connection(TokioIo::new(stream), service)
                    ).await;
                });
            }
        }
    }
}

fn response(request: Request<Incoming>, fs: &Filesystem) -> Response<Full<Bytes>> {
    let (status, body) = if request.uri().path() != "/metrics" {
        (StatusCode::NOT_FOUND, "not found\n".to_owned())
    } else if request.method() != Method::GET {
        (StatusCode::METHOD_NOT_ALLOWED, "GET required\n".to_owned())
    } else if let Some(text) = super::render(fs) {
        (StatusCode::OK, text)
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "metrics snapshot busy\n".to_owned(),
        )
    };
    Response::builder()
        .status(status)
        .header(
            "Content-Type",
            if status == StatusCode::OK {
                "application/openmetrics-text; version=1.0.0; charset=utf-8"
            } else {
                "text/plain; charset=utf-8"
            },
        )
        .header("Connection", "close")
        .header("Allow", "GET")
        .body(Full::new(Bytes::from(body)))
        .expect("static response headers are valid")
}
