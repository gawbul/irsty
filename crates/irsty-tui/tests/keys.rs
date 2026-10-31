//! M7 end to end: completion, the buffer switcher, back and forward, the
//! multi-line paste prompt and key bindings, driven by key presses against a
//! scripted server.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers outside #[test] functions"
)]

mod support;

use crossterm::event::{Event, KeyCode, KeyModifiers};
use irsty_core::Client;
use irsty_tui::app;
use irsty_tui::ui::Ui;
use support::*;

const NONE: KeyModifiers = KeyModifiers::NONE;

fn tab(client: &mut Client, ui: &mut Ui) {
    press(KeyCode::Tab, NONE, client, ui);
}

fn active_name(client: &Client, ui: &Ui) -> String {
    ui.active
        .and_then(|b| client.state().buffer(b))
        .map(|b| b.name.clone())
        .unwrap_or_default()
}

/// End then Ctrl+U: clears the whole line wherever the cursor is.
fn clear_input(client: &mut Client, ui: &mut Ui) {
    press(KeyCode::End, NONE, client, ui);
    press(KeyCode::Char('u'), KeyModifiers::CONTROL, client, ui);
}

#[tokio::test(flavor = "multi_thread")]
async fn completion_cycles_and_knows_its_context() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let (mut client, mut ui, _received) = joined(&path, "#keys", "alice albert bob").await;
    type_line("/window 2", &mut client, &mut ui);
    assert_eq!(active_name(&client, &ui), "#keys");

    // A nick at the start of the line gets ": "; Tab again cycles.
    type_text("al", &mut client, &mut ui);
    tab(&mut client, &mut ui);
    assert_eq!(ui.input.text(), "albert: ");
    tab(&mut client, &mut ui);
    assert_eq!(ui.input.text(), "alice: ");
    tab(&mut client, &mut ui);
    assert_eq!(ui.input.text(), "albert: ", "wraps around");
    press(KeyCode::BackTab, KeyModifiers::SHIFT, &mut client, &mut ui);
    assert_eq!(ui.input.text(), "alice: ");
    // Typing ends the cycle; the next Tab completes the new word.
    type_text("hi b", &mut client, &mut ui);
    tab(&mut client, &mut ui);
    assert_eq!(ui.input.text(), "alice: hi bob ");
    clear_input(&mut client, &mut ui);

    // Commands, then their arguments.
    type_text("/whow", &mut client, &mut ui);
    tab(&mut client, &mut ui);
    assert_eq!(ui.input.text(), "/whowas ");
    clear_input(&mut client, &mut ui);
    type_text("/set ui.time", &mut client, &mut ui);
    tab(&mut client, &mut ui);
    assert_eq!(ui.input.text(), "/set ui.timestamp_format ");
    clear_input(&mut client, &mut ui);
    type_text("/disconnect mo", &mut client, &mut ui);
    tab(&mut client, &mut ui);
    assert_eq!(ui.input.text(), "/disconnect mock ");
    clear_input(&mut client, &mut ui);
    type_text("see #k", &mut client, &mut ui);
    tab(&mut client, &mut ui);
    assert_eq!(ui.input.text(), "see #keys ");
    clear_input(&mut client, &mut ui);
    // No match leaves the input alone.
    type_text("zz", &mut client, &mut ui);
    tab(&mut client, &mut ui);
    assert_eq!(ui.input.text(), "zz");
    client.quit(None).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn switcher_back_forward_and_jump() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let (mut client, mut ui, _received) = joined(&path, "#first", "").await;
    type_line("/join #second", &mut client, &mut ui);
    until(&mut client, &mut ui, "second channel", |_, ui| {
        ui.pending_switch.is_none()
    })
    .await;
    assert_eq!(active_name(&client, &ui), "#second");

    // Ctrl+G, type a fuzzy query, Enter.
    press(
        KeyCode::Char('g'),
        KeyModifiers::CONTROL,
        &mut client,
        &mut ui,
    );
    assert!(ui.switcher.is_some());
    type_text("frst", &mut client, &mut ui);
    let top = ui.switcher_matches(client.state());
    assert_eq!(top[0].2, "#first (mock)");
    press(KeyCode::Enter, NONE, &mut client, &mut ui);
    assert!(ui.switcher.is_none());
    assert_eq!(active_name(&client, &ui), "#first");
    // Esc closes without switching.
    press(
        KeyCode::Char('g'),
        KeyModifiers::CONTROL,
        &mut client,
        &mut ui,
    );
    type_text("second", &mut client, &mut ui);
    press(KeyCode::Esc, NONE, &mut client, &mut ui);
    assert_eq!(active_name(&client, &ui), "#first");

    // Alt+< goes back, Alt+> forward again.
    press(KeyCode::Char('<'), KeyModifiers::ALT, &mut client, &mut ui);
    assert_eq!(active_name(&client, &ui), "#second");
    press(KeyCode::Char('>'), KeyModifiers::ALT, &mut client, &mut ui);
    assert_eq!(active_name(&client, &ui), "#first");

    // The WeeChat preset: Alt+J then two digits.
    type_line("/set ui.keymap weechat", &mut client, &mut ui);
    press(KeyCode::Char('j'), KeyModifiers::ALT, &mut client, &mut ui);
    assert_eq!(ui.jump.as_deref(), Some(""));
    type_text("01", &mut client, &mut ui);
    assert!(ui.jump.is_none());
    assert_eq!(active_name(&client, &ui), "mock");
    // F6 is next buffer in WeeChat.
    press(KeyCode::F(6), NONE, &mut client, &mut ui);
    assert_eq!(active_name(&client, &ui), "#first");
    client.quit(None).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn multi_line_paste_asks_first() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let (mut client, mut ui, mut received) = joined(&path, "#paste", "").await;
    type_line("/window 2", &mut client, &mut ui);
    let paste = |text: &str, client: &mut Client, ui: &mut Ui| {
        app::handle_event(Event::Paste(text.into()), client, ui);
    };

    // One line goes straight into the input.
    paste("just one line", &mut client, &mut ui);
    assert!(ui.paste.is_none());
    assert_eq!(ui.input.text(), "just one line");
    clear_input(&mut client, &mut ui);

    // Several lines wait; Esc cancels and nothing is sent.
    paste("first\nsecond\n", &mut client, &mut ui);
    assert_eq!(
        ui.paste.as_deref(),
        Some(&["first".to_owned(), "second".to_owned()][..])
    );
    press(KeyCode::Esc, NONE, &mut client, &mut ui);
    assert!(ui.paste.is_none());

    // J joins them into the input instead.
    paste("a\r\nb", &mut client, &mut ui);
    press(KeyCode::Char('j'), NONE, &mut client, &mut ui);
    assert_eq!(ui.input.text(), "a b");
    clear_input(&mut client, &mut ui);

    // Enter sends each line as a message; a pasted "/quit" is text, not a command.
    paste("hello\n\n/quit now", &mut client, &mut ui);
    let flow = press(KeyCode::Enter, NONE, &mut client, &mut ui);
    assert_eq!(flow, app::Flow::Continue);
    assert_eq!(next_line(&mut received).await, "PRIVMSG #paste hello");
    assert_eq!(next_line(&mut received).await, "PRIVMSG #paste :/quit now");
    client.quit(None).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn bindings_are_saved_and_take_effect() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let (mut client, mut ui, _received) = joined(&path, "#binds", "").await;

    type_line("/bind alt-x switcher", &mut client, &mut ui);
    press(KeyCode::Char('x'), KeyModifiers::ALT, &mut client, &mut ui);
    assert!(ui.switcher.is_some());
    press(KeyCode::Esc, NONE, &mut client, &mut ui);

    type_line("/unbind ctrl-g", &mut client, &mut ui);
    press(
        KeyCode::Char('g'),
        KeyModifiers::CONTROL,
        &mut client,
        &mut ui,
    );
    assert!(ui.switcher.is_none(), "ctrl-g now does nothing");
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(saved.contains("[keys]"), "{saved}");
    assert!(saved.contains("alt-x = \"switcher\""), "{saved}");
    assert!(saved.contains("ctrl-g = \"none\""), "{saved}");

    type_line("/bind -reset ctrl-g", &mut client, &mut ui);
    press(
        KeyCode::Char('g'),
        KeyModifiers::CONTROL,
        &mut client,
        &mut ui,
    );
    assert!(ui.switcher.is_some(), "back to the preset");
    press(KeyCode::Esc, NONE, &mut client, &mut ui);

    // Bad names are refused and nothing is saved.
    let before = std::fs::read_to_string(&path).unwrap();
    type_line("/bind hyper-q send", &mut client, &mut ui);
    type_line("/bind alt-q fly", &mut client, &mut ui);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    client.quit(None).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn formatting_keys_and_links() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let (mut client, mut ui, mut received) = joined(&path, "#fmt", "").await;
    type_line("/window 2", &mut client, &mut ui);

    // irssi: Ctrl keys type the code with the same byte.
    let ctrl = KeyModifiers::CONTROL;
    press(KeyCode::Char('b'), ctrl, &mut client, &mut ui);
    type_text("bold", &mut client, &mut ui);
    press(KeyCode::Char('b'), ctrl, &mut client, &mut ui);
    press(KeyCode::Char('c'), ctrl, &mut client, &mut ui);
    type_text("4red", &mut client, &mut ui);
    press(KeyCode::Char('7'), ctrl, &mut client, &mut ui); // Ctrl+_ in legacy terminals
    press(KeyCode::Char('o'), ctrl, &mut client, &mut ui);
    assert_eq!(ui.input.text(), "\x02bold\x02\x034red\x1f\x0f");
    press(KeyCode::Enter, NONE, &mut client, &mut ui);
    assert_eq!(
        next_line(&mut received).await,
        "PRIVMSG #fmt \x02bold\x02\x034red\x1f\x0f"
    );

    // WeeChat: Ctrl+C then a letter; Ctrl+B moves left instead.
    type_line("/set ui.keymap weechat", &mut client, &mut ui);
    press(KeyCode::Char('c'), ctrl, &mut client, &mut ui);
    press(KeyCode::Char('i'), NONE, &mut client, &mut ui);
    type_text("it", &mut client, &mut ui);
    press(KeyCode::Char('b'), ctrl, &mut client, &mut ui);
    type_text("x", &mut client, &mut ui);
    assert_eq!(ui.input.text(), "\x1dixt");
    // An unknown letter after Ctrl+C inserts nothing.
    press(KeyCode::Char('c'), ctrl, &mut client, &mut ui);
    press(KeyCode::Char('z'), NONE, &mut client, &mut ui);
    assert_eq!(ui.input.text(), "\x1dixt");
    clear_input(&mut client, &mut ui);

    // /url lists links newest first; a bad number is refused.
    type_line(
        "see https://example.com/one and www.example.org.",
        &mut client,
        &mut ui,
    );
    type_line("and (https://example.net/a_(b))", &mut client, &mut ui);
    type_line("/url", &mut client, &mut ui);
    let channel = ui.active.unwrap();
    let lines: Vec<String> = client
        .state()
        .buffer(channel)
        .unwrap()
        .lines()
        .iter()
        .rev()
        .take(3)
        .map(|l| l.text.to_plain())
        .collect();
    assert_eq!(
        lines,
        [
            "  3. https://example.com/one",
            "  2. www.example.org",
            "  1. https://example.net/a_(b)",
        ]
    );
    type_line("/url 9", &mut client, &mut ui);
    let last = client
        .state()
        .buffer(channel)
        .unwrap()
        .lines()
        .back()
        .unwrap()
        .text
        .to_plain();
    assert_eq!(last, "No link 9; /url lists them");
    client.quit(None).await;
}
