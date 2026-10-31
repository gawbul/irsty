//! M10 through the UI: typing notifications from the input line, and /reply,
//! /react and /unreact typed into it (the tagged messages themselves are
//! tested in irsty-core's `tests/tags.rs`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers outside #[test] functions"
)]

mod support;

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
async fn reply_and_react_commands() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let (mut client, mut ui, mut received) = joined(&path, "#talk", "alice").await;
    type_line("/window 2", &mut client, &mut ui);

    // Nothing from alice with an id: still a reply, addressed to her.
    type_line("/reply alice good idea", &mut client, &mut ui);
    assert_eq!(
        next_line(&mut received).await,
        "PRIVMSG #talk :alice: good idea"
    );

    // Reactions need a message to react to.
    type_line("/react alice 👍", &mut client, &mut ui);
    assert!(
        last_line(&client, &ui).starts_with("No message from alice here to react to"),
        "{}",
        last_line(&client, &ui)
    );
    type_line("/unreact", &mut client, &mut ui);
    assert!(
        last_line(&client, &ui).contains("/unreact <nick> <reaction>"),
        "usage: {}",
        last_line(&client, &ui)
    );
    client.quit(None).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn typing_a_message_says_so_once_turned_on() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let (mut client, mut ui, mut received) = joined(&path, "#talk", "").await;
    type_line("/window 2", &mut client, &mut ui);
    // Off by default: commands and text alike say nothing.
    type_text("hel", &mut client, &mut ui);
    type_line("lo", &mut client, &mut ui);
    assert_eq!(next_line(&mut received).await, "PRIVMSG #talk hello");

    type_line("/set typing.send true", &mut client, &mut ui);
    // A command isn't a message.
    type_text("/wh", &mut client, &mut ui);
    press(
        crossterm::event::KeyCode::Char('u'),
        crossterm::event::KeyModifiers::CONTROL,
        &mut client,
        &mut ui,
    );
    type_text("hi", &mut client, &mut ui);
    assert_eq!(
        next_line(&mut received).await,
        "@+typing=active TAGMSG #talk"
    );
    press(
        crossterm::event::KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
        &mut client,
        &mut ui,
    );
    assert_eq!(
        next_line(&mut received).await,
        "PRIVMSG #talk hi",
        "one notification, then the message"
    );
    client.quit(None).await;
}
