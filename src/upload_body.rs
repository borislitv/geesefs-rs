//! Exact-size, retryable SDK body backed by bounded materialization, not a part Vec.
use crate::{
    backend::Store,
    budget::{Block, Budget},
    error::{Error, Result},
    write::{Snapshot, materialize},
};
use aws_sdk_s3::primitives::ByteStream;
use aws_smithy_types::body::SdkBody;
use bytes::Bytes;
use http_body::{Body, Frame, SizeHint};
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

pub const CHUNK_SIZE: usize = 1024 * 1024;
type Pending = Pin<Box<dyn Future<Output = Result<Block>> + Send>>;
struct UploadBody {
    store: Arc<dyn Store>,
    snapshot: Arc<Snapshot>,
    lane: Budget,
    offset: u64,
    end: u64,
    pending: Mutex<Option<Pending>>,
}
impl Body for UploadBody {
    type Data = Bytes;
    type Error = Error;
    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>>>> {
        let this = self.get_mut();
        if this.offset == this.end {
            return Poll::Ready(None);
        }
        let pending = this
            .pending
            .get_mut()
            .expect("upload future mutex is exclusively polled");
        let length = (this.end - this.offset).min(CHUNK_SIZE as u64) as usize;
        if pending.is_none() {
            let (store, snapshot, lane, offset) = (
                this.store.clone(),
                this.snapshot.clone(),
                this.lane.clone(),
                this.offset,
            );
            *pending = Some(Box::pin(async move {
                admitted(&store, &snapshot, offset, length, &lane).await
            }));
        }
        match pending
            .as_mut()
            .expect("initialized materialization")
            .as_mut()
            .poll(cx)
        {
            Poll::Pending => Poll::Pending,
            Poll::Ready(result) => {
                *pending = None;
                match result {
                    Ok(block) => {
                        this.offset += length as u64;
                        Poll::Ready(Some(Ok(Frame::data(Bytes::from_owner(block)))))
                    }
                    Err(error) => {
                        this.offset = this.end;
                        Poll::Ready(Some(Err(error)))
                    }
                }
            }
        }
    }
    fn size_hint(&self) -> SizeHint {
        SizeHint::with_exact(self.end - self.offset)
    }
    fn is_end_stream(&self) -> bool {
        self.end == self.offset
    }
}

pub fn stream(
    store: &Arc<dyn Store>,
    snapshot: &Snapshot,
    start: u64,
    length: u64,
    lane: &Budget,
) -> ByteStream {
    stream_snapshot(store, Arc::new(snapshot.clone()), start, length, lane)
}
pub fn stream_snapshot(
    store: &Arc<dyn Store>,
    snapshot: Arc<Snapshot>,
    start: u64,
    length: u64,
    lane: &Budget,
) -> ByteStream {
    let (store, lane) = (store.clone(), lane.clone());
    ByteStream::new(SdkBody::retryable(move || {
        SdkBody::from_body_1_x(UploadBody {
            store: store.clone(),
            snapshot: snapshot.clone(),
            lane: lane.clone(),
            offset: start,
            end: start + length,
            pending: Mutex::new(None),
        })
    }))
}

pub async fn admitted(
    store: &Arc<dyn Store>,
    snapshot: &Snapshot,
    offset: u64,
    length: usize,
    lane: &Budget,
) -> Result<Block> {
    // Admit output and conditional-GET scratch atomically, avoiding cross-flusher
    // half-lane deadlocks. Only the emitted frame remains globally charged.
    let mut charge = lane.reserve(2 * length).await?;
    let local = Budget::new((2 * length).max(1))?;
    let block = materialize(store, snapshot, offset, length, &local).await?;
    Ok(block.with_owner(charge.split(length)))
}
