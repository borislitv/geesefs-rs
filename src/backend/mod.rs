pub mod content_type;
pub mod extended_list;
pub mod metadata;
pub mod s3;

use crate::{
    budget::{Block, Budget, Reservation},
    error::{Error, Result},
};
use aws_sdk_s3::{primitives::ByteStream, types::CompletedPart};
use extended_list::ListPage;
use s3::ObjectHead;
use std::{collections::HashMap, sync::Arc};

/// Bounded delivery owned by one admitted range producer.
pub struct RangeSink {
    sender: tokio::sync::mpsc::Sender<(u64, Block)>,
    owner: Arc<Reservation>,
}

impl RangeSink {
    pub fn new(sender: tokio::sync::mpsc::Sender<(u64, Block)>, owner: Reservation) -> Self {
        Self {
            sender,
            owner: Arc::new(owner),
        }
    }

    fn check(&self, length: usize) -> Result<()> {
        if length > self.owner.size() {
            return Err(Error::Budget {
                requested: length,
                limit: self.owner.size(),
            });
        }
        Ok(())
    }

    pub async fn send(&self, offset: u64, block: Block) -> Result<()> {
        self.sender
            .send((offset, block.with_shared_owner(self.owner.clone())))
            .await
            .map_err(|_| Error::Cancelled)
    }
}

/// The storage boundary is shared by the real SDK backend and small deterministic
/// in-memory stores. Filesystem state, generation barriers and overlays are never mocked.
#[async_trait::async_trait]
pub trait Store: Send + Sync + 'static {
    async fn patch(
        &self,
        _key: &str,
        _offset: u64,
        _body: Block,
        _etag: &str,
        _append_part_size: Option<u64>,
    ) -> Result<s3::PatchResult> {
        Err(crate::error::Error::Unsupported(
            "PATCH backend operation".into(),
        ))
    }
    async fn head(&self, key: &str) -> Result<ObjectHead>;
    async fn read_range(
        &self,
        key: &str,
        offset: u64,
        length: usize,
        head: &ObjectHead,
        budget: &Budget,
    ) -> Result<Block>;
    async fn read_range_chunks(
        &self,
        key: &str,
        offset: u64,
        length: usize,
        head: &ObjectHead,
        budget: &Budget,
        output: &RangeSink,
    ) -> Result<()> {
        output.check(length.min(head.size.saturating_sub(offset) as usize))?;
        let block = self.read_range(key, offset, length, head, budget).await?;
        output.send(offset, block).await
    }
    async fn put(
        &self,
        key: &str,
        body: ByteStream,
        metadata: HashMap<String, String>,
    ) -> Result<String>;
    async fn begin_upload(
        &self,
        key: &str,
        metadata: HashMap<String, String>,
        size: u64,
    ) -> Result<String>;
    async fn replace_metadata(
        &self,
        key: &str,
        etag: &str,
        metadata: HashMap<String, String>,
    ) -> Result<ObjectHead>;
    async fn upload_part(
        &self,
        key: &str,
        upload: &str,
        part: i32,
        body: ByteStream,
    ) -> Result<CompletedPart>;
    async fn copy_part(
        &self,
        destination: &str,
        upload: &str,
        part: i32,
        source: &str,
        range: std::ops::RangeInclusive<u64>,
        etag: &str,
    ) -> Result<CompletedPart>;
    async fn complete_upload(
        &self,
        key: &str,
        upload: &str,
        parts: Vec<CompletedPart>,
    ) -> Result<String>;
    async fn abort_upload(&self, key: &str, upload: &str) -> Result<()>;
    async fn copy(&self, source: &str, destination: &str, etag: &str) -> Result<String>;
    async fn delete(&self, key: &str) -> Result<()>;
    async fn list(
        &self,
        prefix: &str,
        delimiter: Option<&str>,
        token: Option<&str>,
    ) -> Result<ListPage>;
}

#[async_trait::async_trait]
impl Store for s3::S3 {
    async fn patch(
        &self,
        key: &str,
        offset: u64,
        body: Block,
        etag: &str,
        append_part_size: Option<u64>,
    ) -> Result<s3::PatchResult> {
        self.patch(key, offset, body, etag, append_part_size).await
    }
    async fn head(&self, key: &str) -> Result<ObjectHead> {
        self.head(key).await
    }
    async fn read_range(
        &self,
        key: &str,
        offset: u64,
        length: usize,
        head: &ObjectHead,
        budget: &Budget,
    ) -> Result<Block> {
        self.read_range(key, offset, length, head, budget).await
    }
    async fn read_range_chunks(
        &self,
        key: &str,
        offset: u64,
        length: usize,
        head: &ObjectHead,
        budget: &Budget,
        output: &RangeSink,
    ) -> Result<()> {
        self.read_range_chunks(key, offset, length, head, budget, output)
            .await
    }
    async fn put(
        &self,
        key: &str,
        body: ByteStream,
        metadata: HashMap<String, String>,
    ) -> Result<String> {
        self.put(key, body, metadata).await
    }
    async fn begin_upload(
        &self,
        key: &str,
        metadata: HashMap<String, String>,
        size: u64,
    ) -> Result<String> {
        self.begin_upload(key, metadata, size).await
    }
    async fn replace_metadata(
        &self,
        key: &str,
        etag: &str,
        metadata: HashMap<String, String>,
    ) -> Result<ObjectHead> {
        self.replace_metadata(key, etag, metadata).await
    }
    async fn upload_part(
        &self,
        key: &str,
        upload: &str,
        part: i32,
        body: ByteStream,
    ) -> Result<CompletedPart> {
        self.upload_part(key, upload, part, body).await
    }
    async fn copy_part(
        &self,
        destination: &str,
        upload: &str,
        part: i32,
        source: &str,
        range: std::ops::RangeInclusive<u64>,
        etag: &str,
    ) -> Result<CompletedPart> {
        self.copy_part(destination, upload, part, source, range, etag)
            .await
    }
    async fn complete_upload(
        &self,
        key: &str,
        upload: &str,
        parts: Vec<CompletedPart>,
    ) -> Result<String> {
        self.complete_upload(key, upload, parts).await
    }
    async fn abort_upload(&self, key: &str, upload: &str) -> Result<()> {
        self.abort_upload(key, upload).await
    }
    async fn copy(&self, source: &str, destination: &str, etag: &str) -> Result<String> {
        self.copy(source, destination, etag).await
    }
    async fn delete(&self, key: &str) -> Result<()> {
        self.delete(key).await
    }
    async fn list(
        &self,
        prefix: &str,
        delimiter: Option<&str>,
        token: Option<&str>,
    ) -> Result<ListPage> {
        self.list(prefix, delimiter, token).await
    }
}
pub mod auth;
mod options;
mod signature;
mod transport;
