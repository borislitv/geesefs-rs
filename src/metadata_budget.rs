//! Application metadata admission, not an allocator/RSS limit.
//!
//! Charges include fixed owner/index allowances and variable owned capacities.
//! SDK/parser allocations, allocator rounding and container spare nodes remain
//! external overhead. A charge follows its owner, not its cache membership.
use std::{collections::BTreeMap, mem::size_of};

use crate::{
    backend::s3::ObjectHead,
    budget::{Budget, Reservation},
    error::{Error, Result},
};

// Logical allowance for tree/hash links, separate from the stored key/value.
// This is intentionally not described as jemalloc usable size.
pub(crate) const LINKS: usize = size_of::<[usize; 8]>();

#[derive(Clone, Default)]
pub struct MetadataBudget(Option<Budget>);

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct MetadataUsage {
    pub limit: Option<usize>,
    pub charged: usize,
}

pub struct MetadataCharge {
    budget: MetadataBudget,
    reservation: Option<Reservation>,
}

impl std::fmt::Debug for MetadataCharge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetadataCharge")
            .field("bytes", &self.bytes())
            .finish()
    }
}

impl MetadataBudget {
    pub(crate) fn enabled(&self) -> bool {
        self.0.is_some()
    }
    pub(crate) fn estimate(&self, bytes: impl FnOnce() -> usize) -> usize {
        if self.enabled() { bytes() } else { 0 }
    }
    pub(crate) fn charge_with(&self, bytes: impl FnOnce() -> usize) -> Result<MetadataCharge> {
        self.charge(self.estimate(bytes))
    }
    pub fn new(limit: Option<usize>) -> Result<Self> {
        Ok(Self(limit.map(Budget::new).transpose()?))
    }

    pub fn usage(&self) -> MetadataUsage {
        MetadataUsage {
            limit: self.0.as_ref().map(Budget::limit),
            charged: self.0.as_ref().map_or(0, Budget::used),
        }
    }

    pub(crate) fn charge(&self, bytes: usize) -> Result<MetadataCharge> {
        let reservation = self
            .0
            .as_ref()
            .map(|budget| {
                budget
                    .try_reserve(bytes)
                    .map_err(|_| Error::Fs(libc::ENOMEM))
            })
            .transpose()?;
        Ok(MetadataCharge {
            budget: self.clone(),
            reservation,
        })
    }
}

/// Immutable application-owned result. Moving the result also moves its permit;
/// only borrowed access to the allocation is exposed.
#[derive(Debug)]
pub struct MetadataOwner<T> {
    value: T,
    _charge: MetadataCharge,
}
impl<T> MetadataOwner<T> {
    pub(crate) fn new(value: T, charge: MetadataCharge) -> Self {
        Self {
            value,
            _charge: charge,
        }
    }
}
impl<T> std::ops::Deref for MetadataOwner<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}
impl<T: PartialEq<R>, R: ?Sized> PartialEq<R> for MetadataOwner<T> {
    fn eq(&self, other: &R) -> bool {
        self.value.eq(other)
    }
}
#[derive(Debug)]
pub struct MetadataString {
    value: String,
    _charge: MetadataCharge,
}
impl MetadataString {
    pub(crate) fn new(value: String, charge: MetadataCharge) -> Self {
        Self {
            value,
            _charge: charge,
        }
    }
    pub fn as_str(&self) -> &str {
        &self.value
    }
}
impl std::ops::Deref for MetadataString {
    type Target = str;
    fn deref(&self) -> &str {
        &self.value
    }
}
impl AsRef<str> for MetadataString {
    fn as_ref(&self) -> &str {
        &self.value
    }
}
impl<R: AsRef<str>> PartialEq<R> for MetadataString {
    fn eq(&self, other: &R) -> bool {
        self.value == other.as_ref()
    }
}
impl Default for MetadataString {
    fn default() -> Self {
        Self::new(
            String::new(),
            MetadataBudget::default()
                .charge(0)
                .expect("disabled budget"),
        )
    }
}

