use std::{
    io::{Read, Write},
    sync::Arc,
};

use crate::{
    budget::Block,
    error::{Error, Result},
};
use foyer::Code;

pub const MAX_IDENTITY_BYTES: usize = 16 * 1024;
pub const MAX_PAYLOAD_BYTES: usize = 128 * 1024;
const KEY_HEADER: usize = 34;
// Foyer header (36), complete bounded key, value envelope (12), payload.
const MAX_ENCODED_RECORD_BYTES: usize = 36 + MAX_IDENTITY_BYTES + 12 + MAX_PAYLOAD_BYTES;
// Foyer uses one limit for raw addr.len on reads AND page-rounded buffer
// admission on writes/reinsertion. The storage envelope must cover both;
// decoded identity/payload bounds remain independently enforced below.
pub(crate) const RECORD_CAP: usize = super::funding::rounded(MAX_ENCODED_RECORD_BYTES);
const KEY_MAGIC: &[u8; 4] = b"GLK1";
const VALUE_MAGIC: &[u8; 4] = b"GLV1";

/// Exact immutable remote identity. Namespace is supplied by the validated caller;
/// this type never derives it from credentials or process-local inode generations.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DiskKey {
    identity: Arc<[u8]>,
    size: u64,
    offset: u64,
    len: u32,
}

