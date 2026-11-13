#![forbid(unsafe_code)]
mod support;

use geesefs_rs::{backend::Store, filesystem::Filesystem};
use std::time::Duration;
use support::MemoryStore;

// The wire codec intentionally lowercases names and replaces invalid UTF-8.
// Those reopen limitations must not change the bytes/names on a live inode.
async fn same_inode_roundtrip(
    metadata_limit: Option<usize>,
    name: &str,
    value: &[u8],
    wire_name: &str,
    wire_value: &str,
) {
    tokio::time::timeout(Duration::from_secs(10), async {
        let store = MemoryStore::new();
        store.seed("file", b"abcdef");
        let original = store.head("file").await.unwrap();
        let mut config = MemoryStore::config(32);
        config.metadata_limit = metadata_limit;
        let fs = Filesystem::new(config, store.clone()).unwrap();
        let node = fs.lookup(1, "file").await.unwrap();
        fs.setxattr(node.ino, name, Some(value), libc::XATTR_CREATE)
            .unwrap();
        fs.setxattr(node.ino, "user.control", Some(b"ordinary"), 0)
            .unwrap();
        assert_eq!(
            fs.xattrs(node.ino).unwrap().get(name),
            Some(value),
            "setxattr must expose the exact name and bytes before fsync"
        );

        fs.fsync(node.ino).await.expect("metadata fsync succeeds");
        let committed = store.head("file").await.unwrap();
        assert_ne!(committed.etag, original.etag);
        assert_eq!(store.bytes("file"), b"abcdef");
        assert_eq!(committed.metadata[wire_name], wire_value);
        assert_eq!(committed.metadata["control"], "ordinary");
        assert_eq!(
            fs.xattrs(node.ino).unwrap().get("s3.etag"),
            Some(committed.etag.as_bytes()),
            "fsync must still publish the committed remote generation"
        );
        fs.fsync(node.ino).await.unwrap();
        assert_eq!(
            store.head("file").await.unwrap().etag,
            committed.etag,
            "an already durable inode must not be uploaded again"
        );
        {
            let attrs = fs.xattrs(node.ino).unwrap();
            assert_eq!(attrs.get("user.control"), Some(&b"ordinary"[..]));
            assert_eq!(
                attrs.get(name),
                Some(value),
                "successful fsync must preserve same-inode xattr name and bytes (metadata_limit={metadata_limit:?})"
            );
        }

        // A later partial write must read unchanged bytes from the new durable
        // generation, not the stale pre-fsync generation. MemoryStore enforces
        // the conditional generation on these base reads.
        fs.write(node.ino, 2, b"XY").await.unwrap();
        fs.fsync(node.ino).await.unwrap();
        let updated = store.head("file").await.unwrap();
        assert_ne!(updated.etag, committed.etag);
        assert_eq!(store.bytes("file"), b"abXYef");
        assert_eq!(updated.metadata[wire_name], wire_value);
        assert_eq!(updated.metadata["control"], "ordinary");
        let attrs = fs.xattrs(node.ino).unwrap();
        assert_eq!(attrs.get(name), Some(value));
        assert_eq!(
            attrs.get("s3.etag"),
            Some(updated.etag.as_bytes())
        );
    })
    .await
    .expect("same-inode fsync xattr regression deadline");
}

#[tokio::test]
async fn uppercase_name_survives_fsync_without_metadata_budget() {
    same_inode_roundtrip(None, "user.UPPER", b"value", "upper", "value").await;
}

#[tokio::test]
async fn uppercase_name_survives_fsync_with_metadata_budget() {
    same_inode_roundtrip(Some(1024 * 1024), "user.UPPER", b"value", "upper", "value").await;
}

#[tokio::test]
async fn binary_value_survives_fsync_without_metadata_budget() {
    same_inode_roundtrip(None, "user.binary", &[0xff], "binary", "\u{fffd}").await;
}

#[tokio::test]
async fn binary_value_survives_fsync_with_metadata_budget() {
    same_inode_roundtrip(
        Some(1024 * 1024),
        "user.binary",
        &[0xff],
        "binary",
        "\u{fffd}",
    )
    .await;
}

#[tokio::test]
async fn lowercase_ascii_survives_fsync_without_metadata_budget() {
    same_inode_roundtrip(None, "user.lower", b"value", "lower", "value").await;
}

#[tokio::test]
async fn lowercase_ascii_survives_fsync_with_metadata_budget() {
    same_inode_roundtrip(Some(1024 * 1024), "user.lower", b"value", "lower", "value").await;
}
