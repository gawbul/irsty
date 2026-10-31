//! M6's acceptance test: starting with no config at all, reach a joined
//! channel using only commands typed into the UI, with every change saved to
//! `config.toml`. Keys go through the real input path, so aliases and parsing
//! are exercised as a user would.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers outside #[test] functions"
)]

mod support;

use irsty_core::config_file::ConfigFile;
use support::*;
use tokio::net::TcpListener;
use tokio::sync::mpsc;

fn joined_to(client: &irsty_core::Client, channel: &str) -> bool {
    client
        .state()
        .networks()
        .any(|n| n.channel(channel).is_some_and(|c| c.joined))
}

#[tokio::test(flavor = "multi_thread")]
async fn first_run_with_commands_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("irsty").join("config.toml");
    let (mut client, mut ui) = empty(&path);
    assert!(ui.config.networks.is_empty() && !path.exists());
    assert_eq!(ui.active, None, "no buffers yet");

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, mut received) = mpsc::unbounded_channel();
    tokio::spawn(mock_server(listener, String::new(), tx));

    // A typo before any network exists shows in the first-run screen.
    type_line("/frobnicate", &mut client, &mut ui);
    assert!(ui.status.iter().any(|(_, t)| t.contains("Unknown command")));

    type_line(
        &format!("/server add mock 127.0.0.1:{port} -notls -nick tester -autojoin #first"),
        &mut client,
        &mut ui,
    );
    until(&mut client, &mut ui, "autojoin", |c, _| {
        joined_to(c, "#first")
    })
    .await;

    let saved = std::fs::read_to_string(&path).unwrap();
    for expected in [
        "name = \"mock\"",
        "nicks = [\"tester\"]",
        "autojoin = [\"#first\"]",
        "tls = false",
    ] {
        assert!(saved.contains(expected), "missing {expected}:\n{saved}");
    }
    let (_, reloaded) = ConfigFile::open(&path).unwrap();
    assert_eq!(reloaded.networks[0].servers[0].port, port);

    // An alias, used from the channel.
    type_line(
        "/alias hi /msg $channel hello from $nick",
        &mut client,
        &mut ui,
    );
    type_line("/window 2", &mut client, &mut ui);
    type_line("/hi", &mut client, &mut ui);
    let sent = next_line(&mut received).await;
    assert_eq!(sent, "PRIVMSG #first :hello from tester");

    // A setting, applied at once and saved.
    assert!(ui.show_nicklist);
    type_line("/set ui.nicklist off", &mut client, &mut ui);
    assert!(!ui.show_nicklist);
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("nicklist = false")
    );

    // A WHOIS goes out, with its replies due here.
    type_line("/whois someone", &mut client, &mut ui);
    let sent = next_line(&mut received).await;
    assert_eq!(sent, "WHOIS someone");

    // Removing the network takes it and its buffers away, and out of the file.
    type_line("/server del mock", &mut client, &mut ui);
    assert_eq!(client.state().networks().count(), 0);
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(!saved.contains("name = \"mock\""), "{saved}");
    assert!(saved.contains("[aliases]"), "the alias stays:\n{saved}");
    client.quit(None).await;
}
