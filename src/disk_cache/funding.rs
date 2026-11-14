use super::key::{MAX_PAYLOAD_BYTES, RECORD_CAP};
use crate::error::{Error, Result};

pub(crate) const PAGE: usize = 4096;
pub(crate) const JOBS: usize = 4;
pub(crate) const POOL: usize = 512 * 1024;
pub(crate) const fn rounded(n: usize) -> usize {
    n.div_ceil(PAGE) * PAGE
}

/// Required deduction from effective L1 capacity, not an RSS/cgroup guarantee.
/// Index coefficient includes hash/order state: measured ~230 B/entry at 8M,
/// conservatively funded at 384, plus separately bounded recent hints/regions.
#[derive(Clone, Debug)]
pub struct Funding {
    pub capacity: usize,
    pub region_bytes: usize,
    pub index_entries: usize,
    pub recent_entries: usize,
    pub recovery_bytes: usize,
    pub fixed_bytes: usize,
    pub metadata_bytes: usize,
    pub copy_bytes: usize,
    pub spill_jobs: usize,
    pub read_work_bytes: usize,
    pub submission_bytes: usize,
    pub required_bytes: usize,
}
impl Funding {
    pub fn derive(capacity: u64, available_payload_bytes: usize) -> Result<Self> {
        let capacity = usize::try_from(capacity)
            .map_err(|_| Error::Config("disk capacity exceeds address space".into()))?;
        if capacity < 8 * 1024 * 1024
            || !capacity.is_multiple_of(PAGE)
            || capacity as u64 > (1_u64 << 44)
        {
            return Err(Error::Config(
                "disk capacity must be page-aligned, between 8 MiB and 16 TiB".into(),
            ));
        }
        let region_bytes = if capacity >= 256 * 1024 * 1024 {
            16 * 1024 * 1024
        } else {
            1024 * 1024
        };
        let regions = capacity / region_bytes;
        // Partial records cannot increase this independently funded index cap.
        let index_entries = (capacity / MAX_PAYLOAD_BYTES)
            .min(available_payload_bytes / 4 / 512)
            .clamp(1, 8_388_608);
        let recent_entries = index_entries.min(65_536);
        let recovery_bytes = rounded(8 * PAGE + 16 * regions + 2 * PAGE);
        let fixed_bytes = 2 * POOL + recovery_bytes + 1024 * 1024;
        let metadata_bytes = 384 * index_entries + 128 * recent_entries + 256 * regions;
        let copy_bytes = JOBS * MAX_PAYLOAD_BYTES;
        // Concurrent encoded records, not raw payloads, must fit one flusher
        // buffer. Keep the existing funding deduction and copy ceiling.
        let spill_jobs =
            JOBS.min(POOL / (RECORD_CAP + 2 * foyer::SubmissionLimits::METADATA_BYTES));
        // Each admitted read may own serialized buffer, decoded key/value, and
        // task bookkeeping simultaneously. Caller output is separately admitted.
        let read_work_bytes =
            JOBS * (RECORD_CAP + 3 * super::MAX_IDENTITY_BYTES + MAX_PAYLOAD_BYTES + PAGE);
        let submission_bytes = JOBS * (RECORD_CAP + 2 * foyer::SubmissionLimits::METADATA_BYTES);
        let required_bytes =
            fixed_bytes + metadata_bytes + copy_bytes + read_work_bytes + submission_bytes;
        if required_bytes > available_payload_bytes {
            return Err(Error::Budget {
                requested: required_bytes,
                limit: available_payload_bytes,
            });
        }
        Ok(Self {
            capacity,
            region_bytes,
            index_entries,
            recent_entries,
            recovery_bytes,
            fixed_bytes,
            metadata_bytes,
            copy_bytes,
            spill_jobs,
            read_work_bytes,
            submission_bytes,
            required_bytes,
        })
    }
}
