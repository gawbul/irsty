//! IRCv3 multi-line messages (`draft/multiline`).
//!
//! A multi-line message is a batch of `PRIVMSG` or `NOTICE` lines to one
//! target. Lines are joined with a line break, except a line tagged
//! `draft/multiline-concat`, which continues the one before it. This module
//! splits text into such lines, groups them into batches within the server's
//! limits, builds the messages, and puts received lines back together.

use crate::Message;
use crate::split::split_text;

/// The capability, and the batch type.
pub const CAP: &str = "draft/multiline";

/// The tag on a line that continues the previous one with no line break.
pub const CONCAT: &str = "draft/multiline-concat";

/// The server's limits, from the capability's value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Most bytes of text in one batch, counting a byte per line break.
    pub max_bytes: usize,
    /// Most lines in one batch, if limited.
    pub max_lines: Option<usize>,
}

impl Limits {
    /// Parses a value such as `max-bytes=4096,max-lines=24`. `max-bytes` is
    /// required; unknown keys are ignored.
    pub fn parse(value: &str) -> Option<Self> {
        let mut max_bytes = None;
        let mut max_lines = None;
        for entry in value.split(',') {
            let (key, value) = entry.split_once('=').unwrap_or((entry, ""));
            match key {
                "max-bytes" => max_bytes = value.parse().ok().filter(|&n| n > 0),
                "max-lines" => max_lines = value.parse().ok().filter(|&n| n > 0),
                _ => {}
            }
        }
        Some(Self {
            max_bytes: max_bytes?,
            max_lines,
        })
    }
}

/// One line of a multi-line message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Piece<'a> {
    /// Its text.
    pub text: &'a str,
    /// It continues the previous line, with no line break between.
    pub concat: bool,
}

/// Splits text into lines of at most `budget` bytes. Each line of the text
/// becomes a piece; a long one is split, and the pieces after the first
/// continue it (spaces at the splits are kept, so joining restores it).
/// Blank lines at the start and end are dropped.
pub fn pieces(text: &str, budget: usize) -> Vec<Piece<'_>> {
    let budget = budget.max(2);
    let lines: Vec<&str> = text
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect();
    let first = lines.iter().position(|l| !l.trim().is_empty());
    let last = lines.iter().rposition(|l| !l.trim().is_empty());
    let (Some(first), Some(last)) = (first, last) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for &line in lines.get(first..=last).unwrap_or_default() {
        if line.is_empty() {
            out.push(Piece {
                text: line,
                concat: false,
            });
            continue;
        }
        // One byte spare for the space a split consumes, which is put back.
        let chunks = split_text(line, budget - 1);
        let base = line.as_ptr() as usize;
        let starts: Vec<usize> = chunks
            .iter()
            .map(|c| (c.as_ptr() as usize).saturating_sub(base))
            .chain(std::iter::once(line.len()))
            .collect();
        for (i, pair) in starts.windows(2).enumerate() {
            if let [start, end] = *pair
                && let Some(text) = line.get(start..end)
            {
                out.push(Piece {
                    text,
                    concat: i > 0,
                });
            }
        }
    }
    out
}

/// Groups pieces into batches that fit `limits`. The first piece of a batch
/// never continues the previous one (that is a separate message), and a batch
/// of blank lines only is dropped, since servers refuse it.
pub fn group<'a>(pieces: &[Piece<'a>], limits: Limits) -> Vec<Vec<Piece<'a>>> {
    let mut batches: Vec<Vec<Piece<'a>>> = Vec::new();
    let mut current: Vec<Piece<'a>> = Vec::new();
    let mut bytes = 0;
    for &piece in pieces {
        let join = usize::from(!current.is_empty() && !piece.concat);
        let full = limits.max_lines.is_some_and(|max| current.len() >= max)
            || bytes + join + piece.text.len() > limits.max_bytes;
        if full && !current.is_empty() {
            batches.push(std::mem::take(&mut current));
            bytes = 0;
        }
        let join = usize::from(!current.is_empty() && !piece.concat);
        bytes += join + piece.text.len();
        current.push(Piece {
            text: piece.text,
            concat: piece.concat && !current.is_empty(),
        });
    }
    batches.push(current);
    batches.retain(|b| b.iter().any(|p| !p.text.trim().is_empty()));
    batches
}

