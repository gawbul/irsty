//! Slash commands: parsing typed input into [`Command`]s.

/// A command name, its usage and what it does. [`COMMANDS`] drives parsing,
/// `/help` and the check that `docs/COMMANDS.md` is complete.
#[derive(Clone, Copy, Debug)]
pub struct Spec {
    /// Name without the slash.
    pub name: &'static str,
    /// Usage, shown by `/help`.
    pub usage: &'static str,
    /// One-line description.
    pub help: &'static str,
}

/// Every command, in `/help` order.
pub const COMMANDS: &[Spec] = &[
    Spec {
        name: "join",
        usage: "/join <#channel>[,<#channel>...] [key[,key...]]",
        help: "Join one or more channels.",
    },
    Spec {
        name: "part",
        usage: "/part [#channel] [reason]",
        help: "Leave a channel (the current one by default).",
    },
    Spec {
        name: "msg",
        usage: "/msg <target> <text>",
        help: "Send a message to a nick or channel.",
    },
    Spec {
        name: "query",
        usage: "/query <nick> [text]",
        help: "Open a private conversation, optionally sending a message.",
    },
    Spec {
        name: "me",
        usage: "/me <action>",
        help: "Send an action to the current buffer.",
    },
    Spec {
        name: "nick",
        usage: "/nick <newnick>",
        help: "Change your nick.",
    },
    Spec {
        name: "raw",
        usage: "/raw <line>",
        help: "Send a raw IRC line. Passwords are hidden in the echo. Alias: /quote.",
    },
    Spec {
        name: "connect",
        usage: "/connect [network]",
        help: "Connect a network (the current one by default), or reconnect it.",
    },
    Spec {
        name: "disconnect",
        usage: "/disconnect [network] [reason]",
        help: "Disconnect a network and stay disconnected.",
    },
    Spec {
        name: "close",
        usage: "/close [reason]",
        help: "Close the current buffer, leaving the channel if joined.",
    },
    Spec {
        name: "quit",
        usage: "/quit [reason]",
        help: "Disconnect from every network and exit.",
    },
    Spec {
        name: "help",
        usage: "/help [command]",
        help: "List commands, or show one command's usage.",
    },
];

/// A parsed command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Plain text for the current buffer.
    Say(String),
    /// `/join`.
    Join {
        /// Comma-separated channels.
        channels: String,
        /// Comma-separated keys.
        keys: Option<String>,
    },
    /// `/part`.
    Part {
        /// Channel, or the current one.
        channel: Option<String>,
        /// Reason.
        reason: Option<String>,
    },
    /// `/msg`.
    Msg {
        /// Nick or channel.
        target: String,
        /// Text.
        text: String,
    },
    /// `/query`.
    Query {
        /// Nick.
        nick: String,
        /// Optional first message.
        text: Option<String>,
    },
    /// `/me`.
    Me(String),
    /// `/nick`.
    Nick(String),
    /// `/raw` or `/quote`.
    Raw(String),
    /// `/connect`.
    Connect(Option<String>),
    /// `/disconnect`.
    Disconnect {
        /// Network, or the current one.
        network: Option<String>,
        /// Reason.
        reason: Option<String>,
    },
    /// `/close`.
    Close(Option<String>),
    /// `/quit`.
    Quit(Option<String>),
    /// `/help`.
    Help(Option<String>),
}

/// Why input could not be parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// No such command.
    Unknown(String),
    /// Missing arguments; carries the usage.
    Usage(&'static str),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(name) => write!(f, "Unknown command /{name}; try /help"),
            Self::Usage(usage) => write!(f, "Usage: {usage}"),
        }
    }
}

/// Splits off the first word.
fn word(s: &str) -> (&str, &str) {
    let s = s.trim_start();
    match s.split_once(' ') {
        Some((first, rest)) => (first, rest.trim_start()),
        None => (s, ""),
    }
}

fn opt(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_owned())
}

fn usage(name: &str) -> ParseError {
    let spec = COMMANDS.iter().find(|s| s.name == name);
    ParseError::Usage(spec.map_or("", |s| s.usage))
}

/// Parses a line typed into the input. Text not starting with `/` (or starting
/// with `//`, to send a literal slash) is plain text.
pub fn parse(input: &str) -> Result<Command, ParseError> {
    let Some(body) = input.strip_prefix('/') else {
        return Ok(Command::Say(input.to_owned()));
    };
    if body.starts_with('/') {
        return Ok(Command::Say(body.to_owned()));
    }
    let (name, rest) = word(body);
    let name = name.to_lowercase();
    let need = |value: &str| {
        if value.is_empty() {
            Err(usage(&name))
        } else {
            Ok(value.to_owned())
        }
    };
    Ok(match name.as_str() {
        "join" | "j" => {
            let (channels, keys) = word(rest);
            Command::Join {
                channels: need(channels)?,
                keys: opt(keys),
            }
        }
        "part" | "leave" => {
            let (first, tail) = word(rest);
            // A first word that looks like a channel is the channel; anything else
            // is a reason for the current channel.
            if first.starts_with(['#', '&', '!', '+']) {
                Command::Part {
                    channel: Some(first.to_owned()),
                    reason: opt(tail),
                }
            } else {
                Command::Part {
                    channel: None,
                    reason: opt(rest),
                }
            }
        }
        "msg" | "m" => {
            let (target, text) = word(rest);
            if target.is_empty() || text.is_empty() {
                return Err(usage("msg"));
            }
            Command::Msg {
                target: target.to_owned(),
                text: text.to_owned(),
            }
        }
        "query" | "q" => {
            let (nick, text) = word(rest);
            Command::Query {
                nick: need(nick)?,
                text: opt(text),
            }
        }
        "me" => Command::Me(need(rest)?),
        "nick" => Command::Nick(need(word(rest).0)?),
        "raw" | "quote" => Command::Raw(need(rest.trim())?),
        "connect" | "server" => Command::Connect(opt(rest)),
        "disconnect" => {
            let (network, reason) = word(rest);
            Command::Disconnect {
                network: opt(network),
                reason: opt(reason),
            }
        }
        "close" | "wc" => Command::Close(opt(rest)),
        "quit" | "exit" => Command::Quit(opt(rest)),
        "help" => Command::Help(opt(rest).map(|c| c.trim_start_matches('/').to_lowercase())),
        _ => return Err(ParseError::Unknown(name)),
    })
}

