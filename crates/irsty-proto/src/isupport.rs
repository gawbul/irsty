//! `RPL_ISUPPORT` (numeric 005): what the server supports.

use std::collections::BTreeMap;

use crate::casemap::CaseMapping;

/// Channel mode classes from `CHANMODES=A,B,C,D`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChanModes {
    /// List modes (always take a parameter), such as bans.
    pub a: String,
    /// Always take a parameter, such as the key.
    pub b: String,
    /// Take a parameter only when set, such as the limit.
    pub c: String,
    /// Never take a parameter.
    pub d: String,
}

impl Default for ChanModes {
    fn default() -> Self {
        Self {
            a: "beI".into(),
            b: "k".into(),
            c: "l".into(),
            d: "imnpst".into(),
        }
    }
}

/// How a channel mode uses its parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeKind {
    /// A list mode (class A).
    List,
    /// Always has a parameter (class B, and prefix modes such as `o`).
    Always,
    /// Has a parameter only when being set (class C).
    OnSet,
    /// Never has a parameter (class D, and unknown modes).
    Never,
}

/// Parsed `RPL_ISUPPORT` state. Defaults follow RFC 1459 and common practice and
/// apply until the server says otherwise.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ISupport {
    /// `CASEMAPPING`.
    pub casemapping: CaseMapping,
    /// `CHANTYPES`: characters that start a channel name.
    pub chantypes: String,
    /// `PREFIX`: `(mode, symbol)` pairs, highest rank first.
    pub prefix: Vec<(char, char)>,
    /// `CHANMODES`.
    pub chanmodes: ChanModes,
    /// `STATUSMSG`: prefixes allowed before a channel target, such as `@#chan`.
    pub statusmsg: String,
    /// `NETWORK`: the network's name.
    pub network: Option<String>,
    /// `NICKLEN`.
    pub nicklen: Option<usize>,
    /// `CHANNELLEN`.
    pub channellen: Option<usize>,
    /// `TOPICLEN`.
    pub topiclen: Option<usize>,
    /// `LINELEN`: maximum line length including CRLF, if larger than 512.
    pub linelen: Option<usize>,
    /// `MONITOR`: present when supported, with the target limit if any.
    pub monitor: Option<Option<usize>>,
    /// `WHOX`.
    pub whox: bool,
    /// `UTF8ONLY`.
    pub utf8only: bool,
    /// `BOT`: the user mode marking bots.
    pub bot: Option<char>,
    /// Every token as received (values unescaped), for features without a field.
    pub raw: BTreeMap<String, String>,
}

impl Default for ISupport {
    fn default() -> Self {
        Self {
            casemapping: CaseMapping::default(),
            chantypes: "#&".into(),
            prefix: vec![('o', '@'), ('v', '+')],
            chanmodes: ChanModes::default(),
            statusmsg: String::new(),
            network: None,
            nicklen: None,
            channellen: None,
            topiclen: None,
            linelen: None,
            monitor: None,
            whox: false,
            utf8only: false,
            bot: None,
            raw: BTreeMap::new(),
        }
    }
}

/// Unescapes `\xHH` sequences in an `RPL_ISUPPORT` value. Malformed escapes and
/// escapes that would produce invalid UTF-8 are kept as they are.
pub fn unescape_value(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if b == b'\\'
            && bytes.get(i + 1) == Some(&b'x')
            && let Some(hex) = bytes.get(i + 2..i + 4)
            && let Ok(hex) = std::str::from_utf8(hex)
            && let Ok(decoded) = u8::from_str_radix(hex, 16)
        {
            out.push(decoded);
            i += 4;
            continue;
        }
        out.push(b);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| value.to_owned())
}

fn parse_prefix(value: &str) -> Option<Vec<(char, char)>> {
    if value.is_empty() {
        return Some(Vec::new());
    }
    let (modes, symbols) = value.strip_prefix('(')?.split_once(')')?;
    (modes.chars().count() == symbols.chars().count())
        .then(|| modes.chars().zip(symbols.chars()).collect())
}

impl ISupport {
    /// Applies the parameters of one `005` reply: the first (our nick) and the
    /// last (the human-readable trailer) are skipped. `-TOKEN` resets a token to its
    /// default.
    pub fn apply(&mut self, params: &[&str]) {
        let [_, tokens @ .., _] = params else {
            return;
        };
        for token in tokens {
            if let Some(name) = token.strip_prefix('-') {
                self.reset(name);
            } else {
                let (name, value) = token.split_once('=').unwrap_or((token, ""));
                self.set(name, &unescape_value(value));
            }
        }
    }

