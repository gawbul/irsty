//! The terminal screens on the project site (`site/index.html`) are real
//! Irsty renders: each scene below is built from IRC lines, drawn by the view
//! as the snapshot tests draw it, and turned into HTML. This test fails if
//! the page is out of date; `mise run site` (`IRSTY_UPDATE_SITE=1`) rewrites
//! it. It also embeds the example script the page shows.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test helpers outside #[test] functions"
)]

use std::fmt::Write as _;
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use irsty_core::config::{ThemeConfig, UiConfig};
use irsty_core::dcc::{Context, Dcc, Out};
use irsty_core::event::{NetEvent, NetworkId};
use irsty_core::state::{LineKind, State};
use irsty_proto::Message;
use irsty_tui::panes::Direction;
use irsty_tui::theme::Theme;
use irsty_tui::ui::{Switcher, Ui};
use irsty_tui::view;
use jiff::Timestamp;
use jiff::tz::TimeZone;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};
use unicode_width::UnicodeWidthStr;

const LIBERA: NetworkId = NetworkId(0);
const OFTC: NetworkId = NetworkId(1);

/// The tour's frames, and the accessibility section's.
const WIDE: (u16, u16) = (110, 22);

fn clock() -> Timestamp {
    "2026-11-08T09:40:00Z".parse().unwrap()
}

fn apply(state: &mut State, network: NetworkId, lines: &[&str]) {
    for line in lines {
        state.apply(network, NetEvent::Message(Message::parse(line).unwrap()));
    }
}

fn connect(state: &mut State, network: NetworkId, name: &str, server: &str) {
    state.add_network(network, name, "gawbul", Vec::new());
    state.apply(
        network,
        NetEvent::Connected {
            server: server.into(),
            insecure: false,
            local: None,
        },
    );
    state.apply(
        network,
        NetEvent::Registered {
            nick: "gawbul".into(),
        },
    );
    state.apply(network, NetEvent::Lag(Duration::from_millis(80)));
}

/// The lines of #irsty that arrive one by one when the page loads.
const ARRIVING: usize = 3;

fn world() -> State {
    world_but(0)
}

