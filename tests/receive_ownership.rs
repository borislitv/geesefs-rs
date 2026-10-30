//! Exercise the public S3 reader with the shared receive-buffer lifecycle used
//! by HTTP bodies. Fully consumed transport chunks must not prevent reuse.
use std::{
    collections::VecDeque,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};

use aws_sdk_s3::{
    Client,
    config::{BehaviorVersion, Credentials, Region, retry::RetryConfig, timeout::TimeoutConfig},
};
use aws_smithy_runtime_api::client::{
    http::{
        HttpClient, HttpConnector, HttpConnectorFuture, HttpConnectorSettings, SharedHttpConnector,
    },
    orchestrator::{HttpRequest, HttpResponse},
    runtime_components::RuntimeComponents,
};
use aws_smithy_types::body::SdkBody;
use bytes::{Bytes, BytesMut};
use geesefs_rs::{
    backend::{
        RangeSink,
        s3::{ObjectHead, S3},
    },
    budget::Budget,
};
use http_body::{Body, Frame};

struct ReceiveBody {
    chunks: VecDeque<Vec<u8>>,
    buffer: BytesMut,
    capacity: usize,
    delivered: bool,
    reuse: Arc<Mutex<Vec<bool>>>,
}

impl Body for ReceiveBody {
    type Data = Bytes;
    type Error = std::io::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        if self.delivered {
            let capacity = self.capacity;
            let reclaimed = self.buffer.try_reclaim(capacity);
            self.reuse.lock().unwrap().push(reclaimed);
        }
        match self.chunks.pop_front() {
            Some(chunk) => {
                self.buffer.extend_from_slice(&chunk);
                self.delivered = !chunk.is_empty();
                Poll::Ready(Some(Ok(Frame::data(self.buffer.split().freeze()))))
            }
            None => Poll::Ready(None),
        }
    }
}

#[derive(Clone, Debug)]
struct ResponseClient(Arc<Mutex<Option<HttpResponse>>>);

impl HttpClient for ResponseClient {
    fn http_connector(
        &self,
        _: &HttpConnectorSettings,
        _: &RuntimeComponents,
    ) -> SharedHttpConnector {
        SharedHttpConnector::new(self.clone())
    }
}

impl HttpConnector for ResponseClient {
    fn call(&self, request: HttpRequest) -> HttpConnectorFuture {
        assert_eq!(request.method(), "GET");
        assert_eq!(
            request.headers().get("if-match"),
            Some("\"generation-one\"")
        );
        let response = self.0.lock().unwrap().take().expect("one conditional GET");
        HttpConnectorFuture::new(async move { Ok(response) })
    }
}

#[tokio::test]
async fn consumed_transport_chunks_are_reusable_before_the_next_body_poll() {
    for (chunk_size, count) in [(8192, 33), (49157, 7)] {
        let chunks: VecDeque<_> = (0..count)
            .flat_map(|index| [vec![b'a' + index; chunk_size], Vec::new()])
            .collect();
        let expected: Vec<_> = chunks.iter().flatten().copied().collect();
        let reuse = Arc::new(Mutex::new(Vec::new()));
        let body = ReceiveBody {
            chunks,
            buffer: BytesMut::with_capacity(chunk_size),
            capacity: chunk_size,
            delivered: false,
            reuse: reuse.clone(),
        };
        let response = http::Response::builder()
            .status(206)
            .header("content-length", expected.len())
            .header(
                "content-range",
                format!("bytes 0-{0}/{1}", expected.len() - 1, expected.len()),
            )
            .body(SdkBody::from_body_1_x(body))
            .unwrap();
        let config = aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new("us-east-1"))
            .credentials_provider(Credentials::new("test", "test", None, None, "test-only"))
            .endpoint_url("http://unused.invalid")
            .http_client(ResponseClient(Arc::new(Mutex::new(Some(
                HttpResponse::try_from(response).unwrap(),
            )))))
            .retry_config(RetryConfig::standard().with_max_attempts(1))
            .timeout_config(
                TimeoutConfig::builder()
                    .operation_timeout(Duration::from_secs(3))
                    .build(),
            )
            .build();
        let store = S3::new(Client::from_conf(config), "bucket".into(), String::new()).unwrap();
        let data = Budget::new(expected.len()).unwrap();
        let local = Budget::new(expected.len()).unwrap();
        let (sender, mut receiver) = tokio::sync::mpsc::channel(4);
        let output = RangeSink::new(sender, data.reserve(expected.len()).await.unwrap());
        let head = ObjectHead {
            size: expected.len() as u64,
            etag: "\"generation-one\"".into(),
            ..Default::default()
        };
        store
            .read_range_chunks("file", 0, expected.len(), &head, &local, &output)
            .await
            .unwrap();
        drop(output);
        let mut observed = Vec::new();
        while let Some((offset, block)) = receiver.recv().await {
            assert_eq!(offset as usize, observed.len());
            observed.extend_from_slice(block.as_slice());
        }
        assert_eq!(
            observed, expected,
            "transport chunk boundaries must not change payloads"
        );
        assert_eq!(data.used(), 0);
        assert_eq!(local.used(), 0);
        let reuse = reuse.lock().unwrap();
        assert_eq!(reuse.len(), count as usize);
        assert!(
            reuse.iter().all(|reclaimed| *reclaimed),
            "a consumed SDK chunk must release its receive-buffer reference before the next body poll: {reuse:?}"
        );
    }
}