    fn set(&mut self, name: &str, value: &str) {
        let number = || value.parse::<usize>().ok().filter(|&n| n > 0);
        match name {
            "CASEMAPPING" => {
                if let Some(mapping) = CaseMapping::from_token(value) {
                    self.casemapping = mapping;
                }
            }
            "CHANTYPES" => value.clone_into(&mut self.chantypes),
            "PREFIX" => {
                if let Some(prefix) = parse_prefix(value) {
                    self.prefix = prefix;
                }
            }
            "CHANMODES" => {
                let mut classes = value.split(',').map(str::to_owned);
                self.chanmodes = ChanModes {
                    a: classes.next().unwrap_or_default(),
                    b: classes.next().unwrap_or_default(),
                    c: classes.next().unwrap_or_default(),
                    d: classes.next().unwrap_or_default(),
                };
            }
            "STATUSMSG" => value.clone_into(&mut self.statusmsg),
            "NETWORK" => self.network = Some(value.to_owned()).filter(|v| !v.is_empty()),
            "NICKLEN" => self.nicklen = number(),
            "CHANNELLEN" => self.channellen = number(),
            "TOPICLEN" => self.topiclen = number(),
            "LINELEN" => self.linelen = number(),
            "MONITOR" => self.monitor = Some(number()),
            "WHOX" => self.whox = true,
            "UTF8ONLY" => self.utf8only = true,
            "BOT" => self.bot = value.chars().next(),
            _ => {}
        }
        self.raw.insert(name.to_owned(), value.to_owned());
    }

    fn reset(&mut self, name: &str) {
        let default = Self::default();
        match name {
            "CASEMAPPING" => self.casemapping = default.casemapping,
            "CHANTYPES" => self.chantypes = default.chantypes,
            "PREFIX" => self.prefix = default.prefix,
            "CHANMODES" => self.chanmodes = default.chanmodes,
            "STATUSMSG" => self.statusmsg = default.statusmsg,
            "NETWORK" => self.network = None,
            "NICKLEN" => self.nicklen = None,
            "CHANNELLEN" => self.channellen = None,
            "TOPICLEN" => self.topiclen = None,
            "LINELEN" => self.linelen = None,
            "MONITOR" => self.monitor = None,
            "WHOX" => self.whox = false,
            "UTF8ONLY" => self.utf8only = false,
            "BOT" => self.bot = None,
            _ => {}
        }
        self.raw.remove(name);
    }

    /// Reports whether `target` is a channel name.
    pub fn is_channel(&self, target: &str) -> bool {
        target
            .chars()
            .next()
            .is_some_and(|c| self.chantypes.contains(c))
    }

    /// Returns the prefix symbol for a membership mode, such as `@` for `o`.
    pub fn symbol_for_mode(&self, mode: char) -> Option<char> {
        self.prefix
            .iter()
            .find(|(m, _)| *m == mode)
            .map(|&(_, s)| s)
    }

    /// Returns the membership mode for a prefix symbol, such as `o` for `@`.
    pub fn mode_for_symbol(&self, symbol: char) -> Option<char> {
        self.prefix
            .iter()
            .find(|(_, s)| *s == symbol)
            .map(|&(m, _)| m)
    }

    /// Ranks a membership mode: 0 is the highest. Unknown modes rank last.
    pub fn mode_rank(&self, mode: char) -> usize {
        self.prefix
            .iter()
            .position(|(m, _)| *m == mode)
            .unwrap_or(usize::MAX)
    }

