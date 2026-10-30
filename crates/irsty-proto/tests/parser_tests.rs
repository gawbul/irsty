//! Conformance against the ircdocs `parser-tests` corpus (CC0), vendored in
//! `tests/fixtures/parser-tests` at the commit recorded in `COMMIT`.
//!
//! `validate-hostname.yaml` is deliberately not vendored: it is for servers, and
//! clients must accept whatever hostnames they are sent.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test helpers outside #[test] functions"
)]

use std::collections::BTreeMap;
use std::path::Path;

use irsty_proto::casemap::CaseMapping;
use irsty_proto::{Message, MessageRef, Source, mask};
use serde::Deserialize;

fn load<T: for<'de> Deserialize<'de>>(name: &str) -> T {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/parser-tests")
        .join(name);
    let yaml = std::fs::read_to_string(&path).unwrap();
    serde_saphyr::from_str(&yaml).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[derive(Debug, Deserialize)]
struct Suite<T> {
    tests: Vec<T>,
}

#[derive(Debug, Default, Deserialize)]
struct Atoms {
    #[serde(default)]
    tags: BTreeMap<String, String>,
    source: Option<String>,
    verb: String,
    #[serde(default)]
    params: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct SplitCase {
    input: String,
    atoms: Atoms,
}

#[derive(Debug, Deserialize)]
struct JoinCase {
    desc: String,
    atoms: Atoms,
    matches: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct UserhostCase {
    source: String,
    atoms: UserhostAtoms,
}

#[derive(Debug, Deserialize)]
struct UserhostAtoms {
    nick: Option<String>,
    user: Option<String>,
    host: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MaskCase {
    mask: String,
    matches: Vec<String>,
    fails: Vec<String>,
}

#[test]
fn msg_split() {
    let suite: Suite<SplitCase> = load("msg-split.yaml");
    assert!(suite.tests.len() >= 30, "corpus looks truncated");
    for case in &suite.tests {
        let msg =
            MessageRef::parse(&case.input).unwrap_or_else(|e| panic!("{:?}: {e}", case.input));
        let tags: BTreeMap<String, String> = msg
            .to_owned()
            .tags
            .iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        assert_eq!(tags, case.atoms.tags, "tags of {:?}", case.input);
        assert_eq!(
            msg.source_raw(),
            case.atoms.source.as_deref(),
            "source of {:?}",
            case.input
        );
        assert_eq!(msg.command(), case.atoms.verb, "verb of {:?}", case.input);
        assert_eq!(
            msg.params(),
            case.atoms.params,
            "params of {:?}",
            case.input
        );
    }
}

#[test]
fn msg_join() {
    let suite: Suite<JoinCase> = load("msg-join.yaml");
    assert!(suite.tests.len() >= 15, "corpus looks truncated");
    for case in &suite.tests {
        let atoms = &case.atoms;
        let msg = Message {
            tags: atoms
                .tags
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect(),
            source: atoms.source.clone(),
            command: atoms.verb.clone(),
            params: atoms.params.clone(),
        };
        let wire = msg
            .to_wire()
            .unwrap_or_else(|e| panic!("{}: {e}", case.desc));
        assert!(
            case.matches.contains(&wire),
            "{}: {wire:?} not in {:?}",
            case.desc,
            case.matches
        );
    }
}

#[test]
fn userhost_split() {
    let suite: Suite<UserhostCase> = load("userhost-split.yaml");
    assert!(!suite.tests.is_empty());
    for case in &suite.tests {
        let source = Source::parse(&case.source);
        assert_eq!(
            Some(source.nick),
            case.atoms.nick.as_deref(),
            "nick of {:?}",
            case.source
        );
        assert_eq!(
            source.user,
            case.atoms.user.as_deref(),
            "user of {:?}",
            case.source
        );
        assert_eq!(
            source.host,
            case.atoms.host.as_deref(),
            "host of {:?}",
            case.source
        );
    }
}

#[test]
fn mask_match() {
    let suite: Suite<MaskCase> = load("mask-match.yaml");
    assert!(!suite.tests.is_empty());
    for case in &suite.tests {
        for text in &case.matches {
            assert!(
                mask::matches(&case.mask, text, CaseMapping::Rfc1459),
                "{:?} should match {text:?}",
                case.mask
            );
        }
        for text in &case.fails {
            assert!(
                !mask::matches(&case.mask, text, CaseMapping::Rfc1459),
                "{:?} should not match {text:?}",
                case.mask
            );
        }
    }
}

#[test]
fn split_round_trips_through_serialiser() {
    // Every msg-split input should survive parse, serialise, parse unchanged.
    let suite: Suite<SplitCase> = load("msg-split.yaml");
    for case in &suite.tests {
        let first = Message::parse(&case.input).unwrap();
        let wire = first
            .to_wire()
            .unwrap_or_else(|e| panic!("{:?}: {e}", case.input));
        assert_eq!(
            Message::parse(&wire).unwrap(),
            first,
            "{:?} -> {wire:?}",
            case.input
        );
    }
}
