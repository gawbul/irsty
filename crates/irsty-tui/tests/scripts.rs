//! M13 through the UI: loading a script with `/script`, its commands and
//! events, and what it may and may not do, against a scripted server.

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

fn lines(client: &Client, ui: &Ui, n: usize) -> Vec<String> {
    let mut out: Vec<String> = client
        .state()
        .buffer(ui.active.unwrap())
        .unwrap()
        .lines()
        .iter()
        .rev()
        .take(n)
        .map(|l| l.text.to_plain())
        .collect();
    out.reverse();
    out
}

const ECHO: &str = r#"
irsty.on("message", function(e)
    print("heard " .. e.nick .. ": " .. e.text .. (e.own and " (own)" or ""))
end)
irsty.command("shout", function(args, here)
    irsty.say(here.network, here.buffer, string.upper(args))
end)
"#;

#[tokio::test(flavor = "multi_thread")]
async fn scripts_from_the_input_line() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let (mut client, mut ui, mut received) = joined(&path, "#s", "").await;
    type_line("/window 2", &mut client, &mut ui);
    std::fs::create_dir(dir.path().join("scripts")).unwrap();
    std::fs::write(dir.path().join("scripts/echo.luau"), ECHO).unwrap();

    // Relative to the config file's directory; no capabilities.
    type_line("/script load scripts/echo.luau", &mut client, &mut ui);
    assert_eq!(lines(&client, &ui, 1), ["Loaded script echo"]);
    type_line("/script", &mut client, &mut ui);
    assert!(
        lines(&client, &ui, 1)[0].starts_with("echo: "),
        "{:?}",
        lines(&client, &ui, 1)
    );
    assert!(lines(&client, &ui, 1)[0].ends_with("; commands /shout"));

    // It hears our message, once; its own output isn't heard back.
    type_line("hello there", &mut client, &mut ui);
    assert_eq!(next_line(&mut received).await, "PRIVMSG #s :hello there");
    assert_eq!(
        lines(&client, &ui, 2),
        ["hello there", "heard tester: hello there (own)"]
    );

    // Sending needs the capability.
    type_line("/shout hi", &mut client, &mut ui);
    assert!(
        lines(&client, &ui, 1)[0].contains("may not send"),
        "{:?}",
        lines(&client, &ui, 1)
    );
    type_line(
        "/script load scripts/echo.luau -allow send",
        &mut client,
        &mut ui,
    );
    assert_eq!(lines(&client, &ui, 1), ["Loaded script echo (may send)"]);
    type_line("/shout hi", &mut client, &mut ui);
    assert_eq!(next_line(&mut received).await, "PRIVMSG #s HI");
    assert_eq!(
        lines(&client, &ui, 1),
        ["HI"],
        "its own message isn't heard back"
    );

    // /help lists script commands; unloading removes them.
    type_line("/help", &mut client, &mut ui);
    assert_eq!(lines(&client, &ui, 1), ["Script commands: /shout"]);
    type_line("/script unload echo", &mut client, &mut ui);
    type_line("/shout hi", &mut client, &mut ui);
    assert!(lines(&client, &ui, 1)[0].contains("Unknown command /shout"));
    type_line("/script load scripts/nothing.luau", &mut client, &mut ui);
    assert!(lines(&client, &ui, 1)[0].starts_with("Script not loaded:"));
    client.quit(None).await;
}
