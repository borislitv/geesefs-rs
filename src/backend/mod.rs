pub mod content_type;
pub mod extended_list;
pub mod metadata;
pub mod s3;

use crate::{
    budget::{Block, Budget},
    error::Result,
};
use aws_sdk_s3::{primitives::ByteStream, types::CompletedPart};
use extended_list::ListPage;
use s3::ObjectHead;
use std::collections::HashMap;

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
