//! Persistent payload identity, independent of process-local inode generations.

use sha2::{Digest, Sha256};

/// Length-framed fields avoid concatenation aliases and keep the encoding stable
/// across architectures. Domains version both the format and its meaning.
pub(crate) struct NamespaceHash(Sha256);

impl NamespaceHash {
    pub(crate) fn new(domain: &[u8]) -> Self {
        let mut hash = Self(Sha256::new());
        hash.field(domain);
        hash
    }

    pub(crate) fn field(&mut self, value: &[u8]) {
        self.0.update((value.len() as u64).to_be_bytes());
        self.0.update(value);
    }

    pub(crate) fn finish(self) -> [u8; 32] {
        self.0.finalize().into()
    }
}

/// Why a backend cannot safely reuse persistent payloads. This is policy, not an
/// authentication failure, and must not be treated as anonymous access.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CacheBypass {
    UnsupportedBackend,
    OpaqueCredentials,
    TemporaryCredentials,
    OpaqueAuthentication,
    UnresolvedEndpoint,
}

pub(crate) fn encryption_context(
    sse: bool,
    requester: bool,
    kms: Option<&str>,
    customer: Option<&[u8]>,
) -> [u8; 32] {
    let mut hash = NamespaceHash::new(b"geesefs-encryption-v1");
    hash.field(&[u8::from(sse), u8::from(requester)]);
    hash.field(&[u8::from(kms.is_some())]);
    hash.field(kms.unwrap_or_default().as_bytes());
    hash.field(&[u8::from(customer.is_some())]);
    if let Some(key) = customer {
        // MD5 is only the public protocol selector. SHA-256 of the actual key
        // separates encryption contexts without persisting the key itself.
        hash.field(key);
    }
    hash.finish()
}

/// Only the opaque digest may be stored in a persistent record. It contains no
/// recoverable credentials, endpoint userinfo or encryption keys.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CacheScope {
    Namespace([u8; 32]),
    Bypass(CacheBypass),
}

impl CacheScope {
    pub fn namespace(self) -> Option<[u8; 32]> {
        match self {
            Self::Namespace(namespace) => Some(namespace),
            Self::Bypass(_) => None,
        }
    }
}