    /// Splits leading membership symbols from a `NAMES` entry. With multi-prefix
    /// there can be several, such as `@+nick`.
    pub fn split_prefixes<'a>(&self, entry: &'a str) -> (&'a str, &'a str) {
        let split = entry
            .char_indices()
            .find(|&(_, c)| self.mode_for_symbol(c).is_none())
            .map_or(entry.len(), |(i, _)| i);
        entry.split_at(split)
    }

    /// Strips a `STATUSMSG` prefix from a target, returning it and the channel.
    pub fn split_statusmsg<'a>(&self, target: &'a str) -> (Option<char>, &'a str) {
        let mut chars = target.chars();
        match chars.next() {
            Some(c) if self.statusmsg.contains(c) && self.is_channel(chars.as_str()) => {
                (Some(c), chars.as_str())
            }
            _ => (None, target),
        }
    }

    /// Classifies how a channel mode uses its parameter.
    pub fn mode_kind(&self, mode: char) -> ModeKind {
        let modes = &self.chanmodes;
        if modes.a.contains(mode) {
            ModeKind::List
        } else if modes.b.contains(mode) || self.symbol_for_mode(mode).is_some() {
            ModeKind::Always
        } else if modes.c.contains(mode) {
            ModeKind::OnSet
        } else {
            ModeKind::Never
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(isupport: &mut ISupport, line: &str) {
        let tokens: Vec<&str> = std::iter::once("me")
            .chain(line.split(' '))
            .chain(std::iter::once("are supported by this server"))
            .collect();
        isupport.apply(&tokens);
    }

    #[test]
    fn parses_common_tokens() {
        let mut i = ISupport::default();
        apply(
            &mut i,
            "CASEMAPPING=ascii CHANTYPES=# PREFIX=(qaohv)~&@%+ CHANMODES=beI,k,l,imnpst \
             STATUSMSG=@+ NETWORK=Libera\\x20Chat NICKLEN=16 LINELEN=4096 MONITOR=100 WHOX UTF8ONLY BOT=B",
        );
        assert_eq!(i.casemapping, CaseMapping::Ascii);
        assert_eq!(i.chantypes, "#");
        assert_eq!(i.prefix.len(), 5);
        assert_eq!(i.network.as_deref(), Some("Libera Chat"));
        assert_eq!(i.nicklen, Some(16));
        assert_eq!(i.linelen, Some(4096));
        assert_eq!(i.monitor, Some(Some(100)));
        assert!(i.whox && i.utf8only);
        assert_eq!(i.bot, Some('B'));
        assert_eq!(
            i.raw.get("NETWORK").map(String::as_str),
            Some("Libera Chat")
        );
    }

    #[test]
    fn negation_resets_to_defaults() {
        let mut i = ISupport::default();
        apply(&mut i, "CASEMAPPING=ascii MONITOR WHOX NETWORK=x");
        assert_eq!(i.monitor, Some(None));
        apply(&mut i, "-CASEMAPPING -MONITOR -WHOX -NETWORK");
        assert_eq!(i.casemapping, CaseMapping::Rfc1459);
        assert_eq!(i.monitor, None);
        assert!(!i.whox);
        assert!(!i.raw.contains_key("NETWORK"));
    }

    #[test]
    fn ignores_bad_values() {
        let mut i = ISupport::default();
        apply(
            &mut i,
            "CASEMAPPING=weird PREFIX=(ov)@ NICKLEN=abc PREFIX=nonsense",
        );
        assert_eq!(i.casemapping, CaseMapping::Rfc1459);
        assert_eq!(i.prefix, ISupport::default().prefix);
        assert_eq!(i.nicklen, None);
        apply(&mut i, "PREFIX=");
        assert!(i.prefix.is_empty());
        // Too few params is ignored entirely.
        i.apply(&["me"]);
    }

    #[test]
    fn unescapes_values() {
        assert_eq!(unescape_value(r"a\x20b\x3Dc"), "a b=c");
        assert_eq!(unescape_value(r"\x2"), r"\x2");
        assert_eq!(unescape_value(r"\xZZ"), r"\xZZ");
        assert_eq!(unescape_value(r"\xFF"), r"\xFF");
    }

    #[test]
    fn membership_helpers() {
        let mut i = ISupport::default();
        apply(&mut i, "PREFIX=(qov)~@+ STATUSMSG=@+");
        assert_eq!(i.split_prefixes("@+nick"), ("@+", "nick"));
        assert_eq!(i.split_prefixes("nick"), ("", "nick"));
        assert_eq!(i.split_prefixes("~"), ("~", ""));
        assert_eq!(i.symbol_for_mode('o'), Some('@'));
        assert_eq!(i.mode_for_symbol('~'), Some('q'));
        assert!(i.mode_rank('q') < i.mode_rank('v'));
        assert_eq!(i.mode_rank('x'), usize::MAX);
        assert_eq!(i.split_statusmsg("@#chan"), (Some('@'), "#chan"));
        assert_eq!(i.split_statusmsg("@nick"), (None, "@nick"));
        assert_eq!(i.split_statusmsg("#chan"), (None, "#chan"));
        assert!(i.is_channel("#x") && !i.is_channel("x") && !i.is_channel(""));
    }

    #[test]
    fn mode_kinds() {
        let i = ISupport::default();
        assert_eq!(i.mode_kind('b'), ModeKind::List);
        assert_eq!(i.mode_kind('k'), ModeKind::Always);
        assert_eq!(i.mode_kind('o'), ModeKind::Always);
        assert_eq!(i.mode_kind('l'), ModeKind::OnSet);
        assert_eq!(i.mode_kind('n'), ModeKind::Never);
        assert_eq!(i.mode_kind('Z'), ModeKind::Never);
    }
}
