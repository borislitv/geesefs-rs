#![forbid(unsafe_code)]
#[path = "support/disk.rs"]
mod support;
pub use support::{budget, disk_cache, error};

/// Short local control, not a production throughput or release acceptance gate.
#[tokio::test]
#[ignore = "opt-in deterministic L2 decode timing control"]
async fn short_l2_decode_control() {
    use budget::Budget;
    use disk_cache::{Demotion, MAX_PAYLOAD_BYTES, READ_WAIT_BYTES};
    use std::time::Instant;
    use support::{DEADLINE, MIB, block, key, open};

    let dir = support::directory();
    let resources = Budget::new(32 * MIB).unwrap();
    let output = Budget::new(MAX_PAYLOAD_BYTES).unwrap();
    let credit = Budget::new(READ_WAIT_BYTES).unwrap();
    let cache = open(dir.path(), &resources).await;
    let payload: Vec<u8> = (0..MAX_PAYLOAD_BYTES).map(|i| (i % 251) as u8).collect();
    let key = key(0, payload.len());
    assert_eq!(
        cache.try_demote(key.clone(), block(&payload)),
        Demotion::Admitted
    );
    cache.drain(DEADLINE).await.unwrap();
    let mut elapsed = std::time::Duration::ZERO;
    for i in 0..1050 {
        let start = Instant::now();
        let slot = cache
            .read_admission(credit.try_reserve(READ_WAIT_BYTES).unwrap(), DEADLINE)
            .await
            .unwrap();
        let value = slot
            .read_reserved(&key, output.try_reserve(payload.len()).unwrap())
            .await
            .unwrap();
        assert_eq!(value.as_slice(), payload);
        assert_eq!(output.used(), payload.len());
        drop(value);
        assert_eq!(output.used(), 0);
        assert_eq!(credit.used(), 0);
        if i >= 50 {
            elapsed += start.elapsed();
        }
    }
    println!(
        "L2 len={} iterations=1000 ns_per_read={}",
        payload.len(),
        elapsed.as_nanos() / 1000
    );
    cache.close(DEADLINE).await.unwrap();
    drop(cache);
    assert_eq!(resources.used(), 0);
}
