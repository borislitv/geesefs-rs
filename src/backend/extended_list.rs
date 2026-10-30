//! Bounded schema adapter only. HTTP extension signing/fallback is not yet wired.
use std::collections::BTreeMap;

use quick_xml::{Reader, events::Event};
use serde::Deserialize;

use crate::error::{Error, Result};

pub const MAX_PAGE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_PAGE_ENTRIES: usize = 1000;
const MAX_DEPTH: usize = 32;

#[derive(Debug, Deserialize)]
#[serde(rename = "ListBucketResult", rename_all = "PascalCase")]
struct WirePage {
    #[serde(default)]
    contents: Vec<WireObject>,
    #[serde(default)]
    common_prefixes: Vec<WirePrefix>,
    is_truncated: bool,
    next_continuation_token: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct WireObject {
    key: String,
    size: u64,
    #[serde(rename = "ETag")]
    etag: Option<String>,
    last_modified: Option<String>,
    #[serde(default)]
    metadata: Vec<WireMetadata>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct WireMetadata {
    name: String,
    value: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct WirePrefix {
    prefix: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedObject {
    pub key: String,
    pub size: u64,
    pub etag: Option<String>,
    pub last_modified: Option<aws_smithy_types::DateTime>,
    /// None: metadata absent, not authoritative; Some(empty): explicitly known empty.
    pub metadata: Option<BTreeMap<String, Vec<u8>>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListPage {
    pub objects: Vec<ListedObject>,
    pub common_prefixes: Vec<String>,
    pub next_token: Option<String>,
}

fn invalid(message: impl ToString) -> Error {
    Error::InvalidResponse(message.to_string())
}

/// Validate the complete document before publishing any page. Input is capped
/// before parsing; depth/entry bounds also cover ignored forward-compatible fields.
pub(super) fn validate_document(xml: &[u8], root: &str) -> Result<()> {
    if xml.len() > MAX_PAGE_BYTES {
        return Err(invalid("listing page too large"));
    }
    let mut reader = Reader::from_reader(xml);
    let mut depth = 0;
    let mut roots = 0;
    let mut entries = 0;
    loop {
        match reader.read_event().map_err(invalid)? {
            Event::Start(start) => {
                if depth == 0 {
                    roots += 1;
                    if roots != 1 || start.local_name().as_ref() != root {
                        return Err(invalid("unexpected/repeated XML root"));
                    }
                }
                depth += 1;
                if depth > MAX_DEPTH {
                    return Err(invalid("listing nesting limit"));
                }
                if depth == 2
                    && matches!(start.local_name().as_ref(), "Contents" | "CommonPrefixes")
                {
                    entries += 1;
                }
            }
            Event::Empty(start) => {
                if depth == 0 {
                    return Err(invalid("empty XML root"));
                }
                if depth == 1
                    && matches!(start.local_name().as_ref(), "Contents" | "CommonPrefixes")
                {
                    entries += 1;
                }
            }
            Event::End(_) => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| invalid("unmatched end"))?;
            }
            Event::Text(text) if depth == 0 => {
                if !text.as_ref().bytes().all(|b| b.is_ascii_whitespace()) {
                    return Err(invalid("text outside XML root"));
                }
            }
            Event::DocType(_) => return Err(invalid("DTD not allowed")),
            Event::CData(_) | Event::GeneralRef(_) if depth == 0 => {
                return Err(invalid("data outside root"));
            }
            Event::Eof => break,
            _ => {}
        }
        if entries > MAX_PAGE_ENTRIES {
            return Err(invalid("listing entry limit"));
        }
    }
    if roots != 1 || depth != 0 {
        return Err(invalid("incomplete XML document"));
    }
    Ok(())
}

pub fn decode_page(xml: &[u8], prefix: &str, previous_token: Option<&str>) -> Result<ListPage> {
    validate_document(xml, "ListBucketResult")?;
    let wire: WirePage = quick_xml::de::from_reader(xml).map_err(invalid)?;
    let next_token = if wire.is_truncated {
        match wire.next_continuation_token {
            Some(token) if !token.is_empty() && Some(token.as_str()) != previous_token => {
                Some(token)
            }
            _ => {
                return Err(invalid(
                    "truncated listing has missing or repeated continuation token",
                ));
            }
        }
    } else {
        None
    };
    let mut objects = Vec::with_capacity(wire.contents.len());
    let mut last_key: Option<String> = None;
    for object in wire.contents {
        if !object.key.starts_with(prefix)
            || object.key.is_empty()
            || last_key.as_ref().is_some_and(|last| *last >= object.key)
        {
            return Err(invalid("unconfined or unordered object key"));
        }
        last_key = Some(object.key.clone());
        // ext-v1 is authoritative even when it has no Metadata elements. Standard
        // LIST, in contrast, leaves metadata unknown until HEAD.
        let metadata = Some(super::metadata::decode_metadata(
            object
                .metadata
                .iter()
                .map(|m| (m.name.as_str(), m.value.as_str())),
        ));
        let last_modified = object
            .last_modified
            .map(|s| {
                aws_smithy_types::DateTime::from_str(
                    &s,
                    aws_smithy_types::date_time::Format::DateTime,
                )
                .map_err(invalid)
            })
            .transpose()?;
        objects.push(ListedObject {
            key: object.key,
            size: object.size,
            etag: object.etag,
            last_modified,
            metadata,
        });
    }
    let mut prefixes = Vec::with_capacity(wire.common_prefixes.len());
    for common in wire.common_prefixes {
        if !common.prefix.starts_with(prefix)
            || !common.prefix.ends_with('/')
            || common.prefix == prefix
        {
            return Err(invalid("unconfined common prefix"));
        }
        prefixes.push(common.prefix);
    }
    Ok(ListPage {
        objects,
        common_prefixes: prefixes,
        next_token,
    })
}

/// Match modeled extension errors, never status alone or an arbitrary message.
pub fn permits_standard_fallback(code: &str) -> bool {
    matches!(code, "InvalidArgument" | "NotImplemented")
}
