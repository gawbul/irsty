//! Tab completion and fuzzy matching. Pure functions over [`State`] and the
//! input text, so they are easy to test.

use irsty_core::event::NetworkId;
use irsty_core::state::{BufferId, BufferKind, State};
use irsty_proto::casemap::CaseMapping;

use crate::command::{self, COMMANDS};
use crate::keymap::{Action, PRESETS};

/// What completion needs to know besides the text.
#[derive(Clone, Copy, Debug)]
pub struct Context<'a> {
    /// The state.
    pub state: &'a State,
    /// The buffer being typed in.
    pub buffer: Option<BufferId>,
    /// Names of the user's aliases.
    pub aliases: &'a [&'a str],
}

/// Matches for the word before the cursor, best first, each with what to put
/// after it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Matches {
    /// Candidates, best first.
    pub words: Vec<String>,
    /// Text to add after a chosen word: ": " after a nick at the start of a
    /// line (as irssi does), otherwise " ".
    pub suffix: &'static str,
}

/// Finds completions for `word`, which starts at byte `start` of `line`.
pub fn complete(line: &str, start: usize, word: &str, ctx: &Context<'_>) -> Matches {
    let network = ctx
        .buffer
        .and_then(|b| ctx.state.buffer(b))
        .map(|b| b.network);
    let mapping = network
        .and_then(|n| ctx.state.network(n))
        .map_or(CaseMapping::Rfc1459, |n| n.isupport.casemapping);
    let before = line.get(..start).unwrap_or_default();
    let mut suffix = " ";
    let pool: Vec<String> = if start == 0 && word.starts_with('/') {
        let typed = word.trim_start_matches('/');
        let names = COMMANDS
            .iter()
            .flat_map(|s| std::iter::once(s.name).chain(s.aliases.iter().copied()))
            .chain(ctx.aliases.iter().copied());
        return Matches {
            words: prefixed(names.map(str::to_owned), typed, CaseMapping::Ascii)
                .into_iter()
                .map(|n| format!("/{n}"))
                .collect(),
            suffix,
        };
    } else if let Some(rest) = before.strip_prefix('/') {
        let words: Vec<&str> = rest.split_whitespace().collect();
        let name = words.first().and_then(|w| command::canonical(w));
        argument_pool(name, &words, word, network, ctx)
    } else if word.starts_with(['#', '&', '!', '+']) {
        channels(ctx.state, network)
    } else {
        if start == 0 {
            suffix = ": ";
        }
        nicks(ctx.state, ctx.buffer)
    };
    Matches {
        words: prefixed(pool, word, mapping),
        suffix,
    }
}

/// Candidates for a command's arguments.
fn argument_pool(
    name: Option<&str>,
    words: &[&str],
    word: &str,
    network: Option<NetworkId>,
    ctx: &Context<'_>,
) -> Vec<String> {
    let position = words.len();
    let owned = |items: &[&str]| items.iter().map(|&s| s.to_owned()).collect::<Vec<_>>();
    let networks = || {
        ctx.state
            .networks()
            .map(|n| n.name.clone())
            .collect::<Vec<_>>()
    };
    match (name, position) {
        (Some("help"), 1) => COMMANDS.iter().map(|s| s.name.to_owned()).collect(),
        (Some("set"), 1) => irsty_core::config_file::SETTINGS
            .iter()
            .map(|(k, _)| (*k).to_owned())
            .chain(std::iter::once("-reset".to_owned()))
            .collect(),
        (Some("set"), 2) if words.get(1) == Some(&"ui.keymap") => owned(PRESETS),
        (Some("set"), 2) if words.get(1) == Some(&"-reset") => irsty_core::config_file::SETTINGS
            .iter()
            .map(|(k, _)| (*k).to_owned())
            .collect(),
        (Some("connect" | "disconnect"), 1) => networks(),
        (Some("server"), 1) => owned(&["add", "del", "update", "list"])
            .into_iter()
            .chain(networks())
            .collect(),
        (Some("server"), 2) if matches!(words.get(1), Some(&("del" | "update"))) => networks(),
        (Some("window"), 1) => owned(&["next", "prev", "close", "list"]),
        (Some("bind"), 2) => Action::names()
            .filter(|n| *n != "buffer_N")
            .map(str::to_owned)
            .collect(),
        (Some("alias" | "unalias"), 1) => ctx.aliases.iter().map(|&a| a.to_owned()).collect(),
        (Some("join" | "part" | "names" | "topic" | "invite"), _)
            if word.starts_with(['#', '&', '!', '+']) || position == 1 =>
        {
            channels(ctx.state, network)
        }
        _ if word.starts_with(['#', '&', '!', '+']) => channels(ctx.state, network),
        _ => nicks(ctx.state, ctx.buffer),
    }
}

