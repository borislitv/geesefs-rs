//! Optional persistent payload tier with bounded application work.
mod funding;
mod key;
mod policy;
mod regions;
mod runtime;
mod store;
#[cfg(test)]
mod testing;

#[cfg(test)]
pub use testing::TestGate;

pub use funding::Funding;
pub use key::{DiskKey, MAX_IDENTITY_BYTES, MAX_PAYLOAD_BYTES};
pub use regions::RegionStats;
pub use store::{
    Demotion, DiskCache, DiskCacheStatus, FundedPreparedDiskCache, LatencyStats, PreparedDiskCache,
    READ_WAIT_BYTES, ReadAdmission, ReadFallback, SpillAdmission, SpillDrop, StartupStats, State,
    Stats,
};
