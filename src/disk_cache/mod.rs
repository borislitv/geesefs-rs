//! Isolated, optional persistent payload tier. No filesystem read-path integration.
mod funding;
mod key;
mod policy;
mod store;

pub use funding::Funding;
pub use key::{DiskKey, MAX_IDENTITY_BYTES, MAX_PAYLOAD_BYTES};
pub use store::{Demotion, DiskCache, DiskCacheStatus, PreparedDiskCache, State, Stats};
