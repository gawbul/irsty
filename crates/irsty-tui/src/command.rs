//! Slash commands: parsing typed input into [`Command`]s.

use irsty_core::config::{IgnoreKind, IgnoreRule};
use irsty_core::config_file::{NetworkOptions, NewServer};

/// A command name, its usage and what it does. [`COMMANDS`] drives parsing,
/// `/help` and the check that `docs/COMMANDS.md` is complete.
#[derive(Clone, Copy, Debug)]
pub struct Spec {
    /// Name without the slash.
    pub name: &'static str,
    /// Other names for it.
    pub aliases: &'static [&'static str],
    /// Usage, shown by `/help`.
    pub usage: &'static str,
    /// One-line description.
    pub help: &'static str,
}

macro_rules! spec {
    ($name:literal, [$($alias:literal),*], $usage:literal, $help:literal) => {
        Spec { name: $name, aliases: &[$($alias),*], usage: $usage, help: $help }
    };
}

/// Every command, in `/help` order.
pub const COMMANDS: &[Spec] = &[
    spec!(
        "join",
        ["j"],
        "/join <#channel>[,<#channel>...] [key[,key...]]",
        "Join one or more channels."
    ),
    spec!(
        "part",
        ["leave"],
        "/part [#channel] [reason]",
        "Leave a channel (the current one by default)."
    ),
    spec!(
        "msg",
        ["m"],
        "/msg <target> <text>",
        "Send a message to a nick or channel."
    ),
    spec!(
        "query",
        ["q"],
        "/query <nick> [text]",
        "Open a private conversation, optionally sending a message."
    ),
    spec!(
        "me",
        [],
        "/me <action>",
        "Send an action to the current buffer."
    ),
    spec!(
        "reply",
        [],
        "/reply <nick> <text>",
        "Reply to the newest message from <nick> here, addressed to them."
    ),
    spec!(
        "react",
        [],
        "/react <nick> <reaction>",
        "React to the newest message from <nick> here, usually with an emoji."
    ),
    spec!(
        "unreact",
        [],
        "/unreact <nick> <reaction>",
        "Take back a reaction to the newest message from <nick> here."
    ),
    spec!(
        "notice",
        [],
        "/notice <target> <text>",
        "Send a notice to a nick or channel."
    ),
    spec!(
        "ctcp",
        [],
        "/ctcp <target> <command> [arguments]",
        "Send a CTCP request, such as VERSION or PING."
    ),
    spec!("nick", [], "/nick <newnick>", "Change your nick."),
    spec!(
        "whois",
        ["wi"],
        "/whois <nick> [nick]",
        "Show who someone is; the replies appear here. Give the nick twice for their idle time."
    ),
    spec!(
        "whowas",
        [],
        "/whowas <nick>",
        "Show who used a nick that is now gone."
    ),
    spec!(
        "who",
        [],
        "/who [mask]",
        "List users matching a mask or channel (the current channel by default)."
    ),
    spec!(
        "names",
        [],
        "/names [#channel]",
        "List who is in a channel."
    ),
    spec!(
        "topic",
        ["t"],
        "/topic [#channel] [new topic]",
        "Show or change a channel's topic."
    ),
    spec!(
        "mode",
        [],
        "/mode [target] <modes> [arguments]",
        "Change or show modes of a channel (the current one by default) or yourself."
    ),
    spec!(
        "op",
        [],
        "/op <nick> [nick...]",
        "Give channel operator status."
    ),
    spec!(
        "deop",
        [],
        "/deop <nick> [nick...]",
        "Take channel operator status away."
    ),
    spec!("voice", [], "/voice <nick> [nick...]", "Give voice."),
    spec!(
        "devoice",
        [],
        "/devoice <nick> [nick...]",
        "Take voice away."
    ),
    spec!(
        "kick",
        ["k"],
        "/kick <nick> [reason]",
        "Remove someone from the current channel."
    ),
    spec!(
        "ban",
        [],
        "/ban [nick|mask]",
        "Ban a nick (by host, when known) or mask from the current channel, or list bans."
    ),
    spec!(
        "unban",
        [],
        "/unban <mask>",
        "Remove a ban from the current channel."
    ),
    spec!(
        "invite",
        [],
        "/invite <nick> [#channel]",
        "Invite someone to a channel (the current one by default)."
    ),
    spec!(
        "away",
        [],
        "/away [message]",
        "Mark yourself away on the current network, or back without a message."
    ),
    spec!(
        "list",
        [],
        "/list [arguments]",
        "List channels on the network. Big networks have many."
    ),
    spec!(
        "raw",
        ["quote"],
        "/raw <line>",
        "Send a raw IRC line. Passwords are hidden in the echo."
    ),
    spec!(
        "connect",
        [],
        "/connect [network]",
        "Connect a network (the current one by default), or reconnect it."
    ),
    spec!(
        "disconnect",
        [],
        "/disconnect [network] [reason]",
        "Disconnect a network and stay disconnected."
    ),
    spec!(
        "server",
        ["network"],
        "/server add|del|update|list|[network] ...",
        "Manage networks in your config, or connect one (see /help server)."
    ),
    spec!(
        "close",
        ["wc"],
        "/close [reason]",
        "Close the current buffer, leaving the channel if joined."
    ),
    spec!(
        "url",
        [],
        "/url [number]",
        "List the newest links in this buffer, or open one (1 is the newest) in your browser."
    ),
    spec!(
        "clear",
        [],
        "/clear",
        "Clear the current buffer's scrollback."
    ),
    spec!(
        "window",
        ["buffer", "w"],
        "/window <number>|next|prev|close|list",
        "Switch buffers by number, or close or list them."
    ),
    spec!(
        "ignore",
        [],
        "/ignore [mask [messages|notices|ctcps|events|invites...] [-network name] [-channel #c]]",
        "Hide someone, or list ignores."
    ),
    spec!("unignore", [], "/unignore <mask>", "Stop ignoring a mask."),
    spec!(
        "highlight",
        ["hilight"],
        "/highlight [word | -regex pattern]",
        "Highlight lines with a word or pattern, or list them."
    ),
    spec!(
        "unhighlight",
        [],
        "/unhighlight <word or pattern>",
        "Remove a highlight."
    ),
    spec!(
        "alias",
        [],
        "/alias [name [/command ...[; /command ...]]]",
        "Define an alias, or list them. $* is every argument, $1 to $9 one each, $nick, $channel and $network what they say."
    ),
    spec!("unalias", [], "/unalias <name>", "Remove an alias."),
    spec!(
        "set",
        [],
        "/set [setting [value]] | /set -reset <setting>",
        "Show or change settings; changes are saved to your config."
    ),
    spec!(
        "bind",
        [],
        "/bind [key [action]] | /bind -reset <key>",
        "Show or change key bindings, saved to your config. /bind lists them; actions are in docs/KEYBINDINGS.md."
    ),
    spec!(
        "unbind",
        [],
        "/unbind <key>",
        "Make a key do nothing (same as /bind <key> none)."
    ),
    spec!(
        "quit",
        ["exit"],
        "/quit [reason]",
        "Disconnect from every network and exit."
    ),
    spec!(
        "help",
        [],
        "/help [command]",
        "List commands, or show one command's usage."
    ),
];

