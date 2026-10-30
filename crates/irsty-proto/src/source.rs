//! Message sources (prefixes): `nick!user@host` or a server name.

/// A parsed message source. Fields borrow from the line.
///
/// Servers send whatever they like in hostnames (vhosts with formatting codes are
/// not unusual), so nothing here is validated. Sanitise before display.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Source<'a> {
    /// The nick, or the whole source if it is a server name.
    pub nick: &'a str,
    /// The username (ident), if present.
    pub user: Option<&'a str>,
    /// The hostname, if present.
    pub host: Option<&'a str>,
}

impl<'a> Source<'a> {
    /// Splits a raw source into its parts. Never fails.
    pub fn parse(raw: &'a str) -> Self {
        let (before_host, host) = match raw.split_once('@') {
            Some((before, host)) => (before, Some(host)),
            None => (raw, None),
        };
        let (nick, user) = match before_host.split_once('!') {
            Some((nick, user)) => (nick, Some(user)),
            None => (before_host, None),
        };
        Self { nick, user, host }
    }

    /// Heuristic: a source with no user or host that contains a dot is a server.
    /// Nicks may not contain dots.
    pub fn is_server(&self) -> bool {
        self.user.is_none() && self.host.is_none() && self.nick.contains('.')
    }
}

#[cfg(test)]
mod tests {
    use super::Source;

    #[test]
    fn parses_full_and_partial_sources() {
        let full = Source::parse("nick!~user@host.example");
        assert_eq!(
            (full.nick, full.user, full.host),
            ("nick", Some("~user"), Some("host.example"))
        );
        let server = Source::parse("irc.example.com");
        assert!(server.is_server());
        assert!(!full.is_server());
        assert!(!Source::parse("nick").is_server());
    }
}
