//! Parsing and serialising IRC messages.
//!
//! [`MessageRef`] borrows from the line and allocates nothing; [`Message`] is the
//! owned form for passing between tasks and for building outgoing messages.

use std::borrow::Cow;

use crate::source::Source;
use crate::tags::{self, RawTags, Tags};
use crate::{MAX_BODY_LEN, MAX_CLIENT_TAGS_LEN};

/// Maximum number of parameters in a message (RFC 2812).
pub const MAX_PARAMS: usize = 15;

/// Why a line could not be parsed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    /// The line was empty or contained only whitespace.
    #[error("empty line")]
    Empty,
    /// The line contained a NUL, CR or LF before its end.
    #[error("line contains a NUL, CR or LF")]
    IllegalCharacter,
    /// Tags or a source were present but no command followed.
    #[error("missing command")]
    MissingCommand,
    /// A `:` was not followed by a source.
    #[error("empty source")]
    EmptySource,
}

/// Why a message could not be serialised.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SerialiseError {
    /// The command was empty or not made of ASCII letters and digits.
    #[error("invalid command {0:?}")]
    InvalidCommand(String),
    /// The source was empty or contained a space, NUL, CR or LF.
    #[error("invalid source")]
    InvalidSource,
    /// A tag key was not valid.
    #[error("invalid tag key {0:?}")]
    InvalidTagKey(String),
    /// A parameter contained NUL, CR or LF.
    #[error("parameter {0} contains a NUL, CR or LF")]
    IllegalCharacter(usize),
    /// A parameter other than the last was empty, contained a space, or started
    /// with `:`.
    #[error("parameter {0} must be non-empty, without spaces, and not start with ':'")]
    InvalidMiddleParam(usize),
    /// More than [`MAX_PARAMS`] parameters.
    #[error("too many parameters ({0})")]
    TooManyParams(usize),
    /// The body (source, command and parameters) exceeded [`MAX_BODY_LEN`].
    #[error("message body is {len} bytes, the limit is {MAX_BODY_LEN}")]
    BodyTooLong {
        /// Length of the body in bytes.
        len: usize,
    },
    /// The tag data exceeded [`MAX_CLIENT_TAGS_LEN`].
    #[error("tag data is {len} bytes, the limit is {MAX_CLIENT_TAGS_LEN}")]
    TagsTooLong {
        /// Length of the tag data in bytes.
        len: usize,
    },
}

/// A message borrowing from its line. Cheap to copy.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct MessageRef<'a> {
    tags: Option<&'a str>,
    source: Option<&'a str>,
    command: &'a str,
    params: [&'a str; MAX_PARAMS],
    len: usize,
}

impl std::fmt::Debug for MessageRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessageRef")
            .field("tags", &self.tags)
            .field("source", &self.source)
            .field("command", &self.command)
            .field("params", &self.params())
            .finish()
    }
}

/// Splits off the next space-delimited token, skipping leading spaces.
fn token(s: &str) -> (&str, &str) {
    let s = s.trim_start_matches(' ');
    s.split_once(' ').unwrap_or((s, ""))
}

