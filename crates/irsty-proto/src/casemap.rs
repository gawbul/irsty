//! Casemapping, as advertised in `RPL_ISUPPORT` `CASEMAPPING`.

use std::borrow::Cow;

/// How a network compares nicks and channel names.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CaseMapping {
    /// Only `A-Z` fold to `a-z`.
    Ascii,
    /// `ascii` plus `[]\~` fold to `{}|^`. The RFC default when nothing is advertised.
    #[default]
    Rfc1459,
    /// `ascii` plus `[]\` fold to `{}|` (no `~`).
    Rfc1459Strict,
    /// Unicode-aware networks (`rfc8265`, `precis`, `rfc7613`). Approximated with
    /// Unicode lowercasing; servers normalise further, but this is enough to
    /// match what they send back.
    Unicode,
}

impl CaseMapping {
    /// Parses an `RPL_ISUPPORT` `CASEMAPPING` value. Unknown values return `None`;
    /// callers should keep the current mapping.
    pub fn from_token(token: &str) -> Option<Self> {
        match token.to_ascii_lowercase().as_str() {
            "ascii" => Some(Self::Ascii),
            "rfc1459" => Some(Self::Rfc1459),
            "rfc1459-strict" | "strict-rfc1459" => Some(Self::Rfc1459Strict),
            "rfc8265" | "rfc7613" | "precis" => Some(Self::Unicode),
            _ => None,
        }
    }

    /// Folds a single character. For [`CaseMapping::Unicode`] this only handles
    /// characters whose lowercase form is a single character.
    fn fold_char(self, c: char) -> char {
        match (self, c) {
            (_, 'A'..='Z') => c.to_ascii_lowercase(),
            (Self::Rfc1459 | Self::Rfc1459Strict, '[') => '{',
            (Self::Rfc1459 | Self::Rfc1459Strict, ']') => '}',
            (Self::Rfc1459 | Self::Rfc1459Strict, '\\') => '|',
            (Self::Rfc1459, '~') => '^',
            (Self::Unicode, c) if !c.is_ascii() => {
                let mut lower = c.to_lowercase();
                match (lower.next(), lower.next()) {
                    (Some(l), None) => l,
                    _ => c,
                }
            }
            _ => c,
        }
    }

    /// Folds a string, borrowing when it is already folded.
    pub fn fold(self, s: &str) -> Cow<'_, str> {
        if self == Self::Unicode {
            // Full lowercasing handles multi-character expansions such as İ.
            let unchanged = s.chars().all(|c| {
                let mut lower = c.to_lowercase();
                lower.next() == Some(c) && lower.next().is_none()
            });
            return if unchanged {
                Cow::Borrowed(s)
            } else {
                Cow::Owned(s.to_lowercase())
            };
        }
        if s.chars().all(|c| self.fold_char(c) == c) {
            return Cow::Borrowed(s);
        }
        Cow::Owned(s.chars().map(|c| self.fold_char(c)).collect())
    }

    /// Compares two names under this mapping.
    pub fn eq(self, a: &str, b: &str) -> bool {
        self.fold(a) == self.fold(b)
    }

    /// Folds one character for wildcard matching.
    pub(crate) fn fold_for_match(self, c: char) -> char {
        self.fold_char(c)
    }
}

/// A name folded under a casemapping, for use as a map key. Two keys are equal
/// exactly when the names are equal under the mapping they were built with.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CaseKey(String);

impl CaseKey {
    /// Folds `name` under `mapping`.
    pub fn new(mapping: CaseMapping, name: &str) -> Self {
        Self(mapping.fold(name).into_owned())
    }

    /// Returns the folded form.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc1459_folds_brackets_and_tilde() {
        let m = CaseMapping::Rfc1459;
        assert_eq!(m.fold("Nick[A]\\~"), "nick{a}|^");
        assert!(m.eq("FOO[]", "foo{}"));
    }

    #[test]
    fn strict_does_not_fold_tilde() {
        assert_eq!(CaseMapping::Rfc1459Strict.fold("A~[]"), "a~{}");
    }

    #[test]
    fn ascii_only_folds_letters() {
        assert_eq!(CaseMapping::Ascii.fold("AB[]~"), "ab[]~");
        assert!(matches!(CaseMapping::Ascii.fold("abc"), Cow::Borrowed(_)));
    }

    #[test]
    fn unicode_lowercases() {
        assert!(CaseMapping::Unicode.eq("ÉCOLE", "école"));
        assert_eq!(CaseMapping::Unicode.fold("İ"), "i\u{307}");
    }

    #[test]
    fn tokens() {
        assert_eq!(CaseMapping::from_token("ASCII"), Some(CaseMapping::Ascii));
        assert_eq!(
            CaseMapping::from_token("rfc1459-strict"),
            Some(CaseMapping::Rfc1459Strict)
        );
        assert_eq!(
            CaseMapping::from_token("rfc8265"),
            Some(CaseMapping::Unicode)
        );
        assert_eq!(
            CaseMapping::from_token("rfc1459"),
            Some(CaseMapping::Rfc1459)
        );
        assert_eq!(CaseMapping::from_token("bogus"), None);
    }

    #[test]
    fn keys_compare_folded() {
        let m = CaseMapping::Rfc1459;
        assert_eq!(CaseKey::new(m, "#Chan[1]"), CaseKey::new(m, "#chan{1}"));
        assert_eq!(CaseKey::new(m, "ABC").as_str(), "abc");
    }
}
