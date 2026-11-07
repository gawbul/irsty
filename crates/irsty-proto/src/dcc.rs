//! DCC: the CTCP messages that set up direct client-to-client connections
//! for files (`SEND`, with `RESUME` and `ACCEPT`) and chats (`CHAT`).
//!
//! These follow what irssi, mIRC and others send rather than a finished
//! spec. File names may be quoted, or not quoted and contain spaces (found
//! by reading the numeric fields from the end); IPv4 addresses travel as a
//! 32-bit integer, IPv6 ones as text. Port 0 with a token is passive
//! ("reverse") DCC: the offerer can't listen, so the other side listens and
//! answers with the same message, its own address and port, and the token.
//!
//! Everything here comes from other people and is untrusted, file names
//! above all; [`safe_file_name`] makes one safe to create.

use std::net::{IpAddr, Ipv4Addr};

/// A DCC request, as carried in a CTCP `DCC` message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dcc {
    /// Offer a file (or, with a token and a real port, answer a passive
    /// offer).
    Send {
        /// The file name as sent: untrusted, see [`safe_file_name`].
        file: String,
        /// Where to connect.
        addr: IpAddr,
        /// The port to connect to; 0 for a passive offer.
        port: u16,
        /// The file's size, if given.
        size: Option<u64>,
        /// The passive DCC token.
        token: Option<String>,
    },
    /// Offer a chat.
    Chat {
        /// Where to connect.
        addr: IpAddr,
        /// The port to connect to; 0 for a passive offer.
        port: u16,
        /// The passive DCC token.
        token: Option<String>,
    },
    /// Ask to resume an offered file from `position`.
    Resume {
        /// The file name, as in the offer.
        file: String,
        /// The offer's port (0 for passive).
        port: u16,
        /// Bytes already received.
        position: u64,
        /// The passive DCC token.
        token: Option<String>,
    },
    /// Agree to resume from `position`.
    Accept {
        /// The file name, as in the offer.
        file: String,
        /// The offer's port (0 for passive).
        port: u16,
        /// Where the transfer resumes.
        position: u64,
        /// The passive DCC token.
        token: Option<String>,
    },
}

/// Parses an address as DCC sends it: a 32-bit integer for IPv4, or text
/// (IPv6, or dotted IPv4 from lenient clients).
pub fn parse_addr(text: &str) -> Option<IpAddr> {
    if let Ok(n) = text.parse::<u32>() {
        return Some(IpAddr::V4(Ipv4Addr::from(n)));
    }
    text.parse().ok()
}

/// An address as DCC sends it.
pub fn encode_addr(addr: IpAddr) -> String {
    match addr {
        IpAddr::V4(v4) => u32::from(v4).to_string(),
        IpAddr::V6(v6) => v6.to_string(),
    }
}

/// A file name for a DCC message: quoted if it has a space.
fn encode_file(file: &str) -> String {
    let file = file.replace('"', "'");
    if file.contains(' ') {
        format!("\"{file}\"")
    } else {
        file
    }
}

fn token_suffix(token: Option<&String>) -> String {
    token.map(|t| format!(" {t}")).unwrap_or_default()
}

