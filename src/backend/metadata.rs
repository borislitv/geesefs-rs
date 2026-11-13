use std::collections::BTreeMap;

use percent_encoding::percent_decode_str;

use crate::{
    error::Result,
    metadata_budget::{MetadataBudget, MetadataCharge, MetadataOwner},
};

type Attribute = (String, Vec<u8>);

/// Canonical public attributes. Clones share both storage and admission; callers
/// can only borrow the backing, so its permit cannot be separated from its data.
#[derive(Clone, Debug)]
pub struct CachedMetadata(std::sync::Arc<MetadataOwner<Box<[Attribute]>>>);

impl CachedMetadata {
    pub fn new(values: BTreeMap<String, Vec<u8>>, budget: &MetadataBudget) -> Result<Self> {
        let mut charge = budget.charge_with(|| Self::estimate(&values))?;
        Self::from_admitted(values, &mut charge)
    }

    pub(crate) fn from_admitted(
        values: BTreeMap<String, Vec<u8>>,
        credit: &mut MetadataCharge,
    ) -> Result<Self> {
        let charge = credit.take(credit.budget().estimate(|| Self::estimate(&values)))?;
        Self::from_entries(values.into_iter().collect(), charge)
    }

    fn estimate(values: &BTreeMap<String, Vec<u8>>) -> usize {
        Self::overhead()
            + values.len() * std::mem::size_of::<Attribute>()
            + values
                .iter()
                .map(|(k, v)| k.capacity() + v.capacity())
                .sum::<usize>()
    }

    fn overhead() -> usize {
        std::mem::size_of::<MetadataOwner<Box<[Attribute]>>>() + 2 * std::mem::size_of::<usize>()
    }

    fn from_entries(mut values: Vec<Attribute>, mut charge: MetadataCharge) -> Result<Self> {
        for (key, value) in &mut values {
            key.shrink_to_fit();
            value.shrink_to_fit();
        }
        let values = values.into_boxed_slice();
        let bytes = charge.budget().estimate(|| {
            Self::overhead()
                + std::mem::size_of_val(&*values)
                + values
                    .iter()
                    .map(|(k, v)| k.capacity() + v.capacity())
                    .sum::<usize>()
        });
        // All admission precedes the allocation. Settlement may only release.
        if bytes > charge.bytes() {
            return Err(crate::error::Error::Fs(libc::ENOMEM));
        }
        charge.resize(bytes)?;
        Ok(Self(std::sync::Arc::new(MetadataOwner::new(
            values, charge,
        ))))
    }

    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.0
            .binary_search_by(|(key, _)| key.as_str().cmp(name))
            .ok()
            .map(|index| self.0[index].1.as_slice())
    }

    pub fn contains_key(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&str, &[u8])> {
        self.0
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_slice()))
    }

    pub fn keys(&self) -> impl ExactSizeIterator<Item = &str> {
        self.0.iter().map(|(key, _)| key.as_str())
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn encode(&self) -> Result<BTreeMap<String, String>> {
        encode_pairs(self.iter())
    }

    pub(crate) fn encoding_bytes(&self) -> usize {
        self.iter()
            .map(|(key, value)| {
                16 * (key.len() + value.len() + 16)
                    + 8 * (std::mem::size_of::<(String, String)>() + crate::metadata_budget::LINKS)
            })
            .sum()
    }

    pub(crate) fn map_bytes(&self) -> usize {
        self.iter()
            .map(|(key, value)| {
                std::mem::size_of::<Attribute>()
                    + crate::metadata_budget::LINKS
                    + key.len()
                    + value.len()
            })
            .sum()
    }

    /// Create the next version only after admitting its complete physical backing.
    /// A failed admission leaves this version and every snapshot untouched.
    pub fn try_insert(&mut self, key: &str, value: &[u8]) -> Result<()> {
        if self.get(key) == Some(value) {
            return Ok(());
        }
        let charge = self.0.charge().budget().charge_with(|| {
            self.backing_bytes() + std::mem::size_of::<Attribute>() + key.len() + value.len()
        })?;
        let at = self.0.binary_search_by(|(name, _)| name.as_str().cmp(key));
        let mut entries = Vec::with_capacity(self.len() + usize::from(at.is_err()));
        for (index, (name, bytes)) in self.0.iter().enumerate() {
            if at != Ok(index) {
                entries.push((name.clone(), bytes.clone()));
            }
        }
        entries.insert(
            at.unwrap_or_else(|index| index),
            (key.to_owned(), value.to_vec()),
        );
        *self = Self::from_entries(entries, charge)?;
        Ok(())
    }

    pub fn try_remove(&mut self, key: &str) -> Result<()> {
        let Ok(at) = self.0.binary_search_by(|(name, _)| name.as_str().cmp(key)) else {
            return Ok(());
        };
        let charge = self
            .0
            .charge()
            .budget()
            .charge_with(|| self.backing_bytes())?;
        let mut entries = Vec::with_capacity(self.len() - 1);
        entries.extend(
            self.0
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != at)
                .map(|(_, entry)| entry.clone()),
        );
        *self = Self::from_entries(entries, charge)?;
        Ok(())
    }

    fn backing_bytes(&self) -> usize {
        Self::overhead()
            + self.len() * std::mem::size_of::<Attribute>()
            + self
                .0
                .iter()
                .map(|(key, value)| key.capacity() + value.capacity())
                .sum::<usize>()
    }
}