/// Two networks: a busy #irsty, a quieter #rust, a query, and #ratatui on
/// OFTC, with unread lines here and there for the activity list; all but
/// the last `held` lines of #irsty.
fn world_but(held: usize) -> State {
    let mut s = State::new(1000);
    s.set_clock(clock);
    connect(&mut s, LIBERA, "libera", "irc.libera.chat:6697 (TLS)");
    connect(&mut s, OFTC, "oftc", "irc.oftc.net:6697 (TLS)");
    let libera = [
        ":irc.libera.chat 005 gawbul PREFIX=(ov)@+ CHANTYPES=# CASEMAPPING=rfc1459 :are supported",
        "@time=2026-11-08T09:00:00Z :gawbul!~g@user/gawbul JOIN #rust",
        "@time=2026-11-08T09:00:00Z :irc.libera.chat 332 gawbul #rust :Rust programming language | https://www.rust-lang.org",
        "@time=2026-11-08T09:00:00Z :irc.libera.chat 353 gawbul = #rust :gawbul ferris kim lena",
        "@time=2026-11-08T09:00:00Z :irc.libera.chat 366 gawbul #rust :End of /NAMES list.",
        "@time=2026-11-08T09:20:00Z :kim!k@host PRIVMSG #rust :is there a lint for needless collect?",
        "@time=2026-11-08T09:21:00Z :lena!l@host PRIVMSG #rust :clippy::needless_collect, in the nursery for a while now",
        "@time=2026-11-08T09:29:00Z :ferris!f@host PRIVMSG gawbul :hey, got a minute for a scripting question?",
        "@time=2026-11-08T09:01:00Z :gawbul!~g@user/gawbul JOIN #irsty",
        "@time=2026-11-08T09:01:00Z :irc.libera.chat 332 gawbul #irsty :Irsty, a modern IRC client for the terminal | v0.1.0 is out: https://gawbul.github.io/irsty/",
        "@time=2026-11-08T09:01:00Z :irc.libera.chat 353 gawbul = #irsty :@gawbul +alice bob carol dave eve",
        "@time=2026-11-08T09:01:00Z :irc.libera.chat 366 gawbul #irsty :End of /NAMES list.",
        "@time=2026-11-08T09:30:00Z :alice!a@host PRIVMSG #irsty :morning all",
        "@time=2026-11-08T09:31:00Z :bob!b@host PRIVMSG #irsty :installed it with brew last night, and history just works through soju",
        "@time=2026-11-08T09:32:00Z :carol!c@host PRIVMSG #irsty :\x01ACTION puts the kettle on\x01",
        "@time=2026-11-08T09:33:00Z :dave!d@host PRIVMSG #irsty :does it do SASL?",
        "@time=2026-11-08T09:34:00Z :eve!e@host PRIVMSG #irsty :dave: PLAIN, SCRAM and EXTERNAL, and the password can live in your keyring",
        "@time=2026-11-08T09:35:00Z :dave!d@host PRIVMSG #irsty :\x02nice\x02, that was the last thing keeping me on weechat",
        "@time=2026-11-08T09:36:00Z :frank!f@host JOIN #irsty",
        "@time=2026-11-08T09:37:00Z :alice!a@host PRIVMSG #irsty :gawbul: the split windows are lovely, thank you",
    ];
    apply(&mut s, LIBERA, &libera[..libera.len() - held]);
    apply(
        &mut s,
        OFTC,
        &[
            "@time=2026-11-08T09:02:00Z :gawbul!~g@user/gawbul JOIN #ratatui",
            "@time=2026-11-08T09:02:00Z :irc.oftc.net 332 gawbul #ratatui :ratatui: cook up terminal user interfaces in Rust",
            "@time=2026-11-08T09:02:00Z :irc.oftc.net 353 gawbul = #ratatui :gawbul joshka orhun",
            "@time=2026-11-08T09:02:00Z :irc.oftc.net 366 gawbul #ratatui :End of /NAMES list.",
            "@time=2026-11-08T09:25:00Z :orhun!o@host PRIVMSG #ratatui :new release notes are up",
        ],
    );
    let read = s.find_buffer(LIBERA, "#rust").unwrap();
    s.mark_read(read);
    s
}

fn ui(state: &State, network: NetworkId, buffer: &str) -> Ui {
    let mut ui = Ui::new(&UiConfig::default(), state);
    ui.tz = TimeZone::UTC;
    ui.now = clock();
    ui.switch(state.find_buffer(network, buffer).unwrap());
    ui
}

fn draw(state: &State, ui: &mut Ui, (width, height): (u16, u16)) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| view::render(frame, state, ui))
        .unwrap();
    terminal.backend().buffer().clone()
}

/// The channel, with the nick list and the activity list, before the last
/// `held` lines arrive.
fn overview(held: usize) -> Buffer {
    let s = world_but(held);
    let mut ui = ui(&s, LIBERA, "#irsty");
    draw(&s, &mut ui, WIDE)
}

