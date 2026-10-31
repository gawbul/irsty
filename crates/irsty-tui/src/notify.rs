//! Desktop notifications through the terminal (ADR 0007): an OSC 777 or OSC 9
//! escape sequence, or the bell. These work over SSH and, wrapped for tmux,
//! inside tmux too.
//!
//! The text comes from other people, so every control character is removed
//! before it goes into an escape sequence: nothing in a message can end the
//! sequence early and smuggle in its own.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use irsty_core::config::NotifyMethod;
use irsty_core::state::{BufferId, Notification};

/// Longest title, in characters.
const MAX_TITLE: usize = 80;
/// Longest body, in characters.
const MAX_BODY: usize = 200;
/// Shortest gap between any two notifications.
const GLOBAL_GAP: Duration = Duration::from_secs(1);
/// Shortest gap between notifications for one buffer.
const BUFFER_GAP: Duration = Duration::from_secs(10);

/// Keeps text that is safe inside an escape sequence: no C0 or C1 controls,
/// no DEL, at most `max` characters.
fn clean(text: &str, max: usize) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(max)
        .collect()
}

/// The escape sequence for a notification, or `None` for no notification.
/// `tmux` wraps it so tmux passes it to the outer terminal.
pub fn sequence(method: NotifyMethod, title: &str, body: &str, tmux: bool) -> Option<String> {
    let title = clean(title, MAX_TITLE);
    let body = clean(body, MAX_BODY);
    let raw = match method {
        // OSC 777 separates fields with ';', so the title must not contain one.
        NotifyMethod::Osc777 => format!("\x1b]777;notify;{};{body}\x1b\\", title.replace(';', ",")),
        NotifyMethod::Osc9 if body.is_empty() => format!("\x1b]9;{title}\x1b\\"),
        NotifyMethod::Osc9 => format!("\x1b]9;{title}: {body}\x1b\\"),
        NotifyMethod::Bell => "\x07".to_owned(),
        NotifyMethod::None => return None,
    };
    if tmux && raw != "\x07" {
        // tmux passthrough: DCS tmux; with every ESC doubled.
        return Some(format!(
            "\x1bPtmux;{}\x1b\\",
            raw.replace('\x1b', "\x1b\x1b")
        ));
    }
    Some(raw)
}

/// The title and body for a notification.
pub fn describe(n: &Notification, show_text: bool) -> (String, String) {
    let title = if n.private {
        format!("{} ({})", n.sender, n.network)
    } else {
        format!("{} in {} ({})", n.sender, n.place, n.network)
    };
    let body = if show_text {
        n.text.clone()
    } else {
        String::new()
    };
    (title, body)
}

/// Rate limits notifications so a busy channel can't flood the desktop.
#[derive(Debug, Default)]
pub struct Limiter {
    last: Option<Instant>,
    per_buffer: HashMap<BufferId, Instant>,
}

impl Limiter {
    /// Whether to notify about `buffer` now; records it if so.
    pub fn allow(&mut self, buffer: BufferId, now: Instant) -> bool {
        let too_soon = |at: Option<&Instant>, gap| at.is_some_and(|&t| now.duration_since(t) < gap);
        if too_soon(self.last.as_ref(), GLOBAL_GAP)
            || too_soon(self.per_buffer.get(&buffer), BUFFER_GAP)
        {
            return false;
        }
        self.last = Some(now);
        self.per_buffer.insert(buffer, now);
        self.per_buffer
            .retain(|_, &mut t| now.duration_since(t) < BUFFER_GAP);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_sequences() {
        assert_eq!(
            sequence(NotifyMethod::Osc777, "bob in #c (libera)", "hi", false).unwrap(),
            "\x1b]777;notify;bob in #c (libera);hi\x1b\\"
        );
        assert_eq!(
            sequence(NotifyMethod::Osc9, "bob", "hi", false).unwrap(),
            "\x1b]9;bob: hi\x1b\\"
        );
        assert_eq!(
            sequence(NotifyMethod::Osc9, "bob", "", false).unwrap(),
            "\x1b]9;bob\x1b\\"
        );
        assert_eq!(
            sequence(NotifyMethod::Bell, "x", "y", true).unwrap(),
            "\x07"
        );
        assert_eq!(sequence(NotifyMethod::None, "x", "y", false), None);
        assert_eq!(
            sequence(NotifyMethod::Osc9, "a", "b", true).unwrap(),
            "\x1bPtmux;\x1b\x1b]9;a: b\x1b\x1b\\\x1b\\"
        );
    }

    /// Nothing in a message can end the sequence and start its own.
    #[test]
    fn hostile_text_cannot_escape() {
        let evil = "x\x1b\\\x1b]52;c;cGF3bmVk\x07\u{9c}\u{7f}y";
        let seq = sequence(NotifyMethod::Osc777, "a;b", evil, false).unwrap();
        assert_eq!(seq.matches('\x1b').count(), 2, "only our own ESCs: {seq:?}");
        assert!(!seq.contains('\x07') && !seq.contains('\u{9c}') && !seq.contains('\u{7f}'));
        assert!(seq.starts_with("\x1b]777;notify;a,b;"), "{seq:?}");
        let long = sequence(
            NotifyMethod::Osc9,
            &"t".repeat(500),
            &"b".repeat(500),
            false,
        )
        .unwrap();
        assert!(long.chars().count() < MAX_TITLE + MAX_BODY + 10);
    }

    #[test]
    fn limits_the_rate() {
        let mut limiter = Limiter::default();
        let start = Instant::now();
        let (a, b) = (BufferId(1), BufferId(2));
        assert!(limiter.allow(a, start));
        assert!(!limiter.allow(b, start), "one a second overall");
        assert!(limiter.allow(b, start + Duration::from_secs(2)));
        assert!(
            !limiter.allow(a, start + Duration::from_secs(5)),
            "ten seconds per buffer"
        );
        assert!(limiter.allow(a, start + Duration::from_secs(11)));
    }

    #[test]
    fn describes_notifications() {
        let n = Notification {
            buffer: BufferId(1),
            network: "libera".into(),
            place: "#irsty".into(),
            sender: "bob".into(),
            text: "gawbul: hi".into(),
            private: false,
        };
        assert_eq!(
            describe(&n, true),
            ("bob in #irsty (libera)".into(), "gawbul: hi".into())
        );
        let private = Notification { private: true, ..n };
        assert_eq!(
            describe(&private, false),
            ("bob (libera)".into(), String::new())
        );
    }
}
