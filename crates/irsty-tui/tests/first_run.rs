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

use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use irsty_core::Client;
use irsty_core::config_file::ConfigFile;
use irsty_tui::app;
use irsty_tui::ui::Ui;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::time::{Instant, timeout};

/// Types `line` and presses Enter.
fn type_line(line: &str, client: &mut Client, ui: &mut Ui) {
    for c in line.chars() {
        let key = KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
        app::handle_event(Event::Key(key), client, ui);
    }
    let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
    app::handle_event(Event::Key(enter), client, ui);
    ui.on_update(client.state());
}

async fn until(client: &mut Client, ui: &mut Ui, what: &str, done: impl Fn(&Client, &Ui) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(client, ui) {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        let _ = timeout(Duration::from_millis(20), client.next()).await;
        ui.on_update(client.state());
    }
}

/// A server that registers the client, confirms joins, and passes on every
/// other line it receives.
async fn mock_server(listener: TcpListener, lines: mpsc::UnboundedSender<String>) {
    let (stream, _) = listener.accept().await.unwrap();
    let (read, mut write) = stream.into_split();
    let mut read = BufReader::new(read).lines();
    while let Ok(Some(line)) = read.next_line().await {
        let reply = if line.starts_with("CAP LS") {
            Some(":srv CAP * LS :".to_owned())
        } else if line == "CAP END" {
            Some(":srv 001 tester :Welcome".to_owned())
        } else if let Some(channel) = line.strip_prefix("JOIN ") {
            Some(format!(
                ":tester!u@h JOIN {channel}\r\n:srv 353 tester = {channel} :tester\r\n:srv 366 tester {channel} :End"
            ))
        } else if line.starts_with("NICK ") || line.starts_with("USER ") {
            None
        } else {
            let _ = lines.send(line);
            None
        };
        if let Some(reply) = reply {
            write
                .write_all(format!("{reply}\r\n").as_bytes())
                .await
                .unwrap();
        }
    }
}

fn joined(client: &Client, channel: &str) -> bool {
    client
        .state()
        .networks()
        .any(|n| n.channel(channel).is_some_and(|c| c.joined))
}

#[tokio::test(flavor = "multi_thread")]
async fn first_run_with_commands_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("irsty").join("config.toml");
    let (file, config) = ConfigFile::open(&path).unwrap();
    assert!(config.networks.is_empty() && !path.exists());

    let mut client = Client::start(&config, None, None);
    let mut ui = Ui::new(&config.ui, client.state());
    ui.config = config;
    ui.config_file = Some(file);
    assert_eq!(ui.active, None, "no buffers yet");

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, mut received) = mpsc::unbounded_channel();
    tokio::spawn(mock_server(listener, tx));

    // A typo before any network exists shows in the first-run screen.
    type_line("/frobnicate", &mut client, &mut ui);
    assert!(ui.status.iter().any(|(_, t)| t.contains("Unknown command")));

    type_line(
        &format!("/server add mock 127.0.0.1:{port} -notls -nick tester -autojoin #first"),
        &mut client,
        &mut ui,
    );
    until(&mut client, &mut ui, "autojoin", |c, _| joined(c, "#first")).await;

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
    let sent = timeout(Duration::from_secs(5), received.recv())
        .await
        .unwrap()
        .unwrap();
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
    let sent = timeout(Duration::from_secs(5), received.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sent, "WHOIS someone");

    // Removing the network takes it and its buffers away, and out of the file.
    type_line("/server del mock", &mut client, &mut ui);
    assert_eq!(client.state().networks().count(), 0);
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(!saved.contains("name = \"mock\""), "{saved}");
    assert!(saved.contains("[aliases]"), "the alias stays:\n{saved}");
    client.quit(None).await;
}
