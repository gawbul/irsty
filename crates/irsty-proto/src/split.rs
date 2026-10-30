//! Splitting long text into chunks that fit in a single message.

use unicode_segmentation::UnicodeSegmentation;

use crate::MAX_BODY_LEN;

/// Bytes available for the text of a `command target :text` message sent by
/// `nick!user@host`, as the server will relay it to others.
///
/// Uses a pessimistic host length when the real one is unknown, because the server
/// prepends the full source when relaying and lines over 512 bytes are truncated.
pub fn text_budget(
    nick: &str,
    user: Option<&str>,
    host: Option<&str>,
    command: &str,
    target: &str,
) -> usize {
    const UNKNOWN_USER: usize = 10;
    const UNKNOWN_HOST: usize = 63;
    // ":" nick "!" user "@" host " " command " " target " :"
    let overhead = 1
        + nick.len()
        + 1
        + user.map_or(UNKNOWN_USER, str::len)
        + 1
        + host.map_or(UNKNOWN_HOST, str::len)
        + 1
        + command.len()
        + 1
        + target.len()
        + 2;
    MAX_BODY_LEN.saturating_sub(overhead)
}

/// Splits `text` into chunks of at most `max_bytes` bytes.
///
/// Splits never fall inside a grapheme cluster. Where possible, a chunk ends at the
/// last space within the budget and that space is consumed. A single grapheme
/// larger than the budget is split at character boundaries instead, and every
/// chunk contains at least one character, so this always makes progress.
/// Concatenating the chunks with a space wherever one was consumed restores the
/// original text.
pub fn split_text(text: &str, max_bytes: usize) -> Vec<&str> {
    let mut chunks = Vec::new();
    let mut rest = text;
    while rest.len() > max_bytes {
        let mut end = 0;
        let mut last_space = None;
        for (i, g) in rest.grapheme_indices(true) {
            // A space exactly at the budget is a valid split point: it is consumed.
            if g == " " && i <= max_bytes {
                last_space = Some(i);
            }
            if i + g.len() > max_bytes {
                break;
            }
            end = i + g.len();
        }
        if end == 0 {
            // The first grapheme alone exceeds the budget.
            end = rest
                .char_indices()
                .map(|(i, c)| i + c.len_utf8())
                .take_while(|&e| e <= max_bytes)
                .last()
                .unwrap_or_else(|| rest.chars().next().map_or(rest.len(), char::len_utf8));
        }
        match last_space {
            // Only consume a space that separates text; a trailing space is kept.
            Some(space) if space > 0 && space + 1 < rest.len() => {
                chunks.push(&rest[..space]);
                rest = &rest[space + 1..];
            }
            _ => {
                chunks.push(&rest[..end]);
                rest = &rest[end..];
            }
        }
    }
    if !rest.is_empty() || chunks.is_empty() {
        chunks.push(rest);
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_spaces() {
        assert_eq!(
            split_text("hello brave new world", 11),
            ["hello brave", "new world"]
        );
        assert_eq!(split_text("short", 100), ["short"]);
        assert_eq!(split_text("", 10), [""]);
        // A space at the very end is content, not a separator.
        assert_eq!(split_text("abcd ", 4), ["abcd", " "]);
    }

    #[test]
    fn hard_splits_long_words_on_graphemes() {
        assert_eq!(split_text("abcdefgh", 3), ["abc", "def", "gh"]);
        // "e" + combining acute is one grapheme of 3 bytes; never split inside it.
        assert_eq!(split_text("ae\u{301}b", 3), ["a", "e\u{301}", "b"]);
        assert_eq!(split_text("ééé", 3), ["é", "é", "é"]);
    }

    #[test]
    fn oversized_grapheme_falls_back_to_chars() {
        let family = "👨\u{200d}👩\u{200d}👧";
        let chunks = split_text(family, 4);
        assert!(chunks.iter().all(|c| c.len() <= 4 && !c.is_empty()));
        assert_eq!(chunks.concat(), family);
        // Budget smaller than one character still makes progress.
        assert_eq!(split_text("€€", 1), ["€", "€"]);
    }

    #[test]
    fn budget() {
        let b = text_budget("nick", Some("user"), Some("host"), "PRIVMSG", "#chan");
        assert_eq!(b, 510 - ":nick!user@host PRIVMSG #chan :".len());
        assert!(text_budget("n", None, None, "PRIVMSG", "#c") < b);
    }
}
