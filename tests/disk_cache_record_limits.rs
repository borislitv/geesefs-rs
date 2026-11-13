#![forbid(unsafe_code)]
#[path = "support/disk.rs"]
mod support;
pub use support::{budget, disk_cache, error};

use budget::Budget;
use disk_cache::{
    Demotion, DiskCache, DiskKey, Funding, MAX_IDENTITY_BYTES, MAX_PAYLOAD_BYTES, PreparedDiskCache,
};
use foyer::Code;
use std::sync::{Arc, Mutex};
use support::{DEADLINE, MIB, block, open};

#[derive(Debug)]
struct RecordReadBuffers(Arc<Mutex<Vec<usize>>>);

#[derive(Debug)]
struct RecordedIo {
    real: Arc<dyn foyer::IoEngine>,
    lengths: Arc<Mutex<Vec<usize>>>,
}

impl foyer::IoEngineConfig for RecordReadBuffers {
    fn build(
        self: Box<Self>,
        context: foyer::IoEngineBuildContext,
    ) -> futures_util::future::BoxFuture<'static, foyer::Result<Arc<dyn foyer::IoEngine>>> {
        Box::pin(async move {
            let real = Box::new(foyer::PsyncIoEngineConfig::new())
                .build(context)
                .await?;
            Ok(Arc::new(RecordedIo {
                real,
                lengths: self.0,
            }) as Arc<dyn foyer::IoEngine>)
        })
    }
}

impl foyer::IoEngine for RecordedIo {
    fn read(
        &self,
        buffer: foyer_storage::IoReadBuffer,
        partition: &dyn foyer_storage::Partition,
        offset: u64,
    ) -> foyer::IoHandle {
        self.lengths.lock().unwrap().push(buffer.len());
        self.real.read(buffer, partition, offset)
    }

    fn write(
        &self,
        buffer: foyer_storage::IoWriteBuffer,
        partition: &dyn foyer_storage::Partition,
        offset: u64,
    ) -> foyer::IoHandle {
        self.real.write(buffer, partition, offset)
    }
}

fn boundary_key(identity_bytes: usize, payload_bytes: usize) -> DiskKey {
    // Wire header 34 B, plus one object byte and one ETag byte. An opaque
    // namespace fills the rest; no test expectation uses the storage limit.
    DiskKey::new(
        &vec![0xff; identity_bytes - 36],
        b"k",
        b"\0",
        131_072,
        0,
        payload_bytes,
    )
    .unwrap()
}

#[tokio::test]
async fn maximum_and_page_neighbor_records_survive_drain_and_reopen() {
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        let directory = support::directory();
        let resources = Budget::new(32 * MIB).unwrap();
        let output = Budget::new(131_072).unwrap();
        // Raw record = 36-B Foyer header + identity + 12-B value envelope
        // + payload. Cover 147456 (36 pages) -1/exact/+1, then raw maximum
        // 147504 -1/exact. These literal expectations are independent of the
        // production rounding helper and use the real maximum payload.
        let cases = [
            (16_384, 131_072),
            (16_335, 131_072),
            (16_336, 131_072),
            (16_337, 131_072),
            (16_383, 131_072),
            (16_384, 131_071),
        ];
        let expected = (0_u8..=255).cycle().take(131_072).collect::<Vec<_>>();
        let lengths = Arc::new(Mutex::new(Vec::new()));
        let cache = DiskCache::open_test(
            PreparedDiskCache::prepare(directory.path(), (16 * MIB) as u64).unwrap(),
            &resources,
            Some(Box::new(RecordReadBuffers(lengths.clone()))),
        )
        .await
        .unwrap();
        for (identity, length) in cases {
            let key = boundary_key(identity, length);
            let mut encoded = Vec::new();
            key.encode(&mut encoded).unwrap();
            assert_eq!(encoded.len(), identity);
            assert_eq!(
                cache.try_demote(key, block(&expected[..length])),
                Demotion::Admitted
            );
            // Copy admission is not publication. Drain each record instead of
            // assuming a batch of best-effort spills all become readable.
            cache.drain(DEADLINE).await.unwrap();
        }
        lengths.lock().unwrap().clear();
        for (identity, length) in cases {
            let actual = cache
                .read_block_with_deadline(&boundary_key(identity, length), &output, DEADLINE)
                .await
                .unwrap_or_else(|| panic!("record missing: identity={identity}, payload={length}"));
            assert_eq!(actual.as_slice(), &expected[..length]);
        }
        let observed = lengths.lock().unwrap().clone();
        assert!(
            observed.contains(&151_552),
            "maximum padded read absent: {observed:?}"
        );
        assert!(
            observed.iter().all(|&n| n <= 151_552),
            "unfunded padded read: {observed:?}"
        );
        assert_eq!(output.used(), 0);
        cache.close(DEADLINE).await.unwrap();
        drop(cache);
        assert_eq!(resources.used(), 0);

        let reopened = open(directory.path(), &resources).await;
        for (identity, length) in cases {
            let actual = reopened
                .read_block_with_deadline(&boundary_key(identity, length), &output, DEADLINE)
                .await
                .unwrap_or_else(|| {
                    panic!("record missing after reopen: identity={identity}, payload={length}")
                });
            assert_eq!(actual.as_slice(), &expected[..length]);
        }
        assert_eq!(output.used(), 0);
        reopened.close(DEADLINE).await.unwrap();
        let stats = reopened.stats();
        assert_eq!(
            (
                stats.active_jobs,
                stats.copied_bytes,
                stats.submission_bytes
            ),
            (0, 0, 0)
        );
        drop(reopened);
        assert_eq!(resources.used(), 0);
    })
    .await
    .expect("bounded real-engine record boundary fixture");
}

#[test]
fn one_byte_above_codec_limits_is_not_a_valid_key() {
    assert_eq!((MAX_IDENTITY_BYTES, MAX_PAYLOAD_BYTES), (16_384, 131_072));
    assert!(DiskKey::new(&vec![0xff; 16_349], b"k", b"e", 131_072, 0, 131_072).is_err());
    assert!(DiskKey::new(b"n", b"k", b"e", 131_073, 0, 131_073).is_err());
}

#[test]
fn read_and_submission_funding_cover_the_existing_maximum_physical_record() {
    let funding = Funding::derive((16 * MIB) as u64, 32 * MIB).unwrap();
    // Four jobs, each with a 151552-B padded I/O buffer. Existing decoded
    // identity/value and native submission bookkeeping allowances stay intact.
    assert_eq!(funding.read_work_bytes, 1_343_488);
    assert_eq!(funding.submission_bytes, 610_304);
    assert!(Funding::derive((16 * MIB) as u64, funding.required_bytes - 1).is_err());
    assert_eq!(
        Funding::derive((16 * MIB) as u64, funding.required_bytes)
            .unwrap()
            .required_bytes,
        funding.required_bytes
    );
}