impl<'a> MessageRef<'a> {
    /// Parses one line. A trailing CR and/or LF is ignored.
    ///
    /// Parsing is lenient where real servers are sloppy: runs of spaces separate
    /// parameters, trailing spaces are ignored, and if more than
    /// [`MAX_PARAMS`] parameters appear, the last one takes the rest of the line.
    pub fn parse(line: &'a str) -> Result<Self, ParseError> {
        let line = line.trim_end_matches(['\r', '\n']);
        if line.contains(['\0', '\r', '\n']) {
            return Err(ParseError::IllegalCharacter);
        }
        let mut rest = line.trim_start_matches(' ');
        if rest.is_empty() {
            return Err(ParseError::Empty);
        }

        let mut tags = None;
        if let Some(after) = rest.strip_prefix('@') {
            let (raw, tail) = after.split_once(' ').ok_or(ParseError::MissingCommand)?;
            tags = Some(raw);
            rest = tail.trim_start_matches(' ');
        }

        let mut source = None;
        if let Some(after) = rest.strip_prefix(':') {
            let (raw, tail) = after.split_once(' ').unwrap_or((after, ""));
            if raw.is_empty() {
                return Err(ParseError::EmptySource);
            }
            source = Some(raw);
            rest = tail;
        }

        let (command, mut rest) = token(rest);
        if command.is_empty() {
            return Err(ParseError::MissingCommand);
        }

        let mut params = [""; MAX_PARAMS];
        let mut len = 0;
        for slot in &mut params {
            rest = rest.trim_start_matches(' ');
            if rest.is_empty() {
                break;
            }
            len += 1;
            if let Some(trailing) = rest.strip_prefix(':') {
                *slot = trailing;
                break;
            }
            if len == MAX_PARAMS {
                *slot = rest.trim_end_matches(' ');
                break;
            }
            let (param, tail) = token(rest);
            *slot = param;
            rest = tail;
        }

        Ok(Self {
            tags,
            source,
            command,
            params,
            len,
        })
    }

    /// Raw tag data without the leading `@`, values still escaped.
    pub fn raw_tags(&self) -> Option<&'a str> {
        self.tags
    }

    /// Iterates over tags as `(key, escaped value)`, duplicates included.
    pub fn tags(&self) -> RawTags<'a> {
        RawTags::new(self.tags.unwrap_or(""))
    }

    /// Returns the unescaped value of the last tag named `key`. A tag present
    /// without a value yields `Some("")`.
    pub fn tag(&self, key: &str) -> Option<Cow<'a, str>> {
        self.tags()
            .filter(|(k, _)| *k == key)
            .last()
            .map(|(_, v)| tags::unescape(v))
    }

    /// The raw source, without the leading `:`.
    pub fn source_raw(&self) -> Option<&'a str> {
        self.source
    }

    /// The parsed source.
    pub fn source(&self) -> Option<Source<'a>> {
        self.source.map(Source::parse)
    }

    /// The command or numeric, exactly as sent.
    pub fn command(&self) -> &'a str {
        self.command
    }

    /// All parameters, with the trailing parameter's `:` removed.
    pub fn params(&self) -> &[&'a str] {
        self.params.get(..self.len).unwrap_or_default()
    }

    /// The parameter at `index`, if present.
    pub fn param(&self, index: usize) -> Option<&'a str> {
        self.params().get(index).copied()
    }

    /// Converts to an owned [`Message`], unescaping tags and dropping duplicates.
    pub fn to_owned(&self) -> Message {
        Message {
            tags: self.tags.map(Tags::from_raw).unwrap_or_default(),
            source: self.source.map(str::to_owned),
            command: self.command.to_owned(),
            params: self.params().iter().map(|&p| p.to_owned()).collect(),
        }
    }
}

/// An owned message.
///
/// Fields are public for convenience; validity is checked when serialising.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Message {
    /// Tags with unescaped values.
    pub tags: Tags,
    /// Source without the leading `:`. Clients normally leave this empty.
    pub source: Option<String>,
    /// Command or numeric.
    pub command: String,
    /// Parameters, the last of which may contain spaces.
    pub params: Vec<String>,
}

