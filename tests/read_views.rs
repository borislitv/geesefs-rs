#![forbid(unsafe_code)]
mod support;
use geesefs_rs::filesystem::Filesystem;
use support::MemoryStore;

const MIB: usize = 1024 * 1024;
const STEP: usize = 128 * 1024;

#[tokio::test]
async fn cached_reply_keeps_the_complete_backing_charge_without_another_payload_allocation() {
    let expected: Vec<u8> = (0..2 * MIB).map(|at| (at % 251) as u8).collect();
    let store = MemoryStore::new();
    store.seed("cached", &expected);
    store.seed("other", &vec![b'o'; 2 * MIB]);
    store.seed("pressure", &vec![b'p'; 2 * MIB]);
    let mut config = MemoryStore::config(16);
    config.large_read_cutoff = 0;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let node = fs.lookup(1, "cached").await.unwrap();
    let other = fs.lookup(1, "other").await.unwrap();
    let pressure = fs.lookup(1, "pressure").await.unwrap();
    drop(fs.read(node.ino, 0, STEP).await.unwrap());
    assert_eq!(fs.effective_budgets().data_used_bytes, 2 * MIB);

    let view = fs.read(node.ino, 65_539, 4096).await.unwrap();
    assert_eq!(view.as_slice(), &expected[65_539..65_539 + 4096]);
    assert_eq!(
        fs.effective_budgets().data_used_bytes,
        2 * MIB,
        "a cache-hit reply must share its already charged allocation, not allocate another payload"
    );
    drop(fs.read(other.ino, 0, STEP).await.unwrap());
    drop(fs.read(pressure.ino, 0, STEP).await.unwrap());
    assert_eq!(view.as_slice(), &expected[65_539..65_539 + 4096]);
    assert_eq!(
        fs.effective_budgets().data_used_bytes,
        4 * MIB,
        "eviction must not release the full backing charge while a small view still owns it"
    );
    drop(view);
    assert_eq!(fs.effective_budgets().data_used_bytes, 2 * MIB);
}

#[tokio::test]
async fn cached_reply_remains_an_immutable_generation_after_a_write_and_fsync() {
    let expected: Vec<u8> = (0..2 * MIB).map(|at| (at % 251) as u8).collect();
    let store = MemoryStore::new();
    store.seed("file", &expected);
    let mut config = MemoryStore::config(16);
    config.large_read_cutoff = 0;
    let fs = Filesystem::new(config, store.clone()).unwrap();
    let node = fs.lookup(1, "file").await.unwrap();
    drop(fs.read(node.ino, 0, STEP).await.unwrap());
    let old = fs.read(node.ino, 65_539, 4096).await.unwrap();
    fs.write(node.ino, 65_539, b"new-generation").await.unwrap();
    fs.fsync(node.ino).await.unwrap();
    assert_eq!(old.as_slice(), &expected[65_539..65_539 + 4096]);
    drop(old);
    let new = fs.read(node.ino, 65_539, 4096).await.unwrap();
    assert_eq!(&new.as_slice()[..14], b"new-generation");
    assert_eq!(&new.as_slice()[14..], &expected[65_539 + 14..65_539 + 4096]);
    assert_eq!(&store.bytes("file")[65_539..65_539 + 14], b"new-generation");
    assert!(fs.effective_budgets().data_used_bytes <= fs.effective_budgets().data_shared_bytes);
}
