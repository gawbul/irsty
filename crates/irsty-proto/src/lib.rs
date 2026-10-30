//! IRC and IRCv3 protocol primitives for Irsty.
//!
//! This crate performs no I/O and has no async runtime. It parses and serialises
//! messages, tracks `RPL_ISUPPORT`, applies casemapping, and drives the CAP and SASL
//! negotiation state machines, so every piece can be tested without a socket.
//!
//! Text from the network must pass through [`format::StyledText`] before it reaches
//! a terminal; see ADR 0005.
#![forbid(unsafe_code)]

pub mod casemap;
pub mod ctcp;
pub mod decode;
pub mod format;
pub mod isupport;
pub mod mask;
pub mod message;
pub mod source;
pub mod split;
pub mod tags;

pub use casemap::CaseMapping;
pub use isupport::ISupport;
pub use message::{Message, MessageRef, ParseError, SerialiseError};
pub use source::Source;
pub use tags::Tags;

/// Maximum length of the non-tag part of a line in bytes, excluding the trailing CRLF.
pub const MAX_BODY_LEN: usize = 510;

/// Maximum length of tag data a client may send, excluding the leading `@` and the
/// trailing space (IRCv3 message-tags).
pub const MAX_CLIENT_TAGS_LEN: usize = 4094;

/// Maximum length of the whole tag section a server may send, including the leading
/// `@` and the trailing space.
pub const MAX_SERVER_TAGS_LEN: usize = 8191;

/// Hard cap on a whole incoming line: tags plus body plus CRLF. Anything longer is a
/// protocol violation and is discarded by the codec.
pub const MAX_LINE_LEN: usize = MAX_SERVER_TAGS_LEN + MAX_BODY_LEN + 2;