impl PartialEq for CachedMetadata {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.0, &other.0) || **self.0 == **other.0
    }
}
impl Eq for CachedMetadata {}

impl std::ops::Index<&str> for CachedMetadata {
    type Output = [u8];
    fn index(&self, key: &str) -> &[u8] {
        self.get(key).expect("metadata key exists")
    }
}

/// The committed generation retained by inodes and snapshots. HTTP properties
/// belong to transient backend HEAD/COPY/REPLACE handling, not every cached file.
#[derive(Clone, Debug)]
pub struct CachedRemote(std::sync::Arc<MetadataOwner<RemoteRecord>>);

#[derive(Debug)]
pub struct RemoteRecord {
    pub size: u64,
    pub etag: String,
    pub metadata: CachedMetadata,
}

impl std::ops::Deref for CachedRemote {
    type Target = RemoteRecord;
    fn deref(&self) -> &RemoteRecord {
        &self.0
    }
}

impl CachedRemote {
    pub(crate) fn with_generation(&self, size: u64, etag: String) -> Result<Self> {
        let mut credit = self.0.charge().budget().charge_with(|| {
            std::mem::size_of::<MetadataOwner<RemoteRecord>>()
                + 2 * std::mem::size_of::<usize>()
                + etag.capacity()
        })?;
        self.with_generation_from(size, etag, &mut credit)
    }

