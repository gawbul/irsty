//! IRCv3 message tags: escaping, iteration over raw tag data, and an owned map.
//!
//! A tag without a value and a tag with an empty value are equivalent, so values are
//! plain strings and the empty string means "no value". When a key repeats, the last
//! occurrence wins.

use std::borrow::Cow;

/// Iterates over raw `key[=value]` pairs in tag data (without the leading `@`).
///
/// Values are still escaped; use [`unescape`] on them. Empty segments are skipped and
/// duplicates are yielded as they appear.
#[derive(Clone, Debug)]
pub struct RawTags<'a> {
    rest: Option<&'a str>,
}

impl<'a> RawTags<'a> {
    pub(crate) fn new(raw: &'a str) -> Self {
        Self { rest: Some(raw) }
    }
}

impl<'a> Iterator for RawTags<'a> {
    type Item = (&'a str, &'a str);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let rest = self.rest?;
            let (segment, tail) = match rest.split_once(';') {
                Some((segment, tail)) => (segment, Some(tail)),
                None => (rest, None),
            };
            self.rest = tail;
            if segment.is_empty() {
                continue;
            }
            return Some(segment.split_once('=').unwrap_or((segment, "")));
        }
    }
}

/// Unescapes a tag value. Unknown escapes drop the backslash, and a trailing lone
/// backslash is removed.
pub fn unescape(value: &str) -> Cow<'_, str> {
    if !value.contains('\\') {
        return Cow::Borrowed(value);
    }
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some(':') => out.push(';'),
            Some('s') => out.push(' '),
            Some('r') => out.push('\r'),
            Some('n') => out.push('\n'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    Cow::Owned(out)
}

/// Appends `value` to `out` with tag escaping applied.
pub fn escape_into(value: &str, out: &mut String) {
    for c in value.chars() {
        match c {
            ';' => out.push_str("\\:"),
            ' ' => out.push_str("\\s"),
            '\\' => out.push_str("\\\\"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
}

/// Reports whether `key` is a valid tag key: an optional `+` (client-only), an
/// optional vendor prefix ending in `/`, then a non-empty name of ASCII letters,
/// digits and hyphens.
pub fn is_valid_key(key: &str) -> bool {
    let key = key.strip_prefix('+').unwrap_or(key);
    let (vendor, name) = match key.rsplit_once('/') {
        Some((vendor, name)) => (Some(vendor), name),
        None => (None, key),
    };
    let vendor_ok = vendor.is_none_or(|v| {
        !v.is_empty()
            && v.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
    });
    vendor_ok && !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// An owned, ordered set of tags with unescaped values. Inserting an existing key
/// replaces its value in place.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tags(Vec<(String, String)>);

impl Tags {
    /// Creates an empty tag set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds a tag set from raw tag data, unescaping values and keeping the last
    /// occurrence of each key.
    pub fn from_raw(raw: &str) -> Self {
        let mut tags = Self::new();
        for (key, value) in RawTags::new(raw) {
            tags.insert(key, unescape(value));
        }
        tags
    }

    /// Inserts or replaces a tag. An empty value means the tag has no value.
    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<String>) {
        let key = key.into();
        let value = value.into();
        match self.0.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = value,
            None => self.0.push((key, value)),
        }
    }

    /// Returns the value of `key`, if present.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Removes `key`, returning its value if it was present.
    pub fn remove(&mut self, key: &str) -> Option<String> {
        let pos = self.0.iter().position(|(k, _)| k == key)?;
        Some(self.0.remove(pos).1)
    }

    /// Iterates over `(key, value)` pairs in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Returns the number of tags.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Reports whether there are no tags.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Appends the escaped tag data (without `@` or the trailing space) to `out`.
    /// Tags without a value are written as a bare key.
    pub(crate) fn write_into(&self, out: &mut String) {
        for (i, (key, value)) in self.0.iter().enumerate() {
            if i > 0 {
                out.push(';');
            }
            out.push_str(key);
            if !value.is_empty() {
                out.push('=');
                escape_into(value, out);
            }
        }
    }
}

impl<K: Into<String>, V: Into<String>> FromIterator<(K, V)> for Tags {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        let mut tags = Self::new();
        for (k, v) in iter {
            tags.insert(k, v);
        }
        tags
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unescape_handles_all_escapes() {
        assert_eq!(unescape(r"a\:b\sc\\d\re\nf"), "a;b c\\d\re\nf");
        assert_eq!(unescape(r"value\1"), "value1");
        assert_eq!(unescape("value\\"), "value");
        assert!(matches!(unescape("plain"), Cow::Borrowed("plain")));
    }

    #[test]
    fn escape_round_trips() {
        let original = "semi;colon space\\back\r\nline";
        let mut escaped = String::new();
        escape_into(original, &mut escaped);
        assert!(!escaped.contains([';', ' ', '\r', '\n']));
        assert_eq!(unescape(&escaped), original);
    }

    #[test]
    fn raw_iteration_skips_empty_segments() {
        let tags: Vec<_> = RawTags::new(";a=1;;b;c=").collect();
        assert_eq!(tags, [("a", "1"), ("b", ""), ("c", "")]);
    }

    #[test]
    fn last_duplicate_wins_in_place() {
        let tags = Tags::from_raw("a=1;b=2;a=3");
        assert_eq!(tags.iter().collect::<Vec<_>>(), [("a", "3"), ("b", "2")]);
    }

    #[test]
    fn tags_map_operations() {
        let mut tags: Tags = [("a", "1")].into_iter().collect();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags.get("a"), Some("1"));
        assert_eq!(tags.remove("a"), Some("1".to_owned()));
        assert_eq!(tags.remove("a"), None);
        assert!(tags.is_empty());
    }

    #[test]
    fn key_validation() {
        for ok in ["msgid", "+typing", "draft/reply", "+example.com/x-y", "a1"] {
            assert!(is_valid_key(ok), "{ok}");
        }
        for bad in ["", "+", "/x", "a b", "a=b", "a;b", "vendor/", "a_b", "é"] {
            assert!(!is_valid_key(bad), "{bad}");
        }
    }
}
