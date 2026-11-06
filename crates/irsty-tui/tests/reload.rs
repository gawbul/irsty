//! M12 through the UI: `/reload` reads the config file again and applies
//! it, networks and theme included; a broken file changes nothing.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers outside #[test] functions"
)]

mod support;

use irsty_core::Client;
use irsty_tui::theme::Theme;
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

fn names(client: &Client) -> Vec<String> {
    client.state().networks().map(|n| n.name.clone()).collect()
}

const OTHER: &str = r#"
[[network]]
name = "other"
nicks = ["someone"]
autoconnect = false

[[network.servers]]
host = "127.0.0.1"
"#;

#[tokio::test(flavor = "multi_thread")]
async fn reload_applies_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let (mut client, mut ui, _received) = joined(&path, "#r", "").await;
    let base = std::fs::read_to_string(&path).unwrap();

    // A theme change applies at once.
    std::fs::write(&path, format!("{base}\n[theme]\npreset = \"light\"\n")).unwrap();
    type_line("/reload", &mut client, &mut ui);
    assert!(
        last_line(&client, &ui).starts_with("Reloaded"),
        "{}",
        last_line(&client, &ui)
    );
    assert_eq!(ui.theme, Theme::light());

    // A file that doesn't parse changes nothing.
    std::fs::write(&path, "scrollback = \n").unwrap();
    type_line("/reload", &mut client, &mut ui);
    assert!(
        last_line(&client, &ui).starts_with("Not reloaded"),
        "{}",
        last_line(&client, &ui)
    );
    assert_eq!(ui.theme, Theme::light());
    assert_eq!(names(&client), ["mock"]);

    // A new network is added; a bad theme element is reported.
    std::fs::write(
        &path,
        format!("{base}\n[theme]\npreset = \"light\"\nsparkle = \"red\"\n{OTHER}"),
    )
    .unwrap();
    type_line("/reload", &mut client, &mut ui);
    assert_eq!(names(&client), ["mock", "other"]);
    let lines: Vec<String> = client
        .state()
        .buffer(ui.active.unwrap())
        .unwrap()
        .lines()
        .iter()
        .rev()
        .take(2)
        .map(|l| l.text.to_plain())
        .collect();
    assert!(lines[0].contains("sparkle"), "{lines:?}");
    assert!(
        lines[1].contains("networks 1 added, 0 changed (reconnecting), 0 removed"),
        "{lines:?}"
    );

    // Gone from the file: closed here. Logging changes wait for a restart.
    let without_mock = OTHER.to_owned() + "\n[logging]\nrotate = \"daily\"\n";
    std::fs::write(&path, without_mock).unwrap();
    type_line("/reload", &mut client, &mut ui);
    assert_eq!(names(&client), ["other"]);
    let last = last_line(&client, &ui);
    assert!(
        last.contains("1 removed") && last.contains("logging take effect when Irsty restarts"),
        "{last}"
    );
    client.quit(None).await;
}