impl Dcc {
    /// Parses the parameters of a CTCP `DCC` request (everything after
    /// `DCC `). Returns `None` for anything malformed or unknown.
    pub fn parse(params: &str) -> Option<Self> {
        let (kind, rest) = params.trim().split_once(' ')?;
        let words: Vec<&str> = rest.split(' ').filter(|w| !w.is_empty()).collect();
        match kind.to_ascii_uppercase().as_str() {
            "SEND" => {
                let (file, tail) = split_file(&words, |tail| {
                    // <addr> <port> <size> [<token>]. A token is a number,
                    // as clients send it: with port 0 in a passive offer, or
                    // a real port in the answer to one.
                    match tail {
                        [addr, port, size] => {
                            parse_addr(addr).is_some()
                                && port.parse::<u16>().is_ok()
                                && size.parse::<u64>().is_ok()
                        }
                        [addr, port, size, token] => {
                            parse_addr(addr).is_some()
                                && port.parse::<u16>().is_ok()
                                && size.parse::<u64>().is_ok()
                                && !token.is_empty()
                                && token.bytes().all(|b| b.is_ascii_digit())
                        }
                        _ => false,
                    }
                })?;
                Some(Self::Send {
                    file,
                    addr: parse_addr(tail.first()?)?,
                    port: tail.get(1)?.parse().ok()?,
                    size: tail.get(2).and_then(|s| s.parse().ok()),
                    token: tail.get(3).map(|t| (*t).to_owned()),
                })
            }
            "CHAT" => {
                // CHAT <protocol, usually "chat"> <addr> <port> [<token>]
                let addr = parse_addr(words.get(1)?)?;
                let port = words.get(2)?.parse().ok()?;
                Some(Self::Chat {
                    addr,
                    port,
                    token: words.get(3).map(|t| (*t).to_owned()),
                })
            }
            kind @ ("RESUME" | "ACCEPT") => {
                let (file, tail) = split_file(&words, |tail| match tail {
                    [port, position] => {
                        port.parse::<u16>().is_ok() && position.parse::<u64>().is_ok()
                    }
                    [port, position, _token] => *port == "0" && position.parse::<u64>().is_ok(),
                    _ => false,
                })?;
                let port = tail.first()?.parse().ok()?;
                let position = tail.get(1)?.parse().ok()?;
                let token = tail.get(2).map(|t| (*t).to_owned());
                Some(if kind == "RESUME" {
                    Self::Resume {
                        file,
                        port,
                        position,
                        token,
                    }
                } else {
                    Self::Accept {
                        file,
                        port,
                        position,
                        token,
                    }
                })
            }
            _ => None,
        }
    }

    /// The parameters of the CTCP `DCC` request (everything after `DCC `).
    pub fn encode(&self) -> String {
        match self {
            Self::Send {
                file,
                addr,
                port,
                size,
                token,
            } => format!(
                "SEND {} {} {port} {}{}",
                encode_file(file),
                encode_addr(*addr),
                size.unwrap_or(0),
                token_suffix(token.as_ref())
            ),
            Self::Chat { addr, port, token } => format!(
                "CHAT chat {} {port}{}",
                encode_addr(*addr),
                token_suffix(token.as_ref())
            ),
            Self::Resume {
                file,
                port,
                position,
                token,
            } => format!(
                "RESUME {} {port} {position}{}",
                encode_file(file),
                token_suffix(token.as_ref())
            ),
            Self::Accept {
                file,
                port,
                position,
                token,
            } => format!(
                "ACCEPT {} {port} {position}{}",
                encode_file(file),
                token_suffix(token.as_ref())
            ),
        }
    }
}