/// Extra help for `/server`, which has subcommands.
pub const SERVER_HELP: &[&str] = &[
    "/server add <network> <host>[:port] [options]",
    "  Add a server, creating the network if it is new (a new network needs -nick).",
    "  Options: -nick <nick>[,<alt>...]  -autojoin <#a>[,<#b>...]  -notls  -insecure",
    "           -sasl plain|scram-sha-256|external|none  -account <name>  -keyring <entry>",
    "/server del <network> [host]   Remove a network, or one of its servers.",
    "/server update <network> [options]   Change a network's nicks, autojoin or SASL.",
    "/server list   Show configured networks.",
    "/server [network]   Connect, as /connect.",
];

/// What `/window` should do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Window {
    /// Go to the buffer at this position.
    Number(usize),
    /// The next buffer.
    Next,
    /// The previous buffer.
    Prev,
    /// Close the current buffer.
    Close,
    /// List buffers.
    List,
}

/// `/server` subcommands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerCommand {
    /// Connect a network, as `/connect`.
    Connect(Option<String>),
    /// Add a server.
    Add {
        /// Network name.
        network: String,
        /// The server.
        server: NewServer,
        /// Network options.
        options: NetworkOptions,
    },
    /// Remove a network or one server.
    Del {
        /// Network name.
        network: String,
        /// A server's host, to remove just that server.
        host: Option<String>,
    },
    /// Change a network's options.
    Update {
        /// Network name.
        network: String,
        /// The changes.
        options: NetworkOptions,
    },
    /// List networks.
    List,
}

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
    /// `/notice`.
    Notice {
        /// Nick or channel.
        target: String,
        /// Text.
        text: String,
    },
    /// `/ctcp`.
    Ctcp {
        /// Nick or channel.
        target: String,
        /// The CTCP command, upper-cased.
        command: String,
        /// Its arguments.
        args: Option<String>,
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
    /// `/reply`.
    Reply {
        /// Whose newest message.
        nick: String,
        /// What to say.
        text: String,
    },
    /// `/react` and `/unreact`.
    React {
        /// Whose newest message.
        nick: String,
        /// The reaction.
        reaction: String,
        /// Take it back.
        remove: bool,
    },
    /// `/nick`.
    Nick(String),
    /// `/whois`: the arguments as typed.
    Whois(String),
    /// `/whowas`.
    Whowas(String),
    /// `/who`.
    Who(Option<String>),
    /// `/names`.
    Names(Option<String>),
    /// `/topic`.
    Topic {
        /// Channel, or the current one.
        channel: Option<String>,
        /// New topic; `None` shows it.
        text: Option<String>,
    },
    /// `/mode`.
    Mode {
        /// Channel or nick, or the current channel.
        target: Option<String>,
        /// Modes and their arguments.
        modes: String,
    },
    /// `/op`, `/deop`, `/voice`, `/devoice`.
    Members {
        /// `o` or `v`.
        mode: char,
        /// Give (`true`) or take.
        give: bool,
        /// Who.
        nicks: Vec<String>,
    },
    /// `/kick`.
    Kick {
        /// Who.
        nick: String,
        /// Why.
        reason: Option<String>,
    },
    /// `/ban`: a nick or mask, or list bans.
    Ban(Option<String>),
    /// `/unban`.
    Unban(String),
    /// `/invite`.
    Invite {
        /// Who.
        nick: String,
        /// Channel, or the current one.
        channel: Option<String>,
    },
    /// `/away`.
    Away(Option<String>),
    /// `/list`.
    List(Option<String>),
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
    /// `/server` and `/network`.
    Server(ServerCommand),
    /// `/close`.
    Close(Option<String>),
    /// `/clear`.
    Clear,
    /// `/url`: list links, or open one by number.
    Url(Option<usize>),
    /// `/window` and `/buffer`.
    Window(Window),
    /// `/ignore`: a rule to add, or `None` to list.
    Ignore(Option<IgnoreRule>),
    /// `/unignore`.
    Unignore(String),
    /// `/highlight`: a word or regex to add, or `None` to list.
    Highlight(Option<(String, bool)>),
    /// `/unhighlight`.
    Unhighlight(String),
    /// `/alias`.
    Alias {
        /// Name; `None` lists aliases.
        name: Option<String>,
        /// Commands; `None` shows the alias.
        commands: Option<Vec<String>>,
    },
    /// `/unalias`.
    Unalias(String),
    /// `/set`.
    Set {
        /// Setting; `None` lists them.
        key: Option<String>,
        /// New value; `None` shows the setting.
        value: Option<String>,
        /// Reset to the default.
        reset: bool,
    },
    /// `/bind`.
    Bind {
        /// Key; `None` lists bindings.
        key: Option<String>,
        /// Action; `None` shows the binding.
        action: Option<String>,
        /// Go back to the preset's binding.
        reset: bool,
    },
    /// `/unbind`.
    Unbind(String),
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
    /// Missing or wrong arguments; carries the usage.
    Usage(&'static str),
    /// Something specific is wrong.
    Invalid(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(name) => write!(f, "Unknown command /{name}; try /help"),
            Self::Usage(usage) => write!(f, "Usage: {usage}"),
            Self::Invalid(message) => f.write_str(message),
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

fn is_channel(s: &str) -> bool {
    s.starts_with(['#', '&', '!', '+'])
}

fn list(s: &str) -> Vec<String> {
    s.split(',')
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The canonical name for a command or one of its aliases.
pub fn canonical(name: &str) -> Option<&'static str> {
    let name = name.to_lowercase();
    COMMANDS
        .iter()
        .find(|s| s.name == name || s.aliases.contains(&name.as_str()))
        .map(|s| s.name)
}

/// Parses a line typed into the input. Text not starting with `/` (or starting
/// with `//`, to send a literal slash) is plain text.
#[allow(
    clippy::too_many_lines,
    reason = "one arm per command reads better than indirection"
)]
pub fn parse(input: &str) -> Result<Command, ParseError> {
    let Some(body) = input.strip_prefix('/') else {
        return Ok(Command::Say(input.to_owned()));
    };
    if body.starts_with('/') {
        return Ok(Command::Say(body.to_owned()));
    }
    let (typed, rest) = word(body);
    let name = canonical(typed).ok_or_else(|| ParseError::Unknown(typed.to_lowercase()))?;
    let need = |value: &str| {
        if value.trim().is_empty() {
            Err(usage(name))
        } else {
            Ok(value.trim().to_owned())
        }
    };
    let (first, tail) = word(rest);
    Ok(match name {
        "join" => Command::Join {
            channels: need(first)?,
            keys: opt(tail),
        },
        // A first word that looks like a channel is the channel; anything else
        // is a reason for the current channel.
        "part" if is_channel(first) => Command::Part {
            channel: Some(first.to_owned()),
            reason: opt(tail),
        },
        "part" => Command::Part {
            channel: None,
            reason: opt(rest),
        },
        "msg" | "notice" => {
            if first.is_empty() || tail.is_empty() {
                return Err(usage(name));
            }
            let (target, text) = (first.to_owned(), tail.to_owned());
            if name == "msg" {
                Command::Msg { target, text }
            } else {
                Command::Notice { target, text }
            }
        }
        "ctcp" => {
            let (command, args) = word(tail);
            if first.is_empty() || command.is_empty() {
                return Err(usage(name));
            }
            Command::Ctcp {
                target: first.to_owned(),
                command: command.to_ascii_uppercase(),
                args: opt(args),
            }
        }
        "query" => Command::Query {
            nick: need(first)?,
            text: opt(tail),
        },
        "me" => Command::Me(need(rest)?),
        "reply" if !first.is_empty() && !tail.trim().is_empty() => Command::Reply {
            nick: first.to_owned(),
            text: tail.to_owned(),
        },
        "react" | "unreact" if !first.is_empty() && !tail.trim().is_empty() => Command::React {
            nick: first.to_owned(),
            reaction: tail.trim().to_owned(),
            remove: name == "unreact",
        },
        "reply" | "react" | "unreact" => return Err(usage(name)),
        "nick" => Command::Nick(need(first)?),
        "whois" => Command::Whois(need(rest)?),
        "whowas" => Command::Whowas(need(first)?),
        "who" => Command::Who(opt(rest)),
        "names" => Command::Names(opt(first)),
        "topic" if is_channel(first) => Command::Topic {
            channel: Some(first.to_owned()),
            text: opt(tail),
        },
        "topic" => Command::Topic {
            channel: None,
            text: opt(rest),
        },
        "mode" if first.starts_with(['+', '-']) || first.is_empty() => Command::Mode {
            target: None,
            modes: rest.trim().to_owned(),
        },
        "mode" => Command::Mode {
            target: Some(first.to_owned()),
            modes: tail.trim().to_owned(),
        },
        "op" | "deop" | "voice" | "devoice" => {
            let nicks: Vec<String> = rest.split_whitespace().map(str::to_owned).collect();
            if nicks.is_empty() {
                return Err(usage(name));
            }
            Command::Members {
                mode: if name.contains("op") { 'o' } else { 'v' },
                give: !name.starts_with("de"),
                nicks,
            }
        }
        "kick" => Command::Kick {
            nick: need(first)?,
            reason: opt(tail),
        },
        "ban" => Command::Ban(opt(first)),
        "unban" => Command::Unban(need(first)?),
        "invite" => Command::Invite {
            nick: need(first)?,
            channel: opt(word(tail).0),
        },
        "away" => Command::Away(opt(rest)),
        "list" => Command::List(opt(rest)),
        "raw" => Command::Raw(need(rest)?),
        "connect" => Command::Connect(opt(first)),
        "disconnect" => Command::Disconnect {
            network: opt(first),
            reason: opt(tail),
        },
        "server" => Command::Server(parse_server(first, tail)?),
        "close" => Command::Close(opt(rest)),
        "clear" => Command::Clear,
        "url" if first.is_empty() => Command::Url(None),
        "url" => Command::Url(Some(first.parse().map_err(|_| usage(name))?)),
        "window" => Command::Window(match first.to_lowercase().as_str() {
            "next" | "n" => Window::Next,
            "prev" | "p" => Window::Prev,
            "close" | "c" => Window::Close,
            "list" | "l" | "" => Window::List,
            n => Window::Number(n.parse().map_err(|_| usage(name))?),
        }),
        "ignore" if first.is_empty() => Command::Ignore(None),
        "ignore" => Command::Ignore(Some(parse_ignore(first, tail)?)),
        "unignore" => Command::Unignore(need(first)?),
        "highlight" if first.is_empty() => Command::Highlight(None),
        "highlight" if first == "-regex" => Command::Highlight(Some((need(tail)?, true))),
        "highlight" => Command::Highlight(Some((need(first)?, false))),
        "unhighlight" => Command::Unhighlight(need(rest)?),
        "alias" => Command::Alias {
            name: opt(first).map(|n| n.trim_start_matches('/').to_lowercase()),
            commands: opt(tail).map(|t| {
                t.split(';')
                    .map(str::trim)
                    .filter(|c| !c.is_empty())
                    .map(str::to_owned)
                    .collect()
            }),
        },
        "unalias" => Command::Unalias(need(first)?.trim_start_matches('/').to_lowercase()),
        "set" if first == "-reset" => Command::Set {
            key: Some(need(tail)?),
            value: None,
            reset: true,
        },
        "set" => Command::Set {
            key: opt(first),
            value: opt(tail),
            reset: false,
        },
        "bind" if first == "-reset" => Command::Bind {
            key: Some(need(tail)?),
            action: None,
            reset: true,
        },
        "bind" => Command::Bind {
            key: opt(first),
            action: opt(tail),
            reset: false,
        },
        "unbind" => Command::Unbind(need(first)?),
        "quit" => Command::Quit(opt(rest)),
        "help" => Command::Help(opt(rest).map(|c| c.trim_start_matches('/').to_lowercase())),
        _ => return Err(ParseError::Unknown(name.to_owned())),
    })
}

fn parse_server(sub: &str, rest: &str) -> Result<ServerCommand, ParseError> {
    let bad = |message: &str| ParseError::Invalid(format!("{message}; see /help server"));
    Ok(match sub.to_lowercase().as_str() {
        "add" => {
            let (network, rest) = word(rest);
            let (address, rest) = word(rest);
            if network.is_empty() || address.is_empty() {
                return Err(bad("/server add needs a network name and a host"));
            }
            let (host, port) = split_host_port(address).ok_or_else(|| bad("bad host or port"))?;
            let (options, flags) = parse_options(rest)?;
            ServerCommand::Add {
                network: network.to_owned(),
                server: NewServer {
                    host,
                    port,
                    plain: flags.contains(&"notls"),
                    insecure: flags.contains(&"insecure"),
                },
                options,
            }
        }
        "del" | "delete" | "remove" | "rm" => {
            let (network, rest) = word(rest);
            if network.is_empty() {
                return Err(bad("/server del needs a network name"));
            }
            ServerCommand::Del {
                network: network.to_owned(),
                host: opt(word(rest).0),
            }
        }
        "update" | "set" => {
            let (network, rest) = word(rest);
            if network.is_empty() {
                return Err(bad("/server update needs a network name"));
            }
            let (options, flags) = parse_options(rest)?;
            if !flags.is_empty() {
                return Err(bad(
                    "-notls and -insecure belong to a server: del and add it again",
                ));
            }
            ServerCommand::Update {
                network: network.to_owned(),
                options,
            }
        }
        "list" | "ls" => ServerCommand::List,
        "" => ServerCommand::Connect(None),
        _ => ServerCommand::Connect(Some(sub.to_owned())),
    })
}

/// `host`, `host:port`, `[v6]:port` or a bare IPv6 address.
fn split_host_port(address: &str) -> Option<(String, Option<u16>)> {
    if let Some(rest) = address.strip_prefix('[') {
        let (host, after) = rest.split_once(']')?;
        let port = match after.strip_prefix(':') {
            Some(port) => Some(port.parse().ok()?),
            None if after.is_empty() => None,
            None => return None,
        };
        return Some((host.to_owned(), port));
    }
    match address.rsplit_once(':') {
        // More than one colon without brackets is a bare IPv6 address.
        Some((host, _)) if host.contains(':') => Some((address.to_owned(), None)),
        Some((host, port)) => Some((host.to_owned(), Some(port.parse().ok()?))),
        None => Some((address.to_owned(), None)),
    }
}

/// Parses `-option value` pairs and bare flags for `/server`.
fn parse_options(mut rest: &str) -> Result<(NetworkOptions, Vec<&'static str>), ParseError> {
    let mut options = NetworkOptions::default();
    let mut flags = Vec::new();
    loop {
        let (flag, after) = word(rest);
        if flag.is_empty() {
            return Ok((options, flags));
        }
        match flag {
            "-notls" => flags.push("notls"),
            "-insecure" => flags.push("insecure"),
            _ => {
                let (value, after) = word(after);
                if value.is_empty() {
                    return Err(ParseError::Invalid(format!(
                        "{flag} needs a value; see /help server"
                    )));
                }
                match flag {
                    "-nick" | "-nicks" => options.nicks = Some(list(value)),
                    "-autojoin" => options.autojoin = Some(list(value)),
                    "-sasl" => options.sasl = Some(value.to_lowercase()),
                    "-account" => options.account = Some(value.to_owned()),
                    "-keyring" => options.keyring = Some(value.to_owned()),
                    _ => {
                        return Err(ParseError::Invalid(format!(
                            "unknown option {flag}; see /help server"
                        )));
                    }
                }
                rest = after;
                continue;
            }
        }
        rest = after;
    }
}

fn parse_ignore(mask: &str, rest: &str) -> Result<IgnoreRule, ParseError> {
    let mut rule = IgnoreRule {
        mask: mask.to_owned(),
        what: Vec::new(),
        networks: Vec::new(),
        channels: Vec::new(),
    };
    let mut words = rest.split_whitespace();
    while let Some(w) = words.next() {
        match w {
            "-network" | "-channel" => {
                let value = words.next().ok_or_else(|| usage("ignore"))?;
                let target = if w == "-network" {
                    &mut rule.networks
                } else {
                    &mut rule.channels
                };
                target.extend(list(value));
            }
            kind => {
                let kind = IgnoreKind::ALL
                    .into_iter()
                    .find(|k| k.name() == kind.to_lowercase())
                    .ok_or_else(|| usage("ignore"))?;
                rule.what.push(kind);
            }
        }
    }
    if rule.what.is_empty() {
        rule.what = IgnoreKind::ALL.to_vec();
    }
    Ok(rule)
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
            let canonical = canonical(name).ok_or_else(|| ParseError::Unknown(name.to_owned()))?;
            if canonical == "server" {
                return Ok(SERVER_HELP.iter().map(|&l| l.to_owned()).collect());
            }
            let spec = COMMANDS
                .iter()
                .find(|s| s.name == canonical)
                .ok_or_else(|| ParseError::Unknown(name.to_owned()))?;
            let mut lines = vec![spec.usage.to_owned(), format!("  {}", spec.help)];
            if !spec.aliases.is_empty() {
                let names: Vec<String> = spec.aliases.iter().map(|a| format!("/{a}")).collect();
                lines.push(format!("  Also: {}", names.join(", ")));
            }
            Ok(lines)
        }
    }
}

