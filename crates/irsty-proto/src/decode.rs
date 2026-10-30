//! Decoding raw lines into text.
//!
//! IRC has no mandated encoding. Lines that are valid UTF-8 are used as they are;
//! anything else is decoded as Windows-1252, the most common legacy encoding on IRC.
//! Decoding the whole line one way avoids mangling the valid parts of a mixed line.

use std::borrow::Cow;

/// Windows-1252 characters for bytes `0x80..=0x9F`. Unassigned bytes map to their
/// Latin-1 code points (C1 controls), which the sanitiser later neutralises.
const CP1252_HIGH: [char; 32] = [
    '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž', '\u{8F}',
    '\u{90}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '•', '–', '\u{2014}', '˜', '™', 'š',
    '›', 'œ', '\u{9D}', 'ž', 'Ÿ',
];

/// Decodes a line, borrowing when it is valid UTF-8.
pub fn decode_line(bytes: &[u8]) -> Cow<'_, str> {
    match std::str::from_utf8(bytes) {
        Ok(s) => Cow::Borrowed(s),
        Err(_) => Cow::Owned(bytes.iter().map(|&b| cp1252(b)).collect()),
    }
}

fn cp1252(b: u8) -> char {
    match b {
        0x80..=0x9F => CP1252_HIGH
            .get(usize::from(b - 0x80))
            .copied()
            .unwrap_or(char::REPLACEMENT_CHARACTER),
        _ => char::from(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_is_borrowed() {
        assert!(matches!(
            decode_line("héllo ☃".as_bytes()),
            Cow::Borrowed("héllo ☃")
        ));
    }

    #[test]
    fn invalid_utf8_falls_back_to_cp1252() {
        assert_eq!(
            decode_line(b"caf\xe9 \x80 \x93q\x94"),
            "café € \u{201C}q\u{201D}"
        );
        assert_eq!(decode_line(b"\x81\xff"), "\u{81}ÿ");
    }
}