pub struct Xattrs {
    values: BTreeMap<String, Vec<u8>>,
    _charge: MetadataCharge,
}
impl Xattrs {
    pub(crate) fn new(values: BTreeMap<String, Vec<u8>>, charge: MetadataCharge) -> Self {
        Self {
            values,
            _charge: charge,
        }
    }
    pub fn get(&self, key: &str) -> Option<&[u8]> {
        self.values.get(key).map(Vec::as_slice)
    }
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.values.keys().map(String::as_str)
    }
    pub fn contains_key(&self, key: &str) -> bool {
        self.values.contains_key(key)
    }
}
impl std::ops::Index<&str> for Xattrs {
    type Output = [u8];
    fn index(&self, key: &str) -> &[u8] {
        &self.values[key]
    }
}

/// Conservative peak bound for percent-encoding, Unicode lowercasing, tree
/// intermediates and the final hash table. Each input byte emits at most three
/// bytes; geometrically grown strings use less than twice that capacity.
/// Escaped non-ASCII output is ASCII, so escaping and Unicode lowercasing cannot
/// compound their expansion: both key buffers together fit below 12 * key.len(),
/// and the value buffer below 6 * value.len(). Sixteen also covers empty-string
/// minimum capacities; entry allowances cover both maps during transfer.
pub(crate) fn encoding_bytes(map: &BTreeMap<String, Vec<u8>>) -> usize {
    map.iter()
        .map(|(key, value)| {
            16 * (key.len() + value.len() + 16) + 8 * (size_of::<(String, String)>() + LINKS)
        })
        .sum()
}

impl MetadataCharge {
    pub(crate) fn take(&mut self, bytes: usize) -> Result<Self> {
        let reservation = match self.reservation.as_mut() {
            None => None,
            Some(old) if old.size() >= bytes => Some(old.split(bytes)),
            Some(_) => return Err(Error::Fs(libc::ENOMEM)),
        };
        Ok(Self {
            budget: self.budget.clone(),
            reservation,
        })
    }
    pub(crate) fn ensure_from(&mut self, bytes: usize, credit: &mut Self) -> Result<()> {
        if self.reservation.is_none() || bytes <= self.bytes() {
            return Ok(());
        }
        let extra = credit.take(bytes - self.bytes())?;
        self.reservation
            .as_mut()
            .unwrap()
            .merge(extra.reservation.unwrap());
        Ok(())
    }
    pub(crate) fn budget(&self) -> &MetadataBudget {
        &self.budget
    }
    pub(crate) fn bytes(&self) -> usize {
        self.reservation.as_ref().map_or(0, Reservation::size)
    }
    pub(crate) fn resize(&mut self, bytes: usize) -> Result<()> {
        let Some(old) = self.reservation.as_mut() else {
            return Ok(());
        };
        if bytes > old.size() {
            let more = self.budget.charge(bytes - old.size())?;
            old.merge(more.reservation.expect("enabled metadata budget"));
        } else {
            drop(old.split(old.size() - bytes));
        }
        Ok(())
    }
}

pub(crate) fn map_bytes(map: &BTreeMap<String, Vec<u8>>) -> usize {
    map.iter()
        .map(|(key, value)| {
            size_of::<(String, Vec<u8>)>() + LINKS + key.capacity() + value.capacity()
        })
        .sum()
}

pub(crate) fn head_bytes(head: &ObjectHead) -> usize {
    size_of::<ObjectHead>()
        + head.etag.capacity()
        + head.metadata.capacity() * (size_of::<(String, String)>() + LINKS)
        + head
            .metadata
            .iter()
            .map(|(key, value)| key.capacity() + value.capacity())
            .sum::<usize>()
        + [
            &head.properties.content_type,
            &head.properties.content_encoding,
            &head.properties.content_disposition,
            &head.properties.content_language,
            &head.properties.cache_control,
            &head.properties.website_redirect_location,
        ]
        .into_iter()
        .flatten()
        .map(String::capacity)
        .sum::<usize>()
}

pub(crate) fn page_bytes(page: &crate::backend::extended_list::ListPage) -> usize {
    use crate::backend::extended_list::ListedObject;
    size_of::<crate::backend::extended_list::ListPage>()
        + page.objects.capacity() * size_of::<ListedObject>()
        + page
            .objects
            .iter()
            .map(|object| {
                object.key.capacity()
                    + object.etag.as_ref().map_or(0, String::capacity)
                    + object.metadata.as_ref().map_or(0, map_bytes)
            })
            .sum::<usize>()
        + page.common_prefixes.capacity() * size_of::<String>()
        + page
            .common_prefixes
            .iter()
            .map(String::capacity)
            .sum::<usize>()
        + page.next_token.as_ref().map_or(0, String::capacity)
}
