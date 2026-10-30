//! Property tests: round trips, and no panics or unsafe output on arbitrary input.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test helpers outside #[test] functions"
)]

use irsty_proto::casemap::CaseMapping;
use irsty_proto::format::StyledText;
use irsty_proto::{ISupport, Message, MessageRef, decode, mask, split, tags};
use proptest::prelude::*;

fn tag_key() -> impl Strategy<Value = String> {
    "(\\+)?([a-z0-9.-]{1,8}/)?[A-Za-z0-9-]{1,10}"
}

fn message() -> impl Strategy<Value = Message> {
    (
        prop::collection::vec((tag_key(), "[^\0]{0,12}"), 0..4),
        prop::option::of("[^ \0\r\n]{1,20}"),
        "[A-Za-z]{1,10}|[0-9]{3}",
        prop::collection::vec("[^ \0\r\n:][^ \0\r\n]{0,10}", 0..14),
        prop::option::of("[^\0\r\n]{0,30}"),
    )
        .prop_map(|(tags, source, command, middle, trailing)| Message {
            tags: tags.into_iter().collect(),
            source,
            command,
            params: middle.into_iter().chain(trailing).collect(),
        })
}

/// Reports whether `text` is the chunks joined by either nothing or one consumed
/// space at each boundary.
fn reassembles(text: &str, chunks: &[&str]) -> bool {
    match chunks {
        [] => text.is_empty(),
        [chunk, rest @ ..] => text.strip_prefix(chunk).is_some_and(|after| {
            reassembles(after, rest)
                || (!rest.is_empty()
                    && after
                        .strip_prefix(' ')
                        .is_some_and(|a| reassembles(a, rest)))
        }),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2048))]

    #[test]
    fn serialise_then_parse_is_identity(msg in message()) {
        let wire = msg.to_wire().unwrap();
        prop_assert_eq!(Message::parse(&wire).unwrap(), msg);
    }

    #[test]
    fn parse_never_panics(line in any::<String>()) {
        if let Ok(msg) = MessageRef::parse(&line) {
            prop_assert!(!msg.command().is_empty());
            prop_assert!(msg.params().len() <= irsty_proto::message::MAX_PARAMS);
            let _ = msg.to_owned();
            let _ = msg.source();
            for (key, value) in msg.tags() {
                let _ = (key, tags::unescape(value));
            }
        }
    }

    #[test]
    fn parse_never_panics_on_ircish_lines(line in "(@[a-z=;\\\\ ]{0,10} )?(:[a-z!@.]{0,10} )?[A-Z0-9 :]{0,40}") {
        let _ = MessageRef::parse(&line);
    }

    #[test]
    fn tag_values_round_trip(value in "[^\0]*") {
        let mut escaped = String::new();
        tags::escape_into(&value, &mut escaped);
        prop_assert!(!escaped.contains([';', ' ', '\r', '\n']));
        prop_assert_eq!(tags::unescape(&escaped), value);
    }

    #[test]
    fn styled_text_is_always_safe(input in any::<String>()) {
        for text in [StyledText::parse(&input), StyledText::plain(&input)] {
            let plain = text.to_plain();
            prop_assert!(!plain.chars().any(char::is_control), "{plain:?}");
            prop_assert!(!plain.contains(['\u{2028}', '\u{2029}']), "separator survived in {:?}", plain);
            // Bidi scopes are balanced: every opener has a closer.
            let opens = plain.chars().filter(|c| matches!(c, '\u{202A}' | '\u{202B}' | '\u{202D}' | '\u{202E}')).count();
            let closes = plain.chars().filter(|&c| c == '\u{202C}').count();
            prop_assert_eq!(opens, closes);
            let isolates = plain.chars().filter(|c| matches!(c, '\u{2066}'..='\u{2068}')).count();
            let pdis = plain.chars().filter(|&c| c == '\u{2069}').count();
            prop_assert_eq!(isolates, pdis);
        }
    }

    #[test]
    fn identifiers_are_safe(input in any::<String>()) {
        let out = irsty_proto::format::sanitise_identifier(&input);
        prop_assert!(!out.chars().any(|c| c.is_control() || matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')), "unsafe identifier {:?}", out);
    }

    #[test]
    fn split_respects_budget_and_loses_nothing(text in "[a-z é👍\u{301}]{0,200}", max in 4usize..64) {
        let chunks = split::split_text(&text, max);
        for chunk in &chunks {
            prop_assert!(chunk.len() <= max, "{:?} exceeds {}", chunk, max);
        }
        prop_assert!(reassembles(&text, &chunks), "{:?} does not reassemble from {:?}", text, chunks);
    }

    #[test]
    fn folding_is_idempotent(s in any::<String>()) {
        for m in [CaseMapping::Ascii, CaseMapping::Rfc1459, CaseMapping::Rfc1459Strict, CaseMapping::Unicode] {
            let once = m.fold(&s).into_owned();
            prop_assert_eq!(m.fold(&once), once.as_str());
        }
    }

    #[test]
    fn masks_without_wildcards_match_themselves(s in "[^*?]{0,30}") {
        prop_assert!(mask::matches(&s, &s, CaseMapping::Rfc1459));
        prop_assert!(mask::matches("*", &s, CaseMapping::Rfc1459));
        let wild = format!("*{s}*");
        prop_assert!(mask::matches(&wild, &s, CaseMapping::Rfc1459));
    }

    #[test]
    fn isupport_never_panics(tokens in prop::collection::vec(any::<String>(), 0..20)) {
        let refs: Vec<&str> = tokens.iter().map(String::as_str).collect();
        let mut isupport = ISupport::default();
        isupport.apply(&refs);
        let _ = isupport.split_prefixes("@+nick");
    }

    #[test]
    fn decode_never_panics(bytes in any::<Vec<u8>>()) {
        let text = decode::decode_line(&bytes);
        prop_assert_eq!(text.chars().count() >= bytes.len() / 4, true);
    }
}