/// The messages for one batch: `BATCH +reference`, a `command` (`PRIVMSG`
/// or `NOTICE`) per piece, then `BATCH -reference`. Tags for the whole
/// message (a label, a reply) belong on the first, which is returned first.
pub fn messages(
    reference: &str,
    command: &str,
    target: &str,
    pieces: &[Piece<'_>],
) -> Vec<Message> {
    let mut out = Vec::with_capacity(pieces.len() + 2);
    out.push(
        Message::new("BATCH")
            .param(format!("+{reference}"))
            .param(CAP)
            .param(target),
    );
    for piece in pieces {
        let mut msg = Message::new(command)
            .tag("batch", reference)
            .param(target)
            .param(piece.text);
        if piece.concat {
            msg = msg.tag(CONCAT, "");
        }
        out.push(msg);
    }
    out.push(Message::new("BATCH").param(format!("-{reference}")));
    out
}

/// Puts received lines back together: each `(text, concat)` either starts a
/// new line or, with `concat`, continues the last one.
pub fn assemble<'a>(parts: impl IntoIterator<Item = (&'a str, bool)>) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for (text, concat) in parts {
        match lines.last_mut() {
            Some(last) if concat => last.push_str(text),
            _ => lines.push(text.to_owned()),
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts<'a>(pieces: &[Piece<'a>]) -> Vec<(&'a str, bool)> {
        pieces.iter().map(|p| (p.text, p.concat)).collect()
    }

    #[test]
    fn parses_limits() {
        assert_eq!(
            Limits::parse("max-bytes=4096,max-lines=24,future=1"),
            Some(Limits {
                max_bytes: 4096,
                max_lines: Some(24)
            })
        );
        assert_eq!(
            Limits::parse("max-bytes=100").map(|l| l.max_lines),
            Some(None)
        );
        assert_eq!(Limits::parse("max-lines=5"), None, "max-bytes is required");
        assert_eq!(Limits::parse("max-bytes=0"), None);
        assert_eq!(Limits::parse("max-bytes=lots"), None);
        assert_eq!(Limits::parse(""), None);
    }

    #[test]
    fn splits_lines_and_keeps_spaces() {
        let p = pieces("\n\none\r\n\ntwo words here\n\n", 8);
        assert_eq!(
            texts(&p),
            [
                ("one", false),
                ("", false),
                ("two ", false),
                ("words ", true),
                ("here", true),
            ]
        );
        let joined = assemble(texts(&p));
        assert_eq!(joined, ["one", "", "two words here"]);
        assert!(pieces("\n \n", 10).is_empty());
        // Splits never break a grapheme, and every piece fits.
        let long = "é".repeat(20);
        let p = pieces(&long, 7);
        assert!(p.iter().all(|x| x.text.len() <= 7), "{p:?}");
        assert_eq!(assemble(texts(&p)), [long]);
    }

    #[test]
    fn groups_within_limits() {
        let p = pieces("aaaa\nbbbb\ncccc\n\n", 100);
        let limits = Limits {
            max_bytes: 9,
            max_lines: None,
        };
        // "aaaa\nbbbb" is 9 bytes; "cccc" starts a new batch.
        let g = group(&p, limits);
        assert_eq!(g.len(), 2);
        assert_eq!(texts(&g[0]), [("aaaa", false), ("bbbb", false)]);
        assert_eq!(texts(&g[1]), [("cccc", false)]);

        let limits = Limits {
            max_bytes: 1000,
            max_lines: Some(2),
        };
        let p = pieces("one two three", 7);
        let g = group(&p, limits);
        assert_eq!(g.len(), 2);
        assert_eq!(
            texts(&g[1]),
            [("three", false)],
            "a new batch never starts with a continuation"
        );

        // A batch of only blank lines is dropped.
        let p = [
            Piece {
                text: "x",
                concat: false,
            },
            Piece {
                text: "",
                concat: false,
            },
        ];
        let limits = Limits {
            max_bytes: 1000,
            max_lines: Some(1),
        };
        assert_eq!(group(&p, limits).len(), 1);
    }

    #[test]
    fn builds_messages() {
        let p = pieces("hi\n\nthere you", 6);
        let wire: Vec<String> = messages("ml1", "PRIVMSG", "#c", &p)
            .iter()
            .map(|m| m.to_wire().unwrap())
            .collect();
        assert_eq!(
            wire,
            [
                "BATCH +ml1 draft/multiline #c",
                "@batch=ml1 PRIVMSG #c hi",
                "@batch=ml1 PRIVMSG #c :",
                "@batch=ml1 PRIVMSG #c :there ",
                "@batch=ml1;draft/multiline-concat PRIVMSG #c you",
                "BATCH -ml1",
            ]
        );
    }
}
