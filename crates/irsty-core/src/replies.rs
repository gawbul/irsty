//! Readable text for numeric replies to user commands: WHOIS, WHOWAS, WHO,
//! LIST and channel mode queries. Pure functions over the parameters; where
//! the lines go is [`crate::state`]'s business.

use jiff::Timestamp;

/// WHOIS replies, which all name the nick in their second parameter.
pub const WHOIS: &[&str] = &[
    "276", "301", "307", "311", "312", "313", "317", "319", "320", "330", "335", "338", "378",
    "379", "671",
];

/// The end of a WHOIS.
pub const END_OF_WHOIS: &str = "318";

/// WHOWAS replies and their end.
pub const WHOWAS: &[&str] = &["312", "314"];
/// The end of a WHOWAS.
pub const END_OF_WHOWAS: &str = "369";

fn param(params: &[String], i: usize) -> &str {
    params.get(i).map_or("", String::as_str)
}

fn last(params: &[String]) -> &str {
    params.last().map_or("", String::as_str)
}

/// `3725` becomes `1h 2m 5s`.
pub fn duration(seconds: u64) -> String {
    let (d, h, m, s) = (
        seconds / 86_400,
        seconds / 3600 % 24,
        seconds / 60 % 60,
        seconds % 60,
    );
    let parts: Vec<String> = [(d, "d"), (h, "h"), (m, "m"), (s, "s")]
        .iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, unit)| format!("{n}{unit}"))
        .collect();
    if parts.is_empty() {
        "0s".into()
    } else {
        parts.join(" ")
    }
}

fn date(epoch: &str) -> Option<String> {
    let secs: i64 = epoch.parse().ok()?;
    let time = Timestamp::from_second(secs).ok()?;
    Some(time.strftime("%Y-%m-%d %H:%M UTC").to_string())
}

/// One WHOIS or WHOWAS line. The first line names the user; the rest are
/// indented details.
pub fn whois(command: &str, params: &[String]) -> Option<String> {
    let nick = param(params, 1);
    Some(match command {
        "311" | "314" => {
            let verb = if command == "311" { "is" } else { "was" };
            format!(
                "{nick} {verb} {}@{} ({})",
                param(params, 2),
                param(params, 3),
                last(params)
            )
        }
        "312" => format!("  server: {} ({})", param(params, 2), last(params)),
        "313" => "  is an IRC operator".into(),
        "317" => {
            let idle = param(params, 2).parse().map(duration).unwrap_or_default();
            match date(param(params, 3)) {
                Some(signon) if params.len() > 4 => {
                    format!("  idle: {idle}, signed on {signon}")
                }
                _ => format!("  idle: {idle}"),
            }
        }
        "319" => format!("  channels: {}", last(params)),
        "330" => format!("  account: {}", param(params, 2)),
        "301" => format!("  away: {}", last(params)),
        "671" => "  using a secure connection".into(),
        "338" if params.len() > 3 => format!("  actual host: {}", param(params, 2)),
        // 276 certfp, 307 registered, 320 special, 335 bot, 378 host,
        // 379 modes and anything else: the server's own words.
        _ => format!("  {}", last(params)),
    })
}

/// A WHO reply (352): `nick (user@host) [flags] realname`.
pub fn who(params: &[String]) -> String {
    let realname = last(params)
        .split_once(' ')
        .map_or(last(params), |(_, name)| name);
    format!(
        "{} ({}@{}) [{}] {realname}",
        param(params, 5),
        param(params, 2),
        param(params, 3),
        param(params, 6)
    )
}

/// A LIST entry (322): `#channel (users): topic`.
pub fn list_entry(params: &[String]) -> String {
    let topic = last(params);
    if params.len() > 3 && !topic.is_empty() {
        format!("{} ({}): {topic}", param(params, 1), param(params, 2))
    } else {
        format!("{} ({})", param(params, 1), param(params, 2))
    }
}

