//! Snapshot tests of the whole screen, rendered with ratatui's `TestBackend`.
//! Review changes with `cargo insta review`.

#![allow(clippy::unwrap_used, reason = "test helpers outside #[test] functions")]

use std::time::Duration;

use irsty_core::config::UiConfig;
use irsty_core::event::{NetEvent, NetworkId};
use irsty_core::state::{State, Status};
use irsty_proto::Message;
use irsty_tui::ui::Ui;
use irsty_tui::view;
use jiff::Timestamp;
use jiff::tz::TimeZone;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

const NET: NetworkId = NetworkId(0);

/// A network with one busy channel and a query.
fn fixed_clock() -> Timestamp {
    "2026-10-30T08:59:00Z".parse().unwrap()
}

fn state() -> State {
    let mut s = State::new(1000);
    s.set_clock(fixed_clock);
    s.add_network(NET, "libera", "gawbul", Vec::new());
    s.apply(
        NET,
        NetEvent::Connected {
            server: "irc.libera.chat:6697 (TLS)".into(),
            insecure: false,
        },
    );
    s.apply(
        NET,
        NetEvent::Registered {
            nick: "gawbul".into(),
        },
    );
    s.apply(NET, NetEvent::Lag(Duration::from_millis(1200)));
    let lines = [
        ":irc.libera.chat 005 gawbul PREFIX=(ov)@+ CHANTYPES=# CASEMAPPING=rfc1459 :are supported",
        "@time=2026-10-30T09:00:00Z :gawbul!~g@user/gawbul JOIN #irsty",
        ":irc.libera.chat 332 gawbul #irsty :Irsty: a terminal IRC client \x0304in Rust\x03 | https://github.com/gawbul/irsty",
        ":irc.libera.chat 353 gawbul = #irsty :@gawbul +alice bob carol dave_ eve",
        ":irc.libera.chat 366 gawbul #irsty :End of /NAMES list.",
        "@time=2026-10-30T09:01:00Z :alice!a@host JOIN #irsty",
        "@time=2026-10-30T09:02:00Z :alice!a@host PRIVMSG #irsty :morning all",
        "@time=2026-10-30T09:02:30Z :bob!b@host PRIVMSG #irsty :\x02bold\x02 and \x1ditalic\x1d and \x0303green\x03 text",
        "@time=2026-10-30T09:03:00Z :carol!c@host PRIVMSG #irsty :\x01ACTION puts the kettle on\x01",
        "@time=2026-10-30T09:04:00Z :dave_!d@host PRIVMSG #irsty :this is a much longer message that will need to wrap onto a second row on an eighty column terminal so we can check the hanging indent",
        "@time=2026-10-30T09:05:00Z :eve!e@host PRIVMSG #irsty :漢字とかなのテスト and emoji 👍🏽 ❤️ 🇬🇧 mixed in",
        "@time=2026-10-30T09:05:30Z :mallory!m@host PRIVMSG #irsty :\x1b]0;pwned\x07 \x1b[2Jtrying escapes",
        "@time=2026-10-30T09:06:00Z :averyveryverylongnickname!x@host PRIVMSG #irsty :long nicks are truncated",
        "@time=2026-10-30T09:07:00Z :ChanServ!cs@services NOTICE #irsty :Welcome to #irsty",
        "@time=2026-10-30T09:08:00Z :bob!b@host PART #irsty :lunch",
        "@time=2026-10-30T09:09:00Z :alice!a@host PRIVMSG #irsty :gawbul: are you there?",
        "@time=2026-10-30T09:10:00Z :frank!f@host PRIVMSG gawbul :psst, private message",
    ];
    for line in lines {
        s.apply(NET, NetEvent::Message(Message::parse(line).unwrap()));
    }
    s
}

fn ui(state: &State, buffer: &str) -> Ui {
    let mut ui = Ui::new(&UiConfig::default(), state);
    ui.tz = TimeZone::UTC;
    ui.now = "2026-10-30T09:15:00Z".parse::<Timestamp>().unwrap();
    if let Some(id) = state.find_buffer(NET, buffer) {
        ui.switch(id);
    }
    ui
}

fn render(state: &State, ui: &mut Ui, width: u16, height: u16) -> Terminal<TestBackend> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| view::render(frame, state, ui))
        .unwrap();
    terminal
}

#[test]
fn channel_80x24() {
    let s = state();
    let mut ui = ui(&s, "#irsty");
    ui.input.insert("typing a reply");
    insta::assert_snapshot!(render(&s, &mut ui, 80, 24).backend());
}

#[test]
fn channel_200x50() {
    let s = state();
    let mut ui = ui(&s, "#irsty");
    insta::assert_snapshot!(render(&s, &mut ui, 200, 50).backend());
}

#[test]
fn server_buffer_narrow() {
    let s = state();
    let mut ui = ui(&s, "libera");
    insta::assert_snapshot!(render(&s, &mut ui, 50, 12).backend());
}

#[test]
fn scrolled_back() {
    let s = state();
    let mut ui = ui(&s, "#irsty");
    ui.scroll_by(&s, 5);
    insta::assert_snapshot!(render(&s, &mut ui, 80, 16).backend());
}

#[test]
fn insecure_and_disconnected() {
    let mut s = state();
    s.apply(
        NET,
        NetEvent::Connected {
            server: "bnc.example:6697 (TLS)".into(),
            insecure: true,
        },
    );
    s.apply(
        NET,
        NetEvent::Disconnected {
            reason: "ping timeout".into(),
            reconnect_in: Some(Duration::from_secs(8)),
        },
    );
    assert_eq!(s.network(NET).unwrap().status, Status::Waiting);
    let mut ui = ui(&s, "libera");
    insta::assert_snapshot!(render(&s, &mut ui, 80, 10).backend());
}

#[test]
fn too_small() {
    let s = state();
    let mut ui = ui(&s, "#irsty");
    insta::assert_snapshot!(render(&s, &mut ui, 15, 4).backend());
}

#[test]
fn no_networks() {
    let mut s = State::new(10);
    s.set_clock(fixed_clock);
    let mut ui = ui(&s, "");
    insta::assert_snapshot!(render(&s, &mut ui, 60, 8).backend());
}