/// Channels we are in on the network.
fn channels(state: &State, network: Option<NetworkId>) -> Vec<String> {
    let Some(net) = network.and_then(|n| state.network(n)) else {
        return Vec::new();
    };
    let mut names: Vec<String> = net
        .channels()
        .filter(|c| c.joined)
        .map(|c| c.name.clone())
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names
}

/// Nicks for the buffer: people who spoke most recently first, then everyone
/// else in the channel alphabetically. In a query, the other person.
fn nicks(state: &State, buffer: Option<BufferId>) -> Vec<String> {
    let Some(buf) = buffer.and_then(|b| state.buffer(b)) else {
        return Vec::new();
    };
    let Some(net) = state.network(buf.network) else {
        return Vec::new();
    };
    let mapping = net.isupport.casemapping;
    let mut out: Vec<String> = Vec::new();
    let push = |nick: &str, out: &mut Vec<String>| {
        if !nick.is_empty() && !net.is_me(nick) && !out.iter().any(|n| mapping.eq(n, nick)) {
            out.push(nick.to_owned());
        }
    };
    match buf.kind {
        BufferKind::Query => push(&buf.name, &mut out),
        BufferKind::Channel => {
            let members: Vec<&str> = net
                .channel(&buf.name)
                .map(|c| c.members().map(|m| m.nick.as_str()).collect())
                .unwrap_or_default();
            for line in buf.lines().iter().rev().take(500) {
                if let Some(sender) = &line.sender
                    && members.iter().any(|m| mapping.eq(m, sender))
                {
                    push(sender, &mut out);
                }
            }
            let mut rest = members;
            rest.sort_by_key(|n| mapping.fold(n).into_owned());
            for nick in rest {
                push(nick, &mut out);
            }
        }
        BufferKind::Server => {}
    }
    out
}

/// The candidates starting with `prefix`, in their original order, compared
/// with `mapping`.
fn prefixed(
    pool: impl IntoIterator<Item = String>,
    prefix: &str,
    mapping: CaseMapping,
) -> Vec<String> {
    let folded = mapping.fold(prefix).into_owned();
    let mut seen: Vec<String> = Vec::new();
    for candidate in pool {
        if mapping.fold(&candidate).starts_with(&folded)
            && !seen.iter().any(|s| mapping.eq(s, &candidate))
        {
            seen.push(candidate);
        }
    }
    seen
}

/// Scores `candidate` against a fuzzy `query`: every query character must
/// appear in order, ignoring case. Higher is better; consecutive runs, word
/// starts and an exact prefix score more, and gaps cost a little. `None`
/// means no match. An empty query matches everything equally.
pub fn fuzzy_score(query: &str, candidate: &str) -> Option<i64> {
    let query: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    let chars: Vec<char> = candidate.chars().collect();
    let mut score = 0i64;
    let mut q = 0;
    let mut last: Option<usize> = None;
    for (i, &c) in chars.iter().enumerate() {
        let Some(&want) = query.get(q) else {
            break;
        };
        if c.to_lowercase().eq(std::iter::once(want)) {
            score += 10;
            match last {
                Some(l) if l + 1 == i => score += 15,
                Some(l) => score -= i64::try_from(i - l - 1).unwrap_or(0).min(10),
                None => {}
            }
            let word_start = i == 0 || chars.get(i - 1).is_some_and(|p| !p.is_alphanumeric());
            if word_start {
                score += 20;
            }
            last = Some(i);
            q += 1;
        }
    }
    if q < query.len() {
        return None;
    }
    let lower: String = candidate.to_lowercase();
    let prefix: String = query.iter().collect();
    if !prefix.is_empty()
        && lower
            .trim_start_matches(['#', '&', '!', '+'])
            .starts_with(&prefix)
    {
        score += 30;
    }
    Some(score)
}