/// IRCv3: replies, reactions, a multi-line message and typing.
fn modern() -> Buffer {
    let mut s = world();
    s.apply(
        LIBERA,
        NetEvent::Caps {
            enabled: vec!["batch".into(), "draft/multiline".into()],
            values: vec![("draft/multiline".into(), "max-bytes=4096".into())],
        },
    );
    apply(
        &mut s,
        LIBERA,
        &[
            "@time=2026-11-08T09:39:00Z;msgid=q1 :bob!b@host PRIVMSG #irsty :shall we write the scripting guide next, or the migration notes?",
            "@time=2026-11-08T09:40:00Z;msgid=r1;+reply=q1 :carol!c@host PRIVMSG #irsty :bob: migration notes, people keep asking",
            "@+reply=q1;+draft/react=👍 :carol!c@host TAGMSG #irsty",
            "@+reply=q1;+draft/react=👍 :eve!e@host TAGMSG #irsty",
            "@+reply=q1;+draft/react=🎉 :dave!d@host TAGMSG #irsty",
            "@time=2026-11-08T09:40:00Z;msgid=ml BATCH +m draft/multiline #irsty",
            "@batch=m :eve!e@host PRIVMSG #irsty :what I'd want in them:",
            "@batch=m :eve!e@host PRIVMSG #irsty :- the keymap presets",
            "@batch=m :eve!e@host PRIVMSG #irsty :- moving my irssi config over",
            "BATCH -m",
            "@+typing=active :bob!b@host TAGMSG #irsty",
        ],
    );
    let mut ui = ui(&s, LIBERA, "#irsty");
    ui.now = clock() + jiff::SignedDuration::from_secs(1);
    draw(&s, &mut ui, WIDE)
}

/// Three windows at once.
fn splits() -> Buffer {
    let s = world();
    let mut ui = ui(&s, LIBERA, "#irsty");
    ui.show_nicklist = false;
    assert!(ui.split(Direction::SideBySide));
    ui.switch(s.find_buffer(LIBERA, "ferris").unwrap());
    assert!(ui.split(Direction::Stacked));
    ui.switch(s.find_buffer(OFTC, "#ratatui").unwrap());
    ui.focus_pane(0);
    draw(&s, &mut ui, WIDE)
}

/// The fuzzy switcher, part way through typing.
fn switcher() -> Buffer {
    let s = world();
    let mut ui = ui(&s, LIBERA, "#irsty");
    ui.switcher = Some(Switcher {
        query: "rat".into(),
        selected: 0,
    });
    draw(&s, &mut ui, WIDE)
}

/// The light theme (the page shows it on a light terminal).
fn light() -> Buffer {
    let s = world();
    let mut ui = ui(&s, LIBERA, "#irsty");
    ui.theme = Theme::light();
    draw(&s, &mut ui, WIDE)
}

/// A DCC chat, above the offer of a file as Irsty's DCC manager words it.
fn dcc() -> Buffer {
    let mut s = world();
    let mut dcc = Dcc::default();
    let request = irsty_proto::dcc::Dcc::parse("SEND notes.pdf 3221225985 5000 248320").unwrap();
    let dir = PathBuf::from("/tmp");
    let ctx = Context {
        dir: &dir,
        ports: None,
        advertise: Some(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 7))),
    };
    for out in dcc.on_request(LIBERA, "ferris", request, ctx) {
        if let Out::Note {
            network,
            kind,
            text,
        } = out
        {
            s.note(network, kind, &text);
        }
    }
    for (text, own) in [
        ("here are the notes from the meetup", false),
        ("thanks! I'll take them once this build finishes", true),
        ("no rush, the offer waits", false),
    ] {
        s.dcc_line(LIBERA, "ferris", text, own);
    }
    let chat = s.dcc_buffer(LIBERA, "ferris").unwrap();
    s.mark_read(chat);
    let mut ui = ui(&s, LIBERA, "=ferris");
    assert!(ui.split(Direction::Stacked));
    ui.switch(s.network(LIBERA).unwrap().server_buffer);
    ui.focus_pane(0);
    draw(&s, &mut ui, WIDE)
}

/// Accessible mode: one column of ASCII, with words for what colour shows.
fn accessible() -> Buffer {
    let mut s = world();
    s.apply(
        LIBERA,
        NetEvent::Caps {
            enabled: vec!["echo-message".into()],
            values: Vec::new(),
        },
    );
    let mut ui = ui(&s, LIBERA, "#irsty");
    ui.theme = Theme::from_config(&ThemeConfig::default(), true).0;
    apply(
        &mut s,
        LIBERA,
        &["@+typing=active :eve!e@host TAGMSG #irsty"],
    );
    s.own_message(
        LIBERA,
        "#irsty",
        LineKind::Message,
        "alice: glad you like them",
    );
    ui.now = clock() + jiff::SignedDuration::from_secs(1);
    draw(&s, &mut ui, (80, 18))
}

