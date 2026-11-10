use std::str::FromStr;

use regex::{Regex, RegexBuilder};

/// Compiled selection for UTF-8, mount-relative regular-file paths.
/// The caller resolves symlinks and supplies a leading `/`, without the S3 prefix.
#[derive(Clone, Debug)]
pub struct CachePolicy(Regex);

impl CachePolicy {
    // Bound both source size and compiled/search state; do not compile per read.
    pub const MAX_PATTERN_BYTES: usize = 16 * 1024;
    const REGEX_SIZE_LIMIT: usize = 1024 * 1024;

    pub fn matches(&self, path: &str) -> bool {
        path.starts_with('/') && self.0.is_match(path)
    }
}

impl FromStr for CachePolicy {
    type Err = String;

    fn from_str(pattern: &str) -> Result<Self, Self::Err> {
        if pattern.len() > Self::MAX_PATTERN_BYTES {
            return Err(format!(
                "disk-cache-regex exceeds {} bytes",
                Self::MAX_PATTERN_BYTES
            ));
        }
        RegexBuilder::new(pattern)
            .size_limit(Self::REGEX_SIZE_LIMIT)
            .dfa_size_limit(Self::REGEX_SIZE_LIMIT)
            .nest_limit(128)
            .build()
            .map(Self)
            .map_err(|error| format!("invalid disk-cache-regex: {error}"))
    }
}