/// Channel information replies: the channel they are about, and the text.
pub fn channel_info(command: &str, params: &[String]) -> Option<(String, String)> {
    let channel = param(params, 1).to_owned();
    let text = match command {
        "324" => format!(
            "Modes for {channel}: {}",
            params.get(2..).unwrap_or_default().join(" ")
        ),
        "329" => format!(
            "{channel} was created {}",
            date(param(params, 2)).unwrap_or_else(|| "at an unknown time".into())
        ),
        "331" => format!("No topic is set for {channel}"),
        "341" => format!("Inviting {} to {}", param(params, 1), param(params, 2)),
        "367" => {
            let by = match (params.len() > 3, date(param(params, 4))) {
                (true, Some(when)) => format!(" (by {}, {when})", param(params, 3)),
                (true, None) => format!(" (by {})", param(params, 3)),
                _ => String::new(),
            };
            format!("{channel} ban: {}{by}", param(params, 2))
        }
        "368" => format!("End of {channel} ban list"),
        _ => return None,
    };
    // 341 names the channel third.
    let channel = if command == "341" {
        param(params, 2).to_owned()
    } else {
        channel
    };
    Some((channel, text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(params: &[&str]) -> Vec<String> {
        params.iter().map(|&s| s.to_owned()).collect()
    }

    #[test]
    fn formats_whois() {
        assert_eq!(
            whois(
                "311",
                &p(&["me", "alice", "a", "host.example", "*", "Alice Liddell"])
            ),
            Some("alice is a@host.example (Alice Liddell)".into())
        );
        assert_eq!(
            whois(
                "312",
                &p(&["me", "alice", "irc.example.net", "Example server"])
            ),
            Some("  server: irc.example.net (Example server)".into())
        );
        assert_eq!(
            whois(
                "317",
                &p(&[
                    "me",
                    "alice",
                    "3725",
                    "1700000000",
                    "seconds idle, signon time"
                ])
            ),
            Some("  idle: 1h 2m 5s, signed on 2023-11-14 22:13 UTC".into())
        );
        assert_eq!(
            whois("319", &p(&["me", "alice", "@#irsty +#rust"])),
            Some("  channels: @#irsty +#rust".into())
        );
        assert_eq!(
            whois("330", &p(&["me", "alice", "alice_acct", "is logged in as"])),
            Some("  account: alice_acct".into())
        );
        assert_eq!(
            whois("378", &p(&["me", "alice", "is connecting from *@1.2.3.4"])),
            Some("  is connecting from *@1.2.3.4".into())
        );
        assert_eq!(
            whois("314", &p(&["me", "bob", "b", "h", "*", "Bob"])),
            Some("bob was b@h (Bob)".into())
        );
    }

    #[test]
    fn formats_other_replies() {
        assert_eq!(duration(0), "0s");
        assert_eq!(duration(90_061), "1d 1h 1m 1s");
        assert_eq!(
            who(&p(&[
                "me",
                "#c",
                "a",
                "host",
                "srv",
                "alice",
                "H@",
                "0 Alice Liddell"
            ])),
            "alice (a@host) [H@] Alice Liddell"
        );
        assert_eq!(
            list_entry(&p(&["me", "#irsty", "12", "A terminal client"])),
            "#irsty (12): A terminal client"
        );
        assert_eq!(list_entry(&p(&["me", "#quiet", "3", ""])), "#quiet (3)");
        assert_eq!(
            channel_info("324", &p(&["me", "#c", "+ntk", "key"])),
            Some(("#c".into(), "Modes for #c: +ntk key".into()))
        );
        assert_eq!(
            channel_info("367", &p(&["me", "#c", "*!*@spam", "op", "1700000000"])),
            Some((
                "#c".into(),
                "#c ban: *!*@spam (by op, 2023-11-14 22:13 UTC)".into()
            ))
        );
        assert_eq!(
            channel_info("341", &p(&["me", "carol", "#c"])),
            Some(("#c".into(), "Inviting carol to #c".into()))
        );
        assert_eq!(channel_info("999", &p(&["me", "#c"])), None);
    }
}
