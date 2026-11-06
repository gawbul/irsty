//! M11 through the UI: split panes, layouts, jumping to unread lines and
//! `/search`, driven by key presses against a scripted server.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers outside #[test] functions"
)]

mod support;

use crossterm::event::{KeyCode, KeyModifiers};
use irsty_core::Client;
use irsty_tui::ui::Ui;
use support::*;

fn last_line(client: &Client, ui: &Ui) -> String {
    client
        .state()
        .buffer(ui.active.unwrap())
        .unwrap()
        .lines()
        .back()
        .unwrap()
        .text
        .to_plain()
}

#[tokio::test(flavor = "multi_thread")]
async fn panes_layouts_and_search() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let (mut client, mut ui, _received) = joined(&path, "#one", "").await;
    type_line("/window 2", &mut client, &mut ui);
    let one = ui.active.unwrap();

    // Split side by side; the new pane has focus and shows the same buffer.
    type_line("/window vsplit", &mut client, &mut ui);
    assert_eq!(ui.panes.len(), 2);
    assert_eq!(ui.panes.focus(), 1);
    type_line("/window 1", &mut client, &mut ui);
    let server = ui.active.unwrap();
    assert_ne!(server, one);
    // F7 and F8 move focus; each pane kept its buffer.
    press(KeyCode::F(8), KeyModifiers::NONE, &mut client, &mut ui);
    assert_eq!(ui.active, Some(one));
    press(KeyCode::F(7), KeyModifiers::NONE, &mut client, &mut ui);
    assert_eq!(ui.active, Some(server));

    // Saved to the config, and restored after merging.
    type_line("/layout save", &mut client, &mut ui);
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(saved.contains("layout = \"h(mock/#one,mock)\""), "{saved}");
    type_line("/window merge all", &mut client, &mut ui);
    assert_eq!(ui.panes.len(), 1);
    type_line("/window merge", &mut client, &mut ui);
    assert_eq!(last_line(&client, &ui), "This is the only pane");
    type_line("/layout apply", &mut client, &mut ui);
    assert_eq!(ui.panes.len(), 2);
    assert_eq!(ui.visible(), [one, server]);
    type_line("/layout reset", &mut client, &mut ui);
    assert_eq!(ui.panes.len(), 1);
    assert!(!std::fs::read_to_string(&path).unwrap().contains("layout"));
    type_line("/set ui.layout v(nope", &mut client, &mut ui);
    assert!(
        last_line(&client, &ui).contains("expected"),
        "{}",
        last_line(&client, &ui)
    );

    // Shift+Alt+U asks to jump to the first unread line.
    press(
        KeyCode::Char('U'),
        KeyModifiers::ALT | KeyModifiers::SHIFT,
        &mut client,
        &mut ui,
    );
    assert!(ui.jump_unread);

    // This harness has no data directory, so no search index.
    type_line("/search hello", &mut client, &mut ui);
    assert!(
        last_line(&client, &ui).contains("search index is off"),
        "{}",
        last_line(&client, &ui)
    );
    client.quit(None).await;
}