/// Expands an alias's commands with the arguments it was given. `$*` is every
/// argument, `$1` to `$9` one each, and `$nick`, `$channel` and `$network`
/// come from `context`.
pub fn expand(commands: &[String], args: &str, context: &[(&str, &str)]) -> Vec<String> {
    let words: Vec<&str> = args.split_whitespace().collect();
    commands
        .iter()
        .map(|command| {
            let mut out = String::new();
            let mut rest = command.as_str();
            while let Some(at) = rest.find('$') {
                out.push_str(&rest[..at]);
                let after = &rest[at + 1..];
                if let Some(tail) = after.strip_prefix('*') {
                    out.push_str(args.trim());
                    rest = tail;
                } else if let Some(d) = after
                    .chars()
                    .next()
                    .and_then(|c| c.to_digit(10))
                    .filter(|&d| d > 0)
                {
                    let index = usize::try_from(d).unwrap_or(0) - 1;
                    out.push_str(words.get(index).copied().unwrap_or(""));
                    rest = &after[1..];
                } else if let Some((name, value)) =
                    context.iter().find(|(name, _)| after.starts_with(name))
                {
                    out.push_str(value);
                    rest = &after[name.len()..];
                } else {
                    out.push('$');
                    rest = after;
                }
            }
            out.push_str(rest);
            out
        })
        .collect()
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
    fn parses_basic_commands() {
        assert_eq!(
            parse("/j #a,#b k1"),
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
        assert_eq!(parse("/me waves"), Ok(Command::Me("waves".into())));
        assert_eq!(
            parse("/reply alice sounds good"),
            Ok(Command::Reply {
                nick: "alice".into(),
                text: "sounds good".into()
            })
        );
        assert_eq!(
            parse("/unreact bob 👍 "),
            Ok(Command::React {
                nick: "bob".into(),
                reaction: "👍".into(),
                remove: true
            })
        );
        assert!(parse("/reply alice").is_err());
        assert!(parse("/react bob").is_err());
        assert_eq!(parse("/nick new extra"), Ok(Command::Nick("new".into())));
        assert_eq!(
            parse("/quote PRIVMSG #a :x"),
            Ok(Command::Raw("PRIVMSG #a :x".into()))
        );
        assert_eq!(
            parse("/server"),
            Ok(Command::Server(ServerCommand::Connect(None)))
        );
        assert_eq!(
            parse("/server libera"),
            Ok(Command::Server(ServerCommand::Connect(Some(
                "libera".into()
            ))))
        );
        assert_eq!(
            parse("/exit see you"),
            Ok(Command::Quit(Some("see you".into())))
        );
        assert_eq!(parse("/help /JOIN"), Ok(Command::Help(Some("join".into()))));
    }

    #[test]
    fn parses_the_full_command_set() {
        assert_eq!(parse("/wi alice"), Ok(Command::Whois("alice".into())));
        assert_eq!(parse("/whois a a"), Ok(Command::Whois("a a".into())));
        assert_eq!(parse("/who"), Ok(Command::Who(None)));
        assert_eq!(
            parse("/notice #c hello"),
            Ok(Command::Notice {
                target: "#c".into(),
                text: "hello".into()
            })
        );
        assert_eq!(
            parse("/ctcp bob version"),
            Ok(Command::Ctcp {
                target: "bob".into(),
                command: "VERSION".into(),
                args: None
            })
        );
        assert_eq!(
            parse("/topic #c new topic"),
            Ok(Command::Topic {
                channel: Some("#c".into()),
                text: Some("new topic".into())
            })
        );
        assert_eq!(
            parse("/t"),
            Ok(Command::Topic {
                channel: None,
                text: None
            })
        );
        assert_eq!(
            parse("/mode +i"),
            Ok(Command::Mode {
                target: None,
                modes: "+i".into()
            })
        );
        assert_eq!(
            parse("/mode #c +k key"),
            Ok(Command::Mode {
                target: Some("#c".into()),
                modes: "+k key".into()
            })
        );
        assert_eq!(
            parse("/devoice a b"),
            Ok(Command::Members {
                mode: 'v',
                give: false,
                nicks: vec!["a".into(), "b".into()]
            })
        );
        assert_eq!(
            parse("/op a"),
            Ok(Command::Members {
                mode: 'o',
                give: true,
                nicks: vec!["a".into()]
            })
        );
        assert_eq!(parse("/ban"), Ok(Command::Ban(None)));
        assert_eq!(parse("/away"), Ok(Command::Away(None)));
        assert_eq!(
            parse("/invite carol"),
            Ok(Command::Invite {
                nick: "carol".into(),
                channel: None
            })
        );
        assert_eq!(parse("/clear"), Ok(Command::Clear));
        assert_eq!(parse("/url"), Ok(Command::Url(None)));
        assert_eq!(parse("/url 2"), Ok(Command::Url(Some(2))));
        assert!(matches!(parse("/url two"), Err(ParseError::Usage(_))));
        assert_eq!(parse("/w 3"), Ok(Command::Window(Window::Number(3))));
        assert_eq!(parse("/buffer next"), Ok(Command::Window(Window::Next)));
        assert!(matches!(
            parse("/window sideways"),
            Err(ParseError::Usage(_))
        ));
    }

    #[test]
    fn parses_server_management() {
        let Ok(Command::Server(ServerCommand::Add {
            network,
            server,
            options,
        })) = parse(
            "/server add libera irc.libera.chat:6697 -nick gawbul,gawbul_ -autojoin #irsty -sasl SCRAM-SHA-256 -account gawbul -keyring libera",
        )
        else {
            panic!();
        };
        assert_eq!(network, "libera");
        assert_eq!(
            server,
            NewServer {
                host: "irc.libera.chat".into(),
                port: Some(6697),
                plain: false,
                insecure: false
            }
        );
        assert_eq!(options.nicks, Some(vec!["gawbul".into(), "gawbul_".into()]));
        assert_eq!(options.sasl.as_deref(), Some("scram-sha-256"));
        assert_eq!(options.keyring.as_deref(), Some("libera"));

        let Ok(Command::Server(ServerCommand::Add { server, .. })) =
            parse("/network add local [::1]:6667 -notls -insecure -nick me")
        else {
            panic!();
        };
        assert_eq!(server.host, "::1");
        assert_eq!(server.port, Some(6667));
        assert!(server.plain && server.insecure);

        assert_eq!(
            split_host_port("2001:db8::1"),
            Some(("2001:db8::1".into(), None))
        );
        assert_eq!(split_host_port("h:x"), None);
        assert_eq!(
            parse("/server del libera irc.eu.libera.chat"),
            Ok(Command::Server(ServerCommand::Del {
                network: "libera".into(),
                host: Some("irc.eu.libera.chat".into())
            }))
        );
        assert_eq!(
            parse("/server list"),
            Ok(Command::Server(ServerCommand::List))
        );
        for bad in [
            "/server add libera",
            "/server add libera h -nick",
            "/server add libera h -frob x",
            "/server update libera -notls",
            "/server del",
        ] {
            assert!(matches!(parse(bad), Err(ParseError::Invalid(_))), "{bad}");
        }
    }

    #[test]
    fn parses_ignores_highlights_aliases_and_settings() {
        let Ok(Command::Ignore(Some(rule))) =
            parse("/ignore *!*@spam.example messages notices -network libera -channel #a,#b")
        else {
            panic!();
        };
        assert_eq!(rule.what, [IgnoreKind::Messages, IgnoreKind::Notices]);
        assert_eq!(rule.networks, ["libera"]);
        assert_eq!(rule.channels, ["#a", "#b"]);
        let Ok(Command::Ignore(Some(rule))) = parse("/ignore bob") else {
            panic!();
        };
        assert_eq!(rule.what.len(), IgnoreKind::ALL.len());
        assert!(matches!(
            parse("/ignore bob sometimes"),
            Err(ParseError::Usage(_))
        ));
        assert_eq!(parse("/ignore"), Ok(Command::Ignore(None)));
        assert_eq!(
            parse("/highlight -regex \\bsteve\\b"),
            Ok(Command::Highlight(Some(("\\bsteve\\b".into(), true))))
        );
        assert_eq!(
            parse("/hilight irsty"),
            Ok(Command::Highlight(Some(("irsty".into(), false))))
        );
        assert_eq!(
            parse("/alias hi /msg $channel hi ; /me waves"),
            Ok(Command::Alias {
                name: Some("hi".into()),
                commands: Some(vec!["/msg $channel hi".into(), "/me waves".into()])
            })
        );
        assert_eq!(parse("/unalias /HI"), Ok(Command::Unalias("hi".into())));
        assert_eq!(
            parse("/set ui.mouse on"),
            Ok(Command::Set {
                key: Some("ui.mouse".into()),
                value: Some("on".into()),
                reset: false
            })
        );
        assert_eq!(
            parse("/set -reset ui.mouse"),
            Ok(Command::Set {
                key: Some("ui.mouse".into()),
                value: None,
                reset: true
            })
        );
    }

    #[test]
    fn parses_bindings() {
        assert_eq!(
            parse("/bind alt-x switcher"),
            Ok(Command::Bind {
                key: Some("alt-x".into()),
                action: Some("switcher".into()),
                reset: false
            })
        );
        assert_eq!(
            parse("/bind -reset ctrl-g"),
            Ok(Command::Bind {
                key: Some("ctrl-g".into()),
                action: None,
                reset: true
            })
        );
        assert_eq!(
            parse("/unbind ctrl-g"),
            Ok(Command::Unbind("ctrl-g".into()))
        );
        assert!(matches!(parse("/unbind"), Err(ParseError::Usage(_))));
    }

    #[test]
    fn expands_aliases() {
        let context = [
            ("nick", "gawbul"),
            ("channel", "#irsty"),
            ("network", "libera"),
        ];
        assert_eq!(
            expand(
                &[
                    "/msg $1 hi from $nick in $channel: $*".into(),
                    "/me $2$3".into()
                ],
                "bob there",
                &context
            ),
            ["/msg bob hi from gawbul in #irsty: bob there", "/me there"]
        );
        // Unknown variables and a lone $ are left alone.
        assert_eq!(
            expand(&["/say $5 $x $".into()], "", &context),
            ["/say  $x $"]
        );
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
        assert!(matches!(parse("/kick"), Err(ParseError::Usage(_))));
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
        assert_eq!(help(Some("j")).unwrap()[2], "  Also: /j");
        assert_eq!(help(Some("network")).unwrap(), SERVER_HELP);
        assert!(help(Some("nope")).is_err());
    }

    #[test]
    fn names_and_aliases_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for spec in COMMANDS {
            for name in std::iter::once(&spec.name).chain(spec.aliases) {
                assert!(seen.insert(*name), "/{name} is defined twice");
            }
        }
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