/// Splits words into a file name and the fields after it. A name starting
/// with `"` runs to the word ending with `"`; otherwise the fields are
/// found from the end, trying the longest `tail_ok` accepts, so unquoted
/// names may contain spaces.
fn split_file<'a>(
    words: &[&'a str],
    tail_ok: impl Fn(&[&'a str]) -> bool,
) -> Option<(String, Vec<&'a str>)> {
    if words.first()?.starts_with('"') {
        let end = words
            .iter()
            .position(|w| w.ends_with('"') && w.len() > 1)
            .or_else(|| {
                words
                    .iter()
                    .skip(1)
                    .position(|w| w.ends_with('"'))
                    .map(|i| i + 1)
            })?;
        let name = words.get(..=end)?.join(" ");
        let name = name.trim_matches('"').to_owned();
        let tail = words.get(end + 1..)?.to_vec();
        return tail_ok(&tail).then_some((name, tail));
    }
    for len in [4, 3, 2] {
        if words.len() > len {
            let (name, tail) = words.split_at(words.len() - len);
            if tail_ok(tail) {
                return Some((name.join(" "), tail.to_vec()));
            }
        }
    }
    None
}

/// Most bytes kept of a file name.
const MAX_NAME: usize = 200;

/// Names Windows reserves for devices, whatever the extension.
const RESERVED: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Makes an untrusted file name safe to create in a download directory:
/// only the last path component is kept, control characters and characters
/// that are special on some file system are replaced, leading dots and
/// spaces are dropped (no hidden files, no `..`), device names are
/// avoided, and it is cut to 200 bytes keeping the extension. Never empty.
pub fn safe_file_name(untrusted: &str) -> String {
    let last = untrusted.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = last
        .chars()
        .map(|c| {
            if c.is_control()
                || matches!(
                    c,
                    '<' | '>'
                        | ':'
                        | '"'
                        | '|'
                        | '?'
                        | '*'
                        | '\u{202e}'
                        | '\u{202d}'
                        | '\u{2066}'
                        | '\u{2067}'
                        | '\u{2068}'
                        | '\u{2069}'
                )
            {
                '_'
            } else {
                c
            }
        })
        .collect();
    let mut name = cleaned
        .trim_start_matches(['.', ' '])
        .trim_end_matches(['.', ' '])
        .to_owned();
    if name.is_empty() {
        return "download".into();
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if RESERVED.contains(&stem.as_str()) {
        name.insert(0, '_');
    }
    if name.len() > MAX_NAME {
        let ext = name
            .rsplit_once('.')
            .map(|(_, e)| e)
            .filter(|e| e.len() <= 16)
            .map(|e| format!(".{e}"))
            .unwrap_or_default();
        let mut end = MAX_NAME - ext.len();
        while !name.is_char_boundary(end) {
            end -= 1;
        }
        name.truncate(end);
        name.push_str(&ext);
    }
    name
}

/// The acknowledgement a receiver sends after data arrives: the total
/// received, as a 32-bit big-endian number (modulo 2³², as files can be
/// bigger).
pub fn ack(received: u64) -> [u8; 4] {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "acknowledgements count modulo 2^32 by design"
    )]
    let low = received as u32;
    low.to_be_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv6Addr;

    #[test]
    fn parses_sends_as_clients_send_them() {
        assert_eq!(
            Dcc::parse("SEND notes.txt 2130706433 5000 1234"),
            Some(Dcc::Send {
                file: "notes.txt".into(),
                addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
                port: 5000,
                size: Some(1234),
                token: None,
            })
        );
        // Quoted, with spaces.
        assert_eq!(
            Dcc::parse("SEND \"my holiday photo.jpg\" 3232235777 6000 99"),
            Some(Dcc::Send {
                file: "my holiday photo.jpg".into(),
                addr: "192.168.1.1".parse().unwrap(),
                port: 6000,
                size: Some(99),
                token: None,
            })
        );
        // Unquoted with spaces: the fields are read from the end.
        let Some(Dcc::Send { file, .. }) = Dcc::parse("SEND my file 2 2130706433 5000 10") else {
            panic!()
        };
        assert_eq!(file, "my file 2");
        // The answer to a passive offer: a real port and the token.
        let Some(Dcc::Send { port, token, .. }) = Dcc::parse("SEND nat.txt 2130706433 54540 11 42")
        else {
            panic!()
        };
        assert_eq!((port, token.as_deref()), (54540, Some("42")));
        // Passive: port 0 and a token, as irssi sends it.
        assert_eq!(
            Dcc::parse("SEND big.iso 16843009 0 4294967300 42"),
            Some(Dcc::Send {
                file: "big.iso".into(),
                addr: "1.1.1.1".parse().unwrap(),
                port: 0,
                size: Some(4_294_967_300),
                token: Some("42".into()),
            })
        );
        // IPv6.
        let Some(Dcc::Send { addr, .. }) = Dcc::parse("SEND x ::1 5000 1") else {
            panic!()
        };
        assert_eq!(addr, IpAddr::V6(Ipv6Addr::LOCALHOST));
    }

    #[test]
    fn parses_chats_resumes_and_accepts() {
        assert_eq!(
            Dcc::parse("CHAT chat 2130706433 7000"),
            Some(Dcc::Chat {
                addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
                port: 7000,
                token: None
            })
        );
        assert_eq!(
            Dcc::parse("RESUME \"a b.txt\" 5000 300"),
            Some(Dcc::Resume {
                file: "a b.txt".into(),
                port: 5000,
                position: 300,
                token: None
            })
        );
        assert_eq!(
            Dcc::parse("ACCEPT file.bin 0 300 17"),
            Some(Dcc::Accept {
                file: "file.bin".into(),
                port: 0,
                position: 300,
                token: Some("17".into())
            })
        );
    }

    #[test]
    fn refuses_nonsense() {
        for bad in [
            "",
            "SEND",
            "SEND x",
            "SEND x 1 2",
            "SEND x notanip 5000 1",
            "SEND x 2130706433 70000 1",
            "SEND x 2130706433 5000 -1",
            "SEND \"unterminated 2130706433 5000 1",
            "CHAT chat",
            "CHAT chat 2130706433 port",
            "RESUME x 5000",
            "TSEND x 1 2 3",
            "SEND x 2130706433 5000 1 token",
        ] {
            assert_eq!(Dcc::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn round_trips() {
        for dcc in [
            Dcc::Send {
                file: "two words.txt".into(),
                addr: "10.0.0.5".parse().unwrap(),
                port: 5001,
                size: Some(7),
                token: None,
            },
            Dcc::Send {
                file: "p.bin".into(),
                addr: "::1".parse().unwrap(),
                port: 0,
                size: Some(7),
                token: Some("9".into()),
            },
            Dcc::Chat {
                addr: "10.0.0.5".parse().unwrap(),
                port: 0,
                token: Some("3".into()),
            },
            Dcc::Resume {
                file: "f".into(),
                port: 5001,
                position: 3,
                token: None,
            },
            Dcc::Accept {
                file: "f g".into(),
                port: 0,
                position: 3,
                token: Some("1".into()),
            },
        ] {
            assert_eq!(
                Dcc::parse(&dcc.encode()),
                Some(dcc.clone()),
                "{}",
                dcc.encode()
            );
        }
        assert_eq!(
            Dcc::Send {
                file: "a.txt".into(),
                addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
                port: 5000,
                size: Some(10),
                token: None
            }
            .encode(),
            "SEND a.txt 2130706433 5000 10"
        );
    }

    #[test]
    fn hostile_file_names_are_made_safe() {
        for (hostile, safe) in [
            ("../../.ssh/authorized_keys", "authorized_keys"),
            ("..\\..\\Windows\\System32\\evil.dll", "evil.dll"),
            ("/etc/passwd", "passwd"),
            ("..", "download"),
            ("...", "download"),
            ("", "download"),
            ("/", "download"),
            (".bashrc", "bashrc"),
            ("  .hidden  ", "hidden"),
            ("a\u{1b}[31mb\u{7}.txt", "a_[31mb_.txt"),
            ("photo\u{202e}gpj.exe", "photo_gpj.exe"),
            ("con.txt", "_con.txt"),
            ("NUL", "_NUL"),
            ("a:b*c?.txt", "a_b_c_.txt"),
            ("C:secret", "C_secret"),
            ("trailing dot.", "trailing dot"),
        ] {
            assert_eq!(safe_file_name(hostile), safe, "{hostile:?}");
        }
        let long = format!("{}.tar.gz", "x".repeat(500));
        let cut = safe_file_name(&long);
        let ext = cut.rsplit_once('.').map(|(_, e)| e);
        assert!(cut.len() <= MAX_NAME && ext == Some("gz"), "{cut}");
        let wide = "é".repeat(300);
        assert!(safe_file_name(&wide).len() <= MAX_NAME);
    }

    #[test]
    fn acks_count_modulo_2_32() {
        assert_eq!(ack(1), [0, 0, 0, 1]);
        assert_eq!(ack(0x1_0000_0005), [0, 0, 0, 5]);
        assert_eq!(encode_addr("1.2.3.4".parse().unwrap()), "16909060");
    }
}