/// The terminal's sixteen colours, in ANSI order.
fn palette_index(colour: Color) -> Option<u8> {
    Some(match colour {
        Color::Black => 0,
        Color::Red => 1,
        Color::Green => 2,
        Color::Yellow => 3,
        Color::Blue => 4,
        Color::Magenta => 5,
        Color::Cyan => 6,
        Color::Gray => 7,
        Color::DarkGray => 8,
        Color::LightRed => 9,
        Color::LightGreen => 10,
        Color::LightYellow => 11,
        Color::LightBlue => 12,
        Color::LightMagenta => 13,
        Color::LightCyan => 14,
        Color::White => 15,
        Color::Indexed(n) if n < 16 => n,
        _ => return None,
    })
}

/// xterm's 256-colour cube and greys, for indexed colours above 15.
fn indexed_rgb(n: u8) -> (u8, u8, u8) {
    if n >= 232 {
        let v = 8 + (n - 232) * 10;
        return (v, v, v);
    }
    let n = n - 16;
    let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
    (level(n / 36), level((n / 6) % 6), level(n % 6))
}

/// A colour as a class (`f3`, `bd`: the terminal's palette, so the page's
/// CSS decides what it looks like) or, for anything else, a CSS value.
enum Paint {
    Class(String),
    Css(String),
}

fn paint(colour: Color, layer: char) -> Option<Paint> {
    if colour == Color::Reset {
        return None;
    }
    if let Some(n) = palette_index(colour) {
        return Some(Paint::Class(format!("{layer}{n}")));
    }
    let (r, g, b) = match colour {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Indexed(n) => indexed_rgb(n),
        _ => return None,
    };
    Some(Paint::Css(format!("#{r:02x}{g:02x}{b:02x}")))
}

