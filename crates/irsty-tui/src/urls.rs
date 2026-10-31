//! Finding URLs in text, and opening them in a browser.

use std::ops::Range;

/// Schemes that start a URL, and `www.`, which implies http.
const STARTS: &[&str] = &["https://", "http://", "ircs://", "irc://", "www."];

/// Byte ranges of the URLs in `text`. A URL starts at the beginning of a word
/// and runs to whitespace or a quote or angle bracket; trailing punctuation
/// is left off unless it closes a bracket inside the URL, so a Wikipedia link
/// keeps its `)` while a sentence's full stop is dropped.
pub fn find(text: &str) -> Vec<Range<usize>> {
    let mut found = Vec::new();
    let mut from = 0;
    while from < text.len() {
        let Some((start, _)) = STARTS
            .iter()
            .filter_map(|s| find_word_start(text, from, s).map(|i| (i, *s)))
            .min_by_key(|&(i, _)| i)
        else {
            break;
        };
        let rest = text.get(start..).unwrap_or_default();
        let len = rest
            .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\''))
            .unwrap_or(rest.len());
        let mut end = start + trim_trailing(rest.get(..len).unwrap_or_default());
        // A bare scheme or "www." alone is not a URL.
        if STARTS.iter().any(|s| {
            text.get(start..end)
                .is_some_and(|u| u.len() <= s.len() && s.starts_with(u))
        }) {
            end = start;
        }
        if end > start {
            found.push(start..end);
        }
        from = start + len.max(1);
    }
    found
}

/// The first `needle` at or after `from` that begins a word, ignoring case.
fn find_word_start(text: &str, from: usize, needle: &str) -> Option<usize> {
    let lower = text.to_ascii_lowercase();
    let mut at = from;
    while let Some(i) = lower.get(at..)?.find(needle) {
        let i = at + i;
        let starts_word = text
            .get(..i)
            .and_then(|b| b.chars().next_back())
            .is_none_or(|c| !c.is_alphanumeric());
        if starts_word {
            return Some(i);
        }
        at = i + needle.len();
    }
    None
}

/// How much of `url` to keep once trailing punctuation is trimmed.
fn trim_trailing(url: &str) -> usize {
    let mut end = url.len();
    loop {
        let Some(last) = url.get(..end).and_then(|u| u.chars().next_back()) else {
            return end;
        };
        let keep = match last {
            '.' | ',' | ';' | ':' | '!' | '?' | '*' => false,
            ')' | ']' | '}' => {
                let open = match last {
                    ')' => '(',
                    ']' => '[',
                    _ => '{',
                };
                let body = url.get(..end).unwrap_or_default();
                body.matches(open).count() >= body.matches(last).count()
            }
            _ => true,
        };
        if keep {
            return end;
        }
        end -= last.len_utf8();
    }
}

/// Whether Irsty will open this URL: only web links, so a crafted
/// `file:` or custom-scheme link can't launch anything else.
pub fn openable(url: &str) -> Option<String> {
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("https://") || lower.starts_with("http://") {
        Some(url.to_owned())
    } else if lower.starts_with("www.") {
        Some(format!("https://{url}"))
    } else {
        None
    }
}

/// Opens a web URL with the system's handler, without a shell, and without
/// waiting for it.
pub fn open(url: &str) -> std::io::Result<()> {
    let Some(url) = openable(url) else {
        return Err(std::io::Error::other(
            "only http and https links are opened",
        ));
    };
    if url.chars().any(char::is_control) {
        return Err(std::io::Error::other(
            "the link contains control characters",
        ));
    }
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = std::process::Command::new("xdg-open");
    #[cfg(windows)]
    let mut command = {
        let mut c = std::process::Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    };
    command
        .arg(&url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn urls(text: &str) -> Vec<&str> {
        find(text).into_iter().map(|r| &text[r]).collect()
    }

    #[test]
    fn finds_urls() {
        assert_eq!(
            urls("see https://example.com/a?b=c#d, and www.rust-lang.org."),
            ["https://example.com/a?b=c#d", "www.rust-lang.org"]
        );
        assert_eq!(
            urls("(https://en.wikipedia.org/wiki/Rust_(programming_language))"),
            ["https://en.wikipedia.org/wiki/Rust_(programming_language)"]
        );
        assert_eq!(urls("<http://x.example/>"), ["http://x.example/"]);
        assert_eq!(
            urls("join ircs://irc.libera.chat/#irsty!"),
            ["ircs://irc.libera.chat/#irsty"]
        );
        assert_eq!(urls("HTTPS://SHOUTY.EXAMPLE"), ["HTTPS://SHOUTY.EXAMPLE"]);
    }

    #[test]
    fn ignores_non_urls() {
        assert!(
            urls("nohttps://x.example").is_empty(),
            "not at a word start"
        );
        assert!(urls("just http:// alone").is_empty());
        assert!(urls("www. is not a site").is_empty());
        assert!(urls("plain text").is_empty());
        assert!(urls("").is_empty());
        assert_eq!(
            urls("ünïcode https://ex.example/ü"),
            ["https://ex.example/ü"]
        );
    }

    #[test]
    fn only_web_links_open() {
        assert_eq!(
            openable("https://a.example"),
            Some("https://a.example".into())
        );
        assert_eq!(
            openable("www.a.example"),
            Some("https://www.a.example".into())
        );
        assert_eq!(openable("irc://a.example"), None);
        assert_eq!(openable("file:///etc/passwd"), None);
        assert_eq!(openable("javascript:alert(1)"), None);
        assert!(open("file:///etc/passwd").is_err());
    }
}