impl DiskKey {
    pub fn new(
        namespace: &[u8],
        object: &[u8],
        etag: &[u8],
        size: u64,
        offset: u64,
        len: usize,
    ) -> Result<Self> {
        let identity_len = KEY_HEADER
            .checked_add(namespace.len())
            .and_then(|n| n.checked_add(object.len()))
            .and_then(|n| n.checked_add(etag.len()));
        if namespace.is_empty()
            || object.is_empty()
            || etag.is_empty()
            || identity_len.is_none_or(|n| n > MAX_IDENTITY_BYTES)
            || len == 0
            || len > MAX_PAYLOAD_BYTES
            || offset.checked_add(len as u64).is_none_or(|end| end > size)
        {
            return Err(Error::Config(
                "invalid or oversized disk cache identity/coverage".into(),
            ));
        }
        let mut bytes = Vec::with_capacity(identity_len.unwrap());
        bytes.extend_from_slice(KEY_MAGIC);
        for field in [namespace, object, etag] {
            bytes.extend_from_slice(&(field.len() as u16).to_le_bytes());
        }
        bytes.extend_from_slice(&size.to_le_bytes());
        bytes.extend_from_slice(&offset.to_le_bytes());
        bytes.extend_from_slice(&(len as u32).to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        for field in [namespace, object, etag] {
            bytes.extend_from_slice(field);
        }
        Ok(Self {
            identity: bytes.into(),
            size,
            offset,
            len: len as u32,
        })
    }

    pub fn covered_len(&self) -> usize {
        self.len as usize
    }
    pub fn offset(&self) -> u64 {
        self.offset
    }
    pub fn object_size(&self) -> u64 {
        self.size
    }
}

fn invalid(message: &'static str) -> foyer::Error {
    foyer::Error::new(foyer::ErrorKind::OutOfRange, message)
}
fn end_of_record(reader: &mut impl Read) -> foyer::Result<()> {
    let mut byte = [0];
    if reader.read(&mut byte).map_err(foyer::Error::io_error)? != 0 {
        return Err(invalid("trailing disk record bytes"));
    }
    Ok(())
}

impl Code for DiskKey {
    fn encode(&self, w: &mut impl Write) -> foyer::Result<()> {
        w.write_all(&self.identity).map_err(foyer::Error::io_error)
    }
    fn decode(r: &mut impl Read) -> foyer::Result<Self> {
        let mut header = [0; KEY_HEADER];
        r.read_exact(&mut header).map_err(foyer::Error::io_error)?;
        if &header[..4] != KEY_MAGIC || header[30..34] != [0; 4] {
            return Err(invalid("unsupported disk key format"));
        }
        let lengths =
            [4, 6, 8].map(|i| u16::from_le_bytes(header[i..i + 2].try_into().unwrap()) as usize);
        let total = KEY_HEADER + lengths.iter().sum::<usize>();
        let size = u64::from_le_bytes(header[10..18].try_into().unwrap());
        let offset = u64::from_le_bytes(header[18..26].try_into().unwrap());
        let len = u32::from_le_bytes(header[26..30].try_into().unwrap());
        if total > MAX_IDENTITY_BYTES
            || lengths.contains(&0)
            || len == 0
            || len as usize > MAX_PAYLOAD_BYTES
            || offset.checked_add(len as u64).is_none_or(|end| end > size)
        {
            return Err(invalid("invalid disk key lengths"));
        }
        // No allocation until ALL declared identity/range lengths are bounded.
        let mut bytes = vec![0; total];
        bytes[..KEY_HEADER].copy_from_slice(&header);
        r.read_exact(&mut bytes[KEY_HEADER..])
            .map_err(foyer::Error::io_error)?;
        end_of_record(r)?;
        Ok(Self {
            identity: bytes.into(),
            size,
            offset,
            len,
        })
    }
    fn estimated_size(&self) -> usize {
        self.identity.len()
    }
}

pub(crate) enum DiskValue {
    Source(Block),
    Decoded(Box<[u8]>),
}
impl DiskValue {
    pub(crate) fn bytes(&self) -> &[u8] {
        match self {
            Self::Source(block) => block.as_slice(),
            Self::Decoded(bytes) => bytes,
        }
    }
}
impl Code for DiskValue {
    fn encode(&self, w: &mut impl Write) -> foyer::Result<()> {
        let bytes = self.bytes();
        if bytes.is_empty() || bytes.len() > MAX_PAYLOAD_BYTES {
            return Err(invalid("invalid disk payload length"));
        }
        w.write_all(VALUE_MAGIC)
            .and_then(|()| w.write_all(&(bytes.len() as u32).to_le_bytes()))
            .and_then(|()| w.write_all(&crc32fast::hash(bytes).to_le_bytes()))
            .and_then(|()| w.write_all(bytes))
            .map_err(foyer::Error::io_error)
    }
    fn decode(r: &mut impl Read) -> foyer::Result<Self> {
        let mut header = [0; 12];
        r.read_exact(&mut header).map_err(foyer::Error::io_error)?;
        let len = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
        if &header[..4] != VALUE_MAGIC || len == 0 || len > MAX_PAYLOAD_BYTES {
            return Err(invalid("invalid disk payload envelope"));
        }
        let mut bytes = vec![0; len].into_boxed_slice();
        r.read_exact(&mut bytes).map_err(foyer::Error::io_error)?;
        if crc32fast::hash(&bytes) != u32::from_le_bytes(header[8..12].try_into().unwrap()) {
            return Err(invalid("disk payload CRC mismatch"));
        }
        end_of_record(r)?;
        Ok(Self::Decoded(bytes))
    }
    fn estimated_size(&self) -> usize {
        12 + self.bytes().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::Budget;

    #[test]
    fn exact_identity_cap_and_opaque_fields_roundtrip() {
        let namespace = vec![0xff; MAX_IDENTITY_BYTES - KEY_HEADER - 2];
        let key = DiskKey::new(&namespace, b"\0", b"\xff", 131_079, 131_072, 7).unwrap();
        let mut bytes = vec![];
        key.encode(&mut bytes).unwrap();
        assert_eq!(bytes.len(), MAX_IDENTITY_BYTES);
        assert_eq!(DiskKey::decode(&mut bytes.as_slice()).unwrap(), key);
        assert_eq!(
            (key.offset(), key.object_size(), key.covered_len()),
            (131_072, 131_079, 7)
        );
        assert!(DiskKey::new(&[namespace, vec![0]].concat(), b"k", b"e", 7, 0, 7).is_err());
    }

    #[test]
    fn invalid_lengths_and_versions_are_rejected_before_body_reads() {
        // This finite reader panics on a body read: rejection must happen from
        // the bounded header alone, before decoding or allocating body storage.
        struct Header(std::io::Cursor<Vec<u8>>);
        impl Read for Header {
            fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
                assert!(
                    (self.0.position() as usize) < self.0.get_ref().len(),
                    "body requested before declared-length rejection"
                );
                self.0.read(output)
            }
        }
        let mut key = vec![0; KEY_HEADER];
        key[..4].copy_from_slice(KEY_MAGIC);
        for i in [4, 6, 8] {
            key[i..i + 2].copy_from_slice(&16_000_u16.to_le_bytes());
        }
        key[10..18].copy_from_slice(&1_u64.to_le_bytes());
        key[26..30].copy_from_slice(&1_u32.to_le_bytes());
        assert!(
            matches!(DiskKey::decode(&mut Header(std::io::Cursor::new(key))), Err(e) if e.kind() == foyer::ErrorKind::OutOfRange)
        );
        let mut one_over = vec![0; KEY_HEADER];
        one_over[..4].copy_from_slice(KEY_MAGIC);
        one_over[4..6].copy_from_slice(&16_349_u16.to_le_bytes());
        for i in [6, 8] {
            one_over[i..i + 2].copy_from_slice(&1_u16.to_le_bytes());
        }
        one_over[10..18].copy_from_slice(&131_072_u64.to_le_bytes());
        one_over[26..30].copy_from_slice(&131_072_u32.to_le_bytes());
        assert!(
            matches!(DiskKey::decode(&mut Header(std::io::Cursor::new(one_over))), Err(e) if e.kind() == foyer::ErrorKind::OutOfRange)
        );
        for len in [0, (MAX_PAYLOAD_BYTES + 1) as u32, u32::MAX] {
            let mut value = VALUE_MAGIC.to_vec();
            value.extend_from_slice(&len.to_le_bytes());
            value.extend_from_slice(&0_u32.to_le_bytes());
            assert!(
                matches!(DiskValue::decode(&mut Header(std::io::Cursor::new(value))), Err(e) if e.kind() == foyer::ErrorKind::OutOfRange)
            );
        }
        let key = DiskKey::new(b"n", b"k", b"e", 9, 0, 9).unwrap();
        let mut bytes = vec![];
        key.encode(&mut bytes).unwrap();
        bytes[3] = b'2';
        assert!(DiskKey::decode(&mut bytes.as_slice()).is_err());
    }

    #[test]
    fn ieee_crc_envelope_is_independent_and_corruption_is_rejected() {
        let budget = Budget::new(9).unwrap();
        let value = DiskValue::Source(budget.try_reserve(9).unwrap().copy(b"123456789").unwrap());
        let mut bytes = vec![];
        value.encode(&mut bytes).unwrap();
        assert_eq!(
            &bytes[..12],
            &[b'G', b'L', b'V', b'1', 9, 0, 0, 0, 0x26, 0x39, 0xf4, 0xcb]
        );
        assert_eq!(
            DiskValue::decode(&mut bytes.as_slice()).unwrap().bytes(),
            b"123456789"
        );
        let mut extra = bytes.clone();
        extra.push(0);
        assert!(DiskValue::decode(&mut extra.as_slice()).is_err());
        bytes[12] ^= 1;
        assert!(
            matches!(DiskValue::decode(&mut bytes.as_slice()), Err(e) if e.kind() == foyer::ErrorKind::OutOfRange)
        );
    }
}
