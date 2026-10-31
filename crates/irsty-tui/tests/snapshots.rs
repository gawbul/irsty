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

/// With echo-message, our lines wait for the server's echo; one that couldn't
/// be sent says why.
#[test]
fn pending_and_failed_own_messages() {
    let mut s = state();
    s.apply(
        NET,
        NetEvent::Caps {
            enabled: vec!["echo-message".into()],
            values: Vec::new(),
        },
    );
    let own = s.own_message(
        NET,
        "#irsty",
        irsty_core::state::LineKind::Message,
        "on my way",
    );
    assert!(own.buffer.is_some());
    let own = s.own_message(
        NET,
        "#irsty",
        irsty_core::state::LineKind::Message,
        "lost one",
    );
    s.fail_newest(own.buffer.unwrap(), 1, "send queue full");
    let mut ui = ui(&s, "#irsty");
    insta::assert_debug_snapshot!(render(&s, &mut ui, 80, 8).backend());
}

/// away-notify: away users are dimmed in the nick list.
#[test]
fn away_users_are_dimmed_in_the_nick_list() {
    let mut s = state();
    s.apply(
        NET,
        NetEvent::Message(Message::parse(":carol!c@host AWAY :gone fishing").unwrap()),
    );
    let mut ui = ui(&s, "#irsty");
    let terminal = render(&s, &mut ui, 120, 24);
    let buffer = terminal.backend().buffer();
    let colour_of = |nick: &str| {
        (0..buffer.area.height)
            .find_map(|y| {
                let row: String = (100..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect();
                row.find(nick)
                    .map(|col| buffer[(100 + u16::try_from(col).unwrap(), y)].fg)
            })
            .unwrap()
    };
    assert_eq!(colour_of("carol"), ratatui::style::Color::DarkGray);
    assert_ne!(colour_of("dave_"), ratatui::style::Color::DarkGray);
}

/// A reply shows what it answers, reactions follow their message, a
/// multi-line message reads as one block, and the status bar says who is
/// typing.
#[test]
fn replies_reactions_multiline_and_typing() {
    let mut s = state();
    s.apply(
        NET,
        NetEvent::Caps {
            enabled: vec!["batch".into(), "draft/multiline".into()],
            values: vec![("draft/multiline".into(), "max-bytes=4096".into())],
        },
    );
    let lines = [
        "@time=2026-10-30T09:11:00Z;msgid=q1 :alice!a@host PRIVMSG #irsty :shall we tag a release today, or wait for the docs?",
        "@time=2026-10-30T09:12:00Z;msgid=r1;+reply=q1 :carol!c@host PRIVMSG #irsty :alice: after the docs land",
        "@+reply=q1;+draft/react=👍 :carol!c@host TAGMSG #irsty",
        "@+reply=q1;+draft/react=👍 :eve!e@host TAGMSG #irsty",
        "@+reply=q1;+draft/react=🎉 :dave_!d@host TAGMSG #irsty",
        "@time=2026-10-30T09:13:00Z;+reply=gone :eve!e@host PRIVMSG #irsty :replying to something long gone",
        "@time=2026-10-30T09:14:00Z;msgid=ml BATCH +m draft/multiline #irsty",
        "@batch=m :dave_!d@host PRIVMSG #irsty :release checklist:",
        "@batch=m :dave_!d@host PRIVMSG #irsty :- docs",
        "@batch=m :dave_!d@host PRIVMSG #irsty :- changelog",
        "BATCH -m",
        "@+typing=active :eve!e@host TAGMSG #irsty",
        "@+typing=paused :carol!c@host TAGMSG #irsty",
    ];
    for line in lines {
        s.apply(NET, NetEvent::Message(Message::parse(line).unwrap()));
    }
    let mut ui = ui(&s, "#irsty");
    // Two seconds after the typing notifications arrived.
    ui.now = fixed_clock() + jiff::SignedDuration::from_secs(2);
    insta::assert_snapshot!(render(&s, &mut ui, 100, 14).backend());
}
