use std::collections::BTreeMap;

use percent_encoding::percent_decode_str;

/// None means no authoritative metadata has been fetched; Some(empty) is known empty.
pub type Metadata = Option<BTreeMap<String, Vec<u8>>>;

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
    meta.iter()
        .map(|(k, v)| Ok((escape(k.as_bytes())?.to_lowercase(), escape(v)?)))
        .collect()
}