/// One cell's look, as HTML attributes (empty for the default).
fn attributes(cell: &ratatui::buffer::Cell) -> String {
    let reversed = cell.modifier.contains(Modifier::REVERSED);
    let (fg, bg) = if reversed {
        (
            paint(cell.bg, 'f').or(Some(Paint::Class("fbg".into()))),
            paint(cell.fg, 'b').or(Some(Paint::Class("bfg".into()))),
        )
    } else {
        (paint(cell.fg, 'f'), paint(cell.bg, 'b'))
    };
    let mut classes = Vec::new();
    let mut style = Vec::new();
    for (paint, property) in [(fg, "color"), (bg, "background")] {
        match paint {
            Some(Paint::Class(class)) => classes.push(class),
            Some(Paint::Css(value)) => style.push(format!("{property}:{value}")),
            None => {}
        }
    }
    for (modifier, class) in [
        (Modifier::BOLD, "B"),
        (Modifier::DIM, "D"),
        (Modifier::ITALIC, "I"),
        (Modifier::UNDERLINED, "U"),
        (Modifier::CROSSED_OUT, "S"),
    ] {
        if cell.modifier.contains(modifier) {
            classes.push(class.into());
        }
    }
    let mut out = String::new();
    if !classes.is_empty() {
        let _ = write!(out, " class=\"{}\"", classes.join(" "));
    }
    if !style.is_empty() {
        let _ = write!(out, " style=\"{}\"", style.join(";"));
    }
    out
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The buffer as HTML: a row per line, runs of the same look as one span.
/// A wide character gets a span two columns wide, so the grid stays true
/// whatever width the browser's font gives it.
fn html(buffer: &Buffer) -> String {
    let area = buffer.area;
    let mut out = String::new();
    for y in 0..area.height {
        let mut run = String::new();
        let mut look = None::<String>;
        let flush = |out: &mut String, run: &mut String, look: &Option<String>| {
            if run.is_empty() {
                return;
            }
            match look.as_deref() {
                Some("") | None => out.push_str(run),
                Some(attrs) => {
                    let _ = write!(out, "<span{attrs}>{run}</span>");
                }
            }
            run.clear();
        };
        let mut x = 0;
        while x < area.width {
            let cell = &buffer[(x, y)];
            let symbol = cell.symbol();
            let width = symbol.width().max(1);
            let attrs = attributes(cell);
            if look.as_deref() != Some(attrs.as_str()) {
                flush(&mut out, &mut run, &look);
                look = Some(attrs.clone());
            }
            if width > 1 {
                flush(&mut out, &mut run, &look);
                let class = if attrs.contains("class=\"") {
                    attrs.replacen("class=\"", "class=\"w ", 1)
                } else {
                    format!(" class=\"w\"{attrs}")
                };
                let _ = write!(out, "<span{class}>{}</span>", escape(symbol));
            } else {
                run.push_str(&escape(symbol));
            }
            x += u16::try_from(width).unwrap();
        }
        flush(&mut out, &mut run, &look);
        let trimmed = out.trim_end_matches(' ').len();
        out.truncate(trimmed);
        out.push('\n');
    }
    out.pop();
    out
}

/// A Luau script as HTML, its comment lines marked so the page can dim them.
fn luau(script: &str) -> String {
    script
        .trim_end()
        .lines()
        .map(|line| {
            if line.trim_start().starts_with("--") {
                format!("<span class=\"c\">{}</span>", escape(line))
            } else {
                escape(line)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Replaces what sits between `<!-- irsty:KIND NAME -->` and
/// `<!-- /irsty:KIND -->` with `content`.
fn fill(page: &str, kind: &str, name: &str, content: &str) -> String {
    let open = format!("<!-- irsty:{kind} {name} -->");
    let close = format!("<!-- /irsty:{kind} -->");
    let start = page
        .find(&open)
        .unwrap_or_else(|| panic!("site/index.html has no {open}"))
        + open.len();
    let end = start
        + page[start..]
            .find(&close)
            .unwrap_or_else(|| panic!("site/index.html has no {close} after {open}"));
    format!("{}{content}{}", &page[..start], &page[end..])
}

#[test]
fn site_frames_are_current() {
    let path = root().join("site/index.html");
    let page = std::fs::read_to_string(&path).unwrap();
    let mut next = page.clone();
    for held in 1..=ARRIVING {
        next = fill(
            &next,
            "frame",
            &format!("overview-{held}"),
            &html(&overview(held)),
        );
    }
    for (name, buffer) in [
        ("overview", overview(0)),
        ("modern", modern()),
        ("splits", splits()),
        ("switcher", switcher()),
        ("light", light()),
        ("dcc", dcc()),
        ("accessible", accessible()),
    ] {
        next = fill(&next, "frame", name, &html(&buffer));
    }
    let script = std::fs::read_to_string(root().join("examples/scripts/slap.luau")).unwrap();
    next = fill(&next, "file", "examples/scripts/slap.luau", &luau(&script));
    if std::env::var_os("IRSTY_UPDATE_SITE").is_some() {
        std::fs::write(&path, &next).unwrap();
    } else {
        assert!(
            page == next,
            "site/index.html is out of date: run `mise run site` and review the diff"
        );
    }
}

#[test]
fn html_keeps_the_grid() {
    let s = world();
    let mut ui = ui(&s, LIBERA, "#irsty");
    let buffer = draw(&s, &mut ui, (60, 8));
    let text = html(&buffer);
    assert_eq!(text.lines().count(), 8);
    // Markup aside, no row is wider than the screen.
    for line in text.lines() {
        let mut plain = String::new();
        let mut in_tag = false;
        for c in line.chars() {
            match c {
                '<' => in_tag = true,
                '>' => in_tag = false,
                c if !in_tag => plain.push(c),
                _ => {}
            }
        }
        let plain = plain
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&");
        assert!(plain.width() <= 60, "{plain:?}");
    }
    assert!(!text.contains("<script"));
}
