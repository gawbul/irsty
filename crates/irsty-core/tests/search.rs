//! M11 end to end: messages are indexed as they are logged, and a search
//! answers in the buffer it was asked from.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test helpers outside #[test] functions"
)]

mod support;

use std::time::Duration;

use irsty_core::Client;
use irsty_core::config::Paths;
use irsty_core::search::Query;
use irsty_core::state::BufferId;
use support::*;

fn channel(client: &Client) -> Option<BufferId> {
    client
        .state()
        .networks()
        .next()?
        .channel("#test")
        .map(|c| c.buffer)
}

fn texts(client: &Client, buffer: BufferId) -> Vec<String> {
    client
        .state()
        .buffer(buffer)
        .unwrap()
        .lines()
        .iter()
        .map(|l| l.text.to_plain())
        .collect()
}

#[tokio::test]
async fn messages_are_indexed_and_found() {
    let mock = Mock::bind().await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(mock.port(), "", &dir.path().join("logs"));
    let paths = Paths {
        config_dir: dir.path().to_owned(),
        data_dir: dir.path().to_owned(),
    };
    let server = tokio::spawn(async move {
        let mut c = mock.accept().await;
        c.register("me", "").await;
        c.expect("JOIN #test").await;
        c.send(":me!u@h JOIN #test").await;
        c.send(":alice!a@h PRIVMSG #test :the borrow checker is great")
            .await;
        c.send(":bob!b@h PRIVMSG #test :lunch?").await;
        c.send(":carol!c@h PRIVMSG bob :borrowing your pen").await;
        c
    });
    let mut client = Client::start(&cfg, Some(&paths), Some(fast(Duration::from_secs(30))));
    assert!(client.has_index());
    let _conn = drive(&mut client, server).await;
    until(&mut client, |c| {
        channel(c).is_some_and(|b| texts(c, b).iter().any(|l| l == "lunch?"))
    })
    .await;
    let buffer = channel(&client).unwrap();

    // In this channel only.
    client
        .search(
            buffer,
            Query {
                words: "borrow".into(),
                network: Some("mock".into()),
                buffer: Some("#test".into()),
                from: None,
                limit: 50,
            },
        )
        .unwrap();
    until(&mut client, |c| {
        texts(c, buffer)
            .iter()
            .any(|l| l.ends_with("<alice> the borrow checker is great"))
    })
    .await;
    let lines = texts(&client, buffer);
    assert!(
        lines.contains(&"Search for \"borrow\" in mock/#test: 1 match".to_owned()),
        "{lines:?}"
    );
    // The match is in bold.
    let hit = client
        .state()
        .buffer(buffer)
        .unwrap()
        .lines()
        .back()
        .unwrap()
        .clone();
    assert!(
        hit.text
            .spans()
            .iter()
            .any(|s| s.text() == "borrow" && s.style().bold)
    );

    // Everywhere: the query too, with where it was said.
    client
        .search(
            buffer,
            Query {
                words: "borrow".into(),
                limit: 50,
                ..Query::default()
            },
        )
        .unwrap();
    until(&mut client, |c| {
        texts(c, buffer)
            .iter()
            .any(|l| l.ends_with("mock/carol <carol> borrowing your pen"))
    })
    .await;
    assert!(
        client
            .search(
                buffer,
                Query {
                    words: " ".into(),
                    limit: 5,
                    ..Query::default()
                }
            )
            .is_err()
    );
    client.quit(None).await;

    // Results aren't written to the logs again.
    let log = std::fs::read_to_string(dir.path().join("logs/mock/#test.log")).unwrap();
    assert!(!log.contains("Search for"), "{log}");
    assert!(log.contains("lunch?"));
}

#[tokio::test]
async fn no_index_without_paths_or_logging() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(1, "", dir.path());
    let client = Client::start(&cfg, None, None);
    assert!(!client.has_index());
    let mut cfg = config(1, "", dir.path());
    cfg.logging.enabled = false;
    let paths = Paths {
        config_dir: dir.path().to_owned(),
        data_dir: dir.path().to_owned(),
    };
    let mut client = Client::start(&cfg, Some(&paths), None);
    assert!(!client.has_index(), "the index keeps what the logs keep");
    let error = client
        .search(
            BufferId(1),
            Query {
                words: "x".into(),
                limit: 5,
                ..Query::default()
            },
        )
        .unwrap_err();
    assert!(error.contains("search index is off"), "{error}");
}