impl Message {
    /// Starts a message with the given command.
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            ..Self::default()
        }
    }

    /// Appends a parameter.
    #[must_use]
    pub fn param(mut self, param: impl Into<String>) -> Self {
        self.params.push(param.into());
        self
    }

    /// Adds or replaces a tag. An empty value means the tag has no value.
    #[must_use]
    pub fn tag(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.tags.insert(key, value);
        self
    }

    /// Sets the source.
    #[must_use]
    pub fn source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Parses an owned message from a line.
    pub fn parse(line: &str) -> Result<Self, ParseError> {
        MessageRef::parse(line).map(|m| m.to_owned())
    }

    /// Serialises to a line without the trailing CRLF, enforcing client limits:
    /// at most [`MAX_CLIENT_TAGS_LEN`] bytes of tag data and [`MAX_BODY_LEN`] bytes
    /// of body.
    pub fn to_wire(&self) -> Result<String, SerialiseError> {
        let mut out = String::with_capacity(64);
        if !self.tags.is_empty() {
            if let Some((key, _)) = self.tags.iter().find(|(k, _)| !tags::is_valid_key(k)) {
                return Err(SerialiseError::InvalidTagKey(key.to_owned()));
            }
            out.push('@');
            self.tags.write_into(&mut out);
            let len = out.len() - 1;
            if len > MAX_CLIENT_TAGS_LEN {
                return Err(SerialiseError::TagsTooLong { len });
            }
            out.push(' ');
        }
        let body_start = out.len();
        self.write_body(&mut out)?;
        let len = out.len() - body_start;
        if len > MAX_BODY_LEN {
            return Err(SerialiseError::BodyTooLong { len });
        }
        Ok(out)
    }

    fn write_body(&self, out: &mut String) -> Result<(), SerialiseError> {
        if let Some(source) = &self.source {
            if source.is_empty() || source.contains([' ', '\0', '\r', '\n']) {
                return Err(SerialiseError::InvalidSource);
            }
            out.push(':');
            out.push_str(source);
            out.push(' ');
        }
        if self.command.is_empty() || !self.command.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(SerialiseError::InvalidCommand(self.command.clone()));
        }
        out.push_str(&self.command);

        if self.params.len() > MAX_PARAMS {
            return Err(SerialiseError::TooManyParams(self.params.len()));
        }
        let last = self.params.len().saturating_sub(1);
        for (i, param) in self.params.iter().enumerate() {
            if param.contains(['\0', '\r', '\n']) {
                return Err(SerialiseError::IllegalCharacter(i));
            }
            let needs_colon = param.is_empty() || param.contains(' ') || param.starts_with(':');
            out.push(' ');
            if needs_colon {
                if i != last {
                    return Err(SerialiseError::InvalidMiddleParam(i));
                }
                out.push(':');
            }
            out.push_str(param);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tags_source_command_params() {
        let m = MessageRef::parse("@a=1;b :nick!u@h PRIVMSG #chan :hello world\r\n").unwrap();
        assert_eq!(m.raw_tags(), Some("a=1;b"));
        assert_eq!(m.tag("a").as_deref(), Some("1"));
        assert_eq!(m.tag("b").as_deref(), Some(""));
        assert_eq!(m.tag("c"), None);
        assert_eq!(m.source().unwrap().nick, "nick");
        assert_eq!(m.command(), "PRIVMSG");
        assert_eq!(m.params(), ["#chan", "hello world"]);
        assert_eq!(m.param(1), Some("hello world"));
        assert_eq!(m.param(2), None);
    }

    #[test]
    fn rejects_malformed_lines() {
        assert_eq!(MessageRef::parse(""), Err(ParseError::Empty));
        assert_eq!(MessageRef::parse("   \r\n"), Err(ParseError::Empty));
        assert_eq!(MessageRef::parse("@a=b"), Err(ParseError::MissingCommand));
        assert_eq!(MessageRef::parse(":src"), Err(ParseError::MissingCommand));
        assert_eq!(
            MessageRef::parse(":src   "),
            Err(ParseError::MissingCommand)
        );
        assert_eq!(MessageRef::parse(": CMD"), Err(ParseError::EmptySource));
        assert_eq!(
            MessageRef::parse("CMD a\0b"),
            Err(ParseError::IllegalCharacter)
        );
        assert_eq!(
            MessageRef::parse("CMD a\rb"),
            Err(ParseError::IllegalCharacter)
        );
    }

    #[test]
    fn fifteenth_param_takes_the_rest() {
        let line = format!(
            "CMD {} last one  ",
            (1..=13)
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(" ")
        );
        let m = MessageRef::parse(&line).unwrap();
        assert_eq!(m.params().len(), 15);
        assert_eq!(m.param(13), Some("last"));
        assert_eq!(m.param(14), Some("one"));

        let line = format!(
            "CMD {} x y z",
            (1..=14)
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(" ")
        );
        let m = MessageRef::parse(&line).unwrap();
        assert_eq!(m.param(14), Some("x y z"));
    }

    #[test]
    fn debug_shows_only_used_params() {
        let m = MessageRef::parse("PING :x").unwrap();
        assert!(format!("{m:?}").contains(r#"params: ["x"]"#));
    }

    #[test]
    fn builds_and_serialises() {
        let m = Message::new("PRIVMSG")
            .tag("+draft/reply", "abc")
            .tag("+typing", "")
            .param("#chan")
            .param(":)");
        assert_eq!(
            m.to_wire().unwrap(),
            "@+draft/reply=abc;+typing PRIVMSG #chan ::)"
        );
        assert_eq!(
            Message::new("CAP").param("END").to_wire().unwrap(),
            "CAP END"
        );
        assert_eq!(Message::new("X").param("").to_wire().unwrap(), "X :");
        assert_eq!(
            Message::new("X")
                .source("srv")
                .param("a b")
                .to_wire()
                .unwrap(),
            ":srv X :a b"
        );
    }

    #[test]
    fn serialise_errors() {
        use SerialiseError as E;
        assert_eq!(
            Message::new("").to_wire(),
            Err(E::InvalidCommand(String::new()))
        );
        assert_eq!(
            Message::new("PRIV MSG").to_wire(),
            Err(E::InvalidCommand("PRIV MSG".into()))
        );
        assert_eq!(
            Message::new("X").source("a b").to_wire(),
            Err(E::InvalidSource)
        );
        assert_eq!(
            Message::new("X").source("").to_wire(),
            Err(E::InvalidSource)
        );
        assert_eq!(
            Message::new("X").source("a\rb").to_wire(),
            Err(E::InvalidSource)
        );
        assert!(Message::new("X").source("a\tb\x03").to_wire().is_ok());
        assert_eq!(
            Message::new("X").tag("bad key", "").to_wire(),
            Err(E::InvalidTagKey("bad key".into()))
        );
        assert_eq!(
            Message::new("X").param("a\r\nQUIT").to_wire(),
            Err(E::IllegalCharacter(0))
        );
        assert_eq!(
            Message::new("X").param("a b").param("c").to_wire(),
            Err(E::InvalidMiddleParam(0))
        );
        assert_eq!(
            Message::new("X").param("").param("c").to_wire(),
            Err(E::InvalidMiddleParam(0))
        );
        assert_eq!(
            Message::new("X").param(":a").param("c").to_wire(),
            Err(E::InvalidMiddleParam(0))
        );
        let mut many = Message::new("X");
        many.params = vec!["a".into(); 16];
        assert_eq!(many.to_wire(), Err(E::TooManyParams(16)));
        assert_eq!(
            Message::new("X").param("a".repeat(MAX_BODY_LEN)).to_wire(),
            Err(E::BodyTooLong {
                len: MAX_BODY_LEN + 2
            })
        );
        assert!(
            Message::new("X")
                .param("a".repeat(MAX_BODY_LEN - 2))
                .to_wire()
                .is_ok()
        );
        assert_eq!(
            Message::new("X")
                .tag("a", "b".repeat(MAX_CLIENT_TAGS_LEN))
                .to_wire(),
            Err(E::TagsTooLong {
                len: MAX_CLIENT_TAGS_LEN + 2
            })
        );
    }

    #[test]
    fn owned_round_trip() {
        let line = "@a=x\\sy;a=z :n!u@h PRIVMSG #c :hi there";
        let owned = Message::parse(line).unwrap();
        assert_eq!(owned.tags.get("a"), Some("z"));
        assert_eq!(owned.to_wire().unwrap(), "@a=z :n!u@h PRIVMSG #c :hi there");
    }
}
