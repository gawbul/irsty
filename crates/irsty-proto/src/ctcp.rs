//! Client-to-client protocol framing inside `PRIVMSG` and `NOTICE`.

const DELIM: char = '\x01';

/// A CTCP request or reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ctcp<'a> {
    /// The CTCP command, such as `ACTION` or `VERSION`, as sent.
    pub command: &'a str,
    /// Everything after the command, if anything.
    pub params: Option<&'a str>,
}

impl<'a> Ctcp<'a> {
    /// Parses a message body as CTCP. Returns `None` if it is not CTCP. A missing
    /// closing delimiter is tolerated, as many clients omit it.
    pub fn parse(text: &'a str) -> Option<Self> {
        let inner = text.strip_prefix(DELIM)?;
        let inner = inner.strip_suffix(DELIM).unwrap_or(inner);
        let (command, params) = match inner.split_once(' ') {
            Some((command, params)) => (command, Some(params)),
            None => (inner, None),
        };
        if command.is_empty() || command.contains(DELIM) {
            return None;
        }
        Some(Self { command, params })
    }

    /// Reports whether this is a `/me` action.
    pub fn is_action(&self) -> bool {
        self.command.eq_ignore_ascii_case("ACTION")
    }
}

/// Wraps a command and optional parameters in CTCP delimiters.
pub fn encode(command: &str, params: Option<&str>) -> String {
    match params {
        Some(params) => format!("{DELIM}{command} {params}{DELIM}"),
        None => format!("{DELIM}{command}{DELIM}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ctcp() {
        let action = Ctcp::parse("\x01ACTION waves hello\x01").unwrap();
        assert!(action.is_action());
        assert_eq!(action.params, Some("waves hello"));
        let version = Ctcp::parse("\x01VERSION").unwrap();
        assert_eq!((version.command, version.params), ("VERSION", None));
        assert_eq!(Ctcp::parse("hello"), None);
        assert_eq!(Ctcp::parse("\x01\x01"), None);
        assert_eq!(Ctcp::parse("\x01 x\x01"), None);
        assert!(!Ctcp::parse("\x01PING 1\x01").unwrap().is_action());
    }

    #[test]
    fn encodes_ctcp() {
        assert_eq!(encode("ACTION", Some("waves")), "\x01ACTION waves\x01");
        assert_eq!(encode("VERSION", None), "\x01VERSION\x01");
    }
}
