//! Hides credentials in text we show or log about our own messages.
//!
//! Anything the user sends with `/raw`, and any message to a services bot that
//! carries a password, goes through here before it reaches scrollback or a log.

use irsty_proto::Message;

/// Shown in place of a secret.
pub const REDACTED: &str = "<redacted>";

/// Services bots whose commands may carry passwords.
const SERVICES: &[&str] = &["nickserv", "ns", "chanserv", "cs", "q", "x", "authserv"];

/// Services commands whose arguments include a password.
const SECRET_COMMANDS: &[&str] = &[
    "identify",
    "id",
    "register",
    "ghost",
    "recover",
    "regain",
    "release",
    "auth",
    "login",
    "set password",
    "setpass",
];

/// Redacts a raw protocol line as typed with `/raw`.
pub fn raw_line(line: &str) -> String {
    let Ok(msg) = Message::parse(line) else {
        return line.to_owned();
    };
    let command = msg.command.to_ascii_uppercase();
    let redacted = match command.as_str() {
        "PASS" => Some(format!("PASS {REDACTED}")),
        "AUTHENTICATE"
            if !matches!(
                msg.params.first().map(String::as_str),
                Some("+" | "*" | "PLAIN" | "EXTERNAL" | "SCRAM-SHA-256")
            ) =>
        {
            Some(format!("AUTHENTICATE {REDACTED}"))
        }
        "OPER" => Some(format!(
            "OPER {} {REDACTED}",
            msg.params.first().map_or("", String::as_str)
        )),
        "PRIVMSG" | "NOTICE" | "NICKSERV" | "NS" | "CHANSERV" | "CS" => {
            let (target, text) = match command.as_str() {
                "PRIVMSG" | "NOTICE" => (
                    msg.params.first().cloned().unwrap_or_default(),
                    msg.params.get(1..).unwrap_or_default().join(" "),
                ),
                _ => (command.to_lowercase(), msg.params.join(" ")),
            };
            let safe = services_text(&target, &text);
            (safe != text).then(|| match command.as_str() {
                "PRIVMSG" | "NOTICE" => format!("{command} {target} :{safe}"),
                _ => format!("{command} {safe}"),
            })
        }
        _ => None,
    };
    redacted.unwrap_or_else(|| line.to_owned())
}

/// Redacts the text of a message to `target` if it is a services command that
/// carries a password, keeping the command name visible.
pub fn services_text(target: &str, text: &str) -> String {
    let nick = target.split('@').next().unwrap_or(target);
    if !SERVICES.iter().any(|s| s.eq_ignore_ascii_case(nick)) {
        return text.to_owned();
    }
    let lower = text.trim_start().to_lowercase();
    for command in SECRET_COMMANDS {
        if lower == *command || lower.starts_with(&format!("{command} ")) {
            let keep = text.trim_start().get(..command.len()).unwrap_or(command);
            return format!("{keep} {REDACTED}");
        }
    }
    text.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_raw_credentials() {
        assert_eq!(raw_line("PASS hunter2"), "PASS <redacted>");
        assert_eq!(raw_line("pass :hunter 2"), "PASS <redacted>");
        assert_eq!(
            raw_line("AUTHENTICATE amFjawBqYWNr"),
            "AUTHENTICATE <redacted>"
        );
        assert_eq!(raw_line("AUTHENTICATE PLAIN"), "AUTHENTICATE PLAIN");
        assert_eq!(raw_line("AUTHENTICATE +"), "AUTHENTICATE +");
        assert_eq!(raw_line("OPER admin s3cret"), "OPER admin <redacted>");
        assert_eq!(
            raw_line("PRIVMSG NickServ :IDENTIFY acct pw"),
            "PRIVMSG NickServ :IDENTIFY <redacted>"
        );
        assert_eq!(
            raw_line("NICKSERV identify pw"),
            "NICKSERV identify <redacted>"
        );
        assert_eq!(
            raw_line("NS REGISTER pw me@example.com"),
            "NS REGISTER <redacted>"
        );
    }

    #[test]
    fn leaves_ordinary_lines_alone() {
        for line in [
            "PRIVMSG #chan :identify yourself",
            "PRIVMSG NickServ :INFO bob",
            "JOIN #x key",
            "NOTICE bob :hi",
            ":::",
        ] {
            assert_eq!(raw_line(line), line);
        }
    }

    #[test]
    fn services_messages() {
        assert_eq!(
            services_text("NickServ", "IDENTIFY pw"),
            "IDENTIFY <redacted>"
        );
        assert_eq!(
            services_text("nickserv@services.libera.chat", "id pw"),
            "id <redacted>"
        );
        assert_eq!(services_text("Q", "AUTH acct pw"), "AUTH <redacted>");
        assert_eq!(
            services_text("NickServ", "set password new"),
            "set password <redacted>"
        );
        assert_eq!(services_text("NickServ", "HELP"), "HELP");
        assert_eq!(services_text("bob", "IDENTIFY pw"), "IDENTIFY pw");
    }
}
