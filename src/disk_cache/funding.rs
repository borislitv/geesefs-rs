use super::key::{MAX_PAYLOAD_BYTES, RECORD_CAP};
use crate::error::{Error, Result};

pub(crate) const PAGE: usize = 4096;
pub(crate) const JOBS: usize = 4;
pub(crate) const POOL: usize = 512 * 1024;
pub(crate) const fn rounded(n: usize) -> usize {
    n.div_ceil(PAGE) * PAGE
}

// Region boundaries are part of Foyer's on-disk layout, not just a tuning knob.
pub(super) struct Layout {
    region_bytes: usize,
    data_file: &'static str,
}
impl Layout {
    pub(super) const R64: Self = Self {
        region_bytes: 64 << 20,
        data_file: "payload-upstream-r64-v1.cache",
    };

    fn for_capacity(capacity: u64) -> Self {
        if capacity >= 8 << 40 {
            return Self::R64;
        }
        Self {
            region_bytes: if capacity < 256 << 20 {
                1 << 20
            } else {
                16 << 20
            },
            data_file: "payload-upstream-v1.cache",
        }
    }
}

/// Required deduction from active data capacity, not an RSS/cgroup guarantee.
/// Retains the conservative nominal allowance for the first upstream comparison.
/// Native index/recovery allocations are not capped by this reservation.
#[derive(Clone, Debug)]
pub struct Funding {
    pub capacity: usize,
    pub region_bytes: usize,
    pub(super) data_file: &'static str,
    pub nominal_entries: usize,
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
        let funding = Self::for_layout(capacity, Layout::for_capacity(capacity))?;
        funding.check_limit(available_payload_bytes)?;
        Ok(funding)
    }

    pub(super) fn for_layout(capacity: u64, layout: Layout) -> Result<Self> {
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
        let Layout {
            region_bytes,
            data_file,
        } = layout;
        let regions = capacity / region_bytes;
        // This plans full-size records; small records can produce more native
        // index entries. There is no application-enforced index slot ceiling.
        let nominal_entries = (capacity / MAX_PAYLOAD_BYTES).max(1);
        let recent_entries = nominal_entries.min(65_536);
        let recovery_bytes = rounded(8 * PAGE + 16 * regions + 2 * PAGE);
        let fixed_bytes = 2 * POOL + recovery_bytes + 1024 * 1024;
        let metadata_bytes = 384 * nominal_entries + 128 * recent_entries + 256 * regions;
        let copy_bytes = JOBS * MAX_PAYLOAD_BYTES;
        let spill_jobs = JOBS.min(POOL / RECORD_CAP);
        // Each admitted read may own serialized buffer, decoded key/value, and
        // task bookkeeping simultaneously. Caller output is separately admitted.
        let read_work_bytes =
            JOBS * (RECORD_CAP + 3 * super::MAX_IDENTITY_BYTES + MAX_PAYLOAD_BYTES + PAGE);
        let submission_bytes = JOBS * RECORD_CAP;
        let required_bytes =
            fixed_bytes + metadata_bytes + copy_bytes + read_work_bytes + submission_bytes;
        Ok(Self {
            capacity,
            region_bytes,
            data_file,
            nominal_entries,
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

    pub(super) fn check_limit(&self, available_payload_bytes: usize) -> Result<()> {
        if self.required_bytes > available_payload_bytes {
            return Err(Error::Budget {
                requested: self.required_bytes,
                limit: available_payload_bytes,
            });
        }
        Ok(())
    }
}