    pub(crate) fn with_generation_from(
        &self,
        size: u64,
        etag: String,
        credit: &mut MetadataCharge,
    ) -> Result<Self> {
        Self::from_admitted(
            super::s3::ObjectHead {
                size,
                etag,
                ..Default::default()
            },
            self.metadata.clone(),
            credit,
        )
    }
    pub(crate) fn decode_admitted(
        head: super::s3::ObjectHead,
        current: &CachedMetadata,
        credit: &mut MetadataCharge,
    ) -> Result<Self> {
        let decoded = decode_metadata(head.metadata.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        let metadata = if current
            .iter()
            .eq(decoded.iter().map(|(k, v)| (k.as_str(), v.as_slice())))
        {
            current.clone()
        } else {
            CachedMetadata::from_admitted(decoded, credit)?
        };
        Self::from_admitted(head, metadata, credit)
    }

    pub(crate) fn from_admitted(
        head: super::s3::ObjectHead,
        metadata: CachedMetadata,
        credit: &mut MetadataCharge,
    ) -> Result<Self> {
        let charge = credit.take(credit.budget().estimate(|| {
            std::mem::size_of::<MetadataOwner<RemoteRecord>>()
                + 2 * std::mem::size_of::<usize>()
                + head.etag.capacity()
        }))?;
        Ok(Self(std::sync::Arc::new(MetadataOwner::new(
            RemoteRecord {
                size: head.size,
                etag: head.etag,
                metadata,
            },
            charge,
        ))))
    }

    /// A conditional read needs no attribute encoding or HTTP property copies.
    pub(crate) fn read_head(&self) -> super::s3::ObjectHead {
        super::s3::ObjectHead {
            size: self.size,
            etag: self.etag.clone(),
            ..Default::default()
        }
    }

    pub(crate) fn bytes(&self) -> usize {
        self.0.charge().bytes()
    }
}

/// None means no authoritative metadata has been fetched; Some(empty) is known empty.
pub type Metadata = Option<BTreeMap<String, Vec<u8>>>;

/// Private upload provenance, not a filesystem attribute. S3 stores this with
/// PUT/multipart metadata and returns it in HEAD, including after a lost reply.
pub(crate) const UPLOAD_RECEIPT: &str = "geesefs-upload-receipt";

fn valid_escapes(s: &str) -> bool {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len()
                || !bytes[i + 1].is_ascii_hexdigit()
                || !bytes[i + 2].is_ascii_hexdigit()
            {
                return false;
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    true
}

/// Reference handles.go drops malformed pairs, lowercases encoded names before
/// decoding, and preserves literal '+'. Values may contain non-UTF8 bytes.
pub fn decode_metadata<'a>(
    pairs: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> BTreeMap<String, Vec<u8>> {
    let mut decoded = BTreeMap::new();
    for (name, value) in pairs {
        let name = name.to_lowercase();
        if !valid_escapes(&name) || !valid_escapes(value) {
            continue;
        }
        // Rust strings cannot represent arbitrary byte keys; these must not be
        // silently substituted with U+FFFD. Non-UTF8 key support is an open gap.
        if let Ok(key) = percent_decode_str(&name).decode_utf8() {
            if key == UPLOAD_RECEIPT {
                continue;
            }
            decoded.insert(key.into_owned(), percent_decode_str(value).collect());
        }
    }
    decoded
}

/// Go ranges over UTF-8 runes, replacing each invalid byte with RuneError. IsPrint
/// permits L/M/N/P/S categories and ASCII space, but not separators/format controls.
pub fn encode_metadata(
    meta: &BTreeMap<String, Vec<u8>>,
) -> crate::error::Result<BTreeMap<String, String>> {
    encode_pairs(
        meta.iter()
            .map(|(key, value)| (key.as_str(), value.as_slice())),
    )
}

fn encode_pairs<'a>(
    pairs: impl IntoIterator<Item = (&'a str, &'a [u8])>,
) -> Result<BTreeMap<String, String>> {
    fn escape(bytes: &[u8]) -> crate::error::Result<String> {
        let mut result = String::new();
        let mut rune = |c: char| {
            use unicode_general_category::{GeneralCategory as G, get_general_category};
            let printable = c == ' '
                || matches!(
                    get_general_category(c),
                    G::UppercaseLetter
                        | G::LowercaseLetter
                        | G::TitlecaseLetter
                        | G::ModifierLetter
                        | G::OtherLetter
                        | G::NonspacingMark
                        | G::SpacingMark
                        | G::EnclosingMark
                        | G::DecimalNumber
                        | G::LetterNumber
                        | G::OtherNumber
                        | G::ConnectorPunctuation
                        | G::DashPunctuation
                        | G::OpenPunctuation
                        | G::ClosePunctuation
                        | G::InitialPunctuation
                        | G::FinalPunctuation
                        | G::OtherPunctuation
                        | G::MathSymbol
                        | G::CurrencySymbol
                        | G::ModifierSymbol
                        | G::OtherSymbol
                );
            if c == '%' {
                result.push_str("%25");
            } else if printable {
                result.push(c);
            } else {
                use std::fmt::Write;
                write!(result, "%{:02X}", c as u32).expect("String formatting cannot fail");
            }
        };
        for chunk in bytes.utf8_chunks() {
            for c in chunk.valid().chars() {
                rune(c);
            }
            for _ in chunk.invalid() {
                rune('\u{fffd}');
            }
        }
        Ok(result)
    }
    pairs
        .into_iter()
        .map(|(k, v)| Ok((escape(k.as_bytes())?.to_lowercase(), escape(v)?)))
        .collect()
}