#[cfg(test)]
mod tests {
    use super::*;
    use irsty_core::event::NetEvent;
    use irsty_proto::Message;

    const NET: NetworkId = NetworkId(0);

    fn state() -> State {
        let mut s = State::new(100);
        s.add_network(NET, "libera", "me", Vec::new());
        for line in [
            ":me!u@h JOIN #irsty",
            ":srv 353 me = #irsty :me alice Alfred bob carol",
            ":srv 366 me #irsty :End",
            ":me!u@h JOIN #rust",
            ":bob!b@h PRIVMSG #irsty :hi",
            ":carol!c@h PRIVMSG #irsty :hello",
        ] {
            s.apply(NET, NetEvent::Message(Message::parse(line).unwrap()));
        }
        s
    }

    fn run(s: &State, line: &str, aliases: &[&str]) -> Matches {
        let buffer = s.find_buffer(NET, "#irsty");
        let (start, word) = {
            let start = line.rfind(' ').map_or(0, |i| i + 1);
            (start, &line[start..])
        };
        complete(
            line,
            start,
            word,
            &Context {
                state: s,
                buffer,
                aliases,
            },
        )
    }

    #[test]
    fn nicks_recent_speakers_first() {
        let s = state();
        let m = run(&s, "", &[]);
        assert_eq!(m.words, ["carol", "bob", "Alfred", "alice"]);
        assert_eq!(m.suffix, ": ", "a nick starting the line gets a colon");
        let m = run(&s, "AL", &[]);
        assert_eq!(m.words, ["Alfred", "alice"]);
        let m = run(&s, "thanks b", &[]);
        assert_eq!(
            (m.words.as_slice(), m.suffix),
            (&["bob".to_owned()][..], " ")
        );
        assert!(run(&s, "me", &[]).words.is_empty(), "never ourselves");
    }

    #[test]
    fn commands_settings_networks_and_channels() {
        let s = state();
        let m = run(&s, "/wh", &["whatever"]);
        assert_eq!(m.words, ["/whois", "/whowas", "/who", "/whatever"]);
        assert_eq!(
            run(&s, "/set ui.n", &[]).words,
            ["ui.nicklist", "ui.nick_colours"]
        );
        assert_eq!(run(&s, "/set ui.keymap w", &[]).words, ["weechat"]);
        assert_eq!(run(&s, "/connect l", &[]).words, ["libera"]);
        assert_eq!(run(&s, "/server d", &[]).words, ["del"]);
        assert_eq!(run(&s, "/server del l", &[]).words, ["libera"]);
        assert_eq!(run(&s, "/join #r", &[]).words, ["#rust"]);
        assert_eq!(run(&s, "see #", &[]).words, ["#irsty", "#rust"]);
        assert_eq!(run(&s, "/bind alt-x sw", &[]).words, ["switcher"]);
        assert_eq!(run(&s, "/kick ca", &[]).words, ["carol"]);
        assert_eq!(run(&s, "/window n", &[]).words, ["next"]);
        assert_eq!(run(&s, "/help qu", &[]).words, ["query", "quit"]);
    }

    #[test]
    fn fuzzy_scores() {
        assert!(fuzzy_score("irs", "#irsty").unwrap() > fuzzy_score("irs", "#first-irs").unwrap());
        assert!(fuzzy_score("rs", "#rust").is_some());
        assert!(fuzzy_score("rsx", "#rust").is_none());
        assert!(fuzzy_score("lib", "libera").unwrap() > fuzzy_score("lib", "#oftc-lib").unwrap());
        assert_eq!(fuzzy_score("", "anything"), Some(0));
        assert!(fuzzy_score("RU", "#rust").is_some(), "case is ignored");
    }
}
