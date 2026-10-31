//! A scripted server and helpers for driving the UI by keys, shared by the
//! UI integration tests.

#![allow(
    dead_code,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers shared between test binaries"
)]

use std::path::Path;
use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use irsty_core::Client;
use irsty_core::config_file::ConfigFile;
use irsty_tui::app;
use irsty_tui::keymap::Keymap;
use irsty_tui::ui::Ui;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::time::{Instant, timeout};

/// Presses a key.
pub fn press(code: KeyCode, mods: KeyModifiers, client: &mut Client, ui: &mut Ui) -> app::Flow {
    let flow = app::handle_event(Event::Key(KeyEvent::new(code, mods)), client, ui);
    ui.on_update(client.state());
    flow
}

/// Types text without pressing Enter.
pub fn type_text(text: &str, client: &mut Client, ui: &mut Ui) {
    for c in text.chars() {
        press(KeyCode::Char(c), KeyModifiers::NONE, client, ui);
    }
}

/// Types `line` and presses Enter.
pub fn type_line(line: &str, client: &mut Client, ui: &mut Ui) {
    type_text(line, client, ui);
    press(KeyCode::Enter, KeyModifiers::NONE, client, ui);
}

/// Drives the client until `done` holds.
pub async fn until(
    client: &mut Client,
    ui: &mut Ui,
    what: &str,
    done: impl Fn(&Client, &Ui) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(client, ui) {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        let _ = timeout(Duration::from_millis(20), client.next()).await;
        ui.on_update(client.state());
    }
}

/// A server that registers the client as `tester`, answers joins with
/// `names` in the channel, and passes on every other line it receives.
pub async fn mock_server(
    listener: TcpListener,
    names: String,
    lines: mpsc::UnboundedSender<String>,
) {
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
                ":tester!u@h JOIN {channel}\r\n:srv 353 tester = {channel} :tester {names}\r\n:srv 366 tester {channel} :End"
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

/// A client and UI with no networks, using a config file at `path`.
pub fn empty(path: &Path) -> (Client, Ui) {
    let (file, config) = ConfigFile::open(path).unwrap();
    let client = Client::start(&config, None, None);
    let mut ui = Ui::new(&config.ui, client.state());
    ui.keymap = Keymap::new(&config.ui.keymap, &config.keys).0;
    ui.config = config;
    ui.config_file = Some(file);
    (client, ui)
}

/// A client joined to `channel` (with `names` in it) on a mock server, and
/// the lines the server receives.
pub async fn joined(
    path: &Path,
    channel: &str,
    names: &str,
) -> (Client, Ui, mpsc::UnboundedReceiver<String>) {
    let (mut client, mut ui) = empty(path);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, received) = mpsc::unbounded_channel();
    tokio::spawn(mock_server(listener, names.to_owned(), tx));
    type_line(
        &format!("/server add mock 127.0.0.1:{port} -notls -nick tester -autojoin {channel}"),
        &mut client,
        &mut ui,
    );
    let channel = channel.to_owned();
    // The names arrive after the join; wait for both, or a test could look
    // at the nick list before it is filled.
    let members = names.split_whitespace().count() + 1;
    until(&mut client, &mut ui, "autojoin and names", |c, _| {
        c.state().networks().any(|n| {
            n.channel(&channel)
                .is_some_and(|ch| ch.joined && ch.members().count() == members)
        })
    })
    .await;
    (client, ui, received)
}

/// The next line the server received.
pub async fn next_line(received: &mut mpsc::UnboundedReceiver<String>) -> String {
    timeout(Duration::from_secs(5), received.recv())
        .await
        .expect("nothing sent")
        .unwrap()
}