/// The `/help` text: every command, or one command's usage.
pub fn help(command: Option<&str>) -> Result<Vec<String>, ParseError> {
    match command {
        None => Ok(
            std::iter::once("Commands (/help <command> for usage):".to_owned())
                .chain(
                    COMMANDS
                        .iter()
                        .map(|s| format!("  /{:<11} {}", s.name, s.help)),
                )
                .collect(),
        ),
        Some(name) => {
            let spec = COMMANDS
                .iter()
                .find(|s| s.name == name)
                .ok_or_else(|| ParseError::Unknown(name.to_owned()))?;
            Ok(vec![spec.usage.to_owned(), format!("  {}", spec.help)])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_and_escapes() {
        assert_eq!(parse("hello"), Ok(Command::Say("hello".into())));
        assert_eq!(
            parse("//not a command"),
            Ok(Command::Say("/not a command".into()))
        );
        assert_eq!(parse(""), Ok(Command::Say(String::new())));
    }

    #[test]
    fn parses_commands() {
        assert_eq!(
            parse("/join #a,#b k1"),
            Ok(Command::Join {
                channels: "#a,#b".into(),
                keys: Some("k1".into())
            })
        );
        assert_eq!(
            parse("/PART #a bye now"),
            Ok(Command::Part {
                channel: Some("#a".into()),
                reason: Some("bye now".into())
            })
        );
        assert_eq!(
            parse("/part going home"),
            Ok(Command::Part {
                channel: None,
                reason: Some("going home".into())
            })
        );
        assert_eq!(
            parse("/msg bob  hi there "),
            Ok(Command::Msg {
                target: "bob".into(),
                text: "hi there ".into()
            })
        );
        assert_eq!(
            parse("/query bob"),
            Ok(Command::Query {
                nick: "bob".into(),
                text: None
            })
        );
        assert_eq!(parse("/me waves"), Ok(Command::Me("waves".into())));
        assert_eq!(parse("/nick new extra"), Ok(Command::Nick("new".into())));
        assert_eq!(
            parse("/quote PRIVMSG #a :x"),
            Ok(Command::Raw("PRIVMSG #a :x".into()))
        );
        assert_eq!(parse("/connect"), Ok(Command::Connect(None)));
        assert_eq!(
            parse("/server libera"),
            Ok(Command::Connect(Some("libera".into())))
        );
        assert_eq!(
            parse("/disconnect libera brb"),
            Ok(Command::Disconnect {
                network: Some("libera".into()),
                reason: Some("brb".into())
            })
        );
        assert_eq!(parse("/close"), Ok(Command::Close(None)));
        assert_eq!(
            parse("/quit see you"),
            Ok(Command::Quit(Some("see you".into())))
        );
        assert_eq!(parse("/help /JOIN"), Ok(Command::Help(Some("join".into()))));
    }

    #[test]
    fn errors() {
        assert_eq!(
            parse("/frobnicate"),
            Err(ParseError::Unknown("frobnicate".into()))
        );
        assert_eq!(
            parse("/join").unwrap_err().to_string(),
            format!("Usage: {}", COMMANDS[0].usage)
        );
        assert!(matches!(parse("/msg bob"), Err(ParseError::Usage(_))));
        assert!(matches!(parse("/me"), Err(ParseError::Usage(_))));
        assert!(matches!(parse("/raw  "), Err(ParseError::Usage(_))));
        assert_eq!(
            ParseError::Unknown("x".into()).to_string(),
            "Unknown command /x; try /help"
        );
    }

    #[test]
    fn help_text() {
        let all = help(None).unwrap();
        assert_eq!(all.len(), COMMANDS.len() + 1);
        assert_eq!(help(Some("me")).unwrap()[0], "/me <action>");
        assert!(help(Some("nope")).is_err());
    }

    #[test]
    fn docs_list_every_command() {
        let docs = include_str!("../../../docs/COMMANDS.md");
        for spec in COMMANDS {
            assert!(
                docs.contains(&format!("`/{}", spec.name)),
                "docs/COMMANDS.md is missing /{}",
                spec.name
            );
        }
    }
}
