//! M3 acceptance: drawing a 100,000-line buffer must take under 2 ms a frame,
//! wherever it is scrolled to.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use irsty_core::config::UiConfig;
use irsty_core::event::{NetEvent, NetworkId};
use irsty_core::state::State;
use irsty_proto::Message;
use irsty_tui::scrollback::Scrollback;
use irsty_tui::ui::Ui;
use irsty_tui::view;
use jiff::tz::TimeZone;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;

const LINES: usize = 100_000;

fn busy_state() -> State {
    let mut state = State::new(LINES);
    let net = NetworkId(0);
    state.add_network(net, "bench", "me", Vec::new());
    let texts = [
        "short line",
        "a longer line with \x02bold\x02 and \x0304colour\x03 that will wrap on narrower terminals because it keeps going for a while",
        "漢字とかなのテスト with emoji 👍🏽 ❤️ mixed in",
    ];
    for i in 0..LINES {
        let text = texts.get(i % texts.len()).copied().unwrap_or_default();
        let line = format!(":nick{}!u@h PRIVMSG #busy :{text} {i}", i % 50);
        if let Ok(msg) = Message::parse(&line) {
            state.apply(net, NetEvent::Message(msg));
        }
    }
    state
}

fn bench(c: &mut Criterion) {
    let state = busy_state();
    let buffer_id = state
        .find_buffer(NetworkId(0), "#busy")
        .unwrap_or_else(|| unreachable!());
    let buffer = state.buffer(buffer_id).unwrap_or_else(|| unreachable!());
    assert_eq!(buffer.lines().len(), LINES);
    let tz = TimeZone::UTC;

    for (name, offset) in [("bottom", 0), ("middle", LINES / 2)] {
        c.bench_function(&format!("scrollback/100k/{name}/200x50"), |b| {
            let area = Rect::new(0, 0, 200, 50);
            let mut cells = Buffer::empty(area);
            b.iter(|| {
                cells.reset();
                Scrollback {
                    buffer,
                    offset,
                    time_format: "%H:%M",
                    tz: &tz,
                    nick_width: 10,
                    colours: &irsty_tui::theme::NickColours::default(),
                }
                .render(area, &mut cells);
                black_box(&cells);
            });
        });
    }

    c.bench_function("screen/100k/200x50", |b| {
        let mut ui = Ui::new(&UiConfig::default(), &state);
        ui.switch(buffer_id);
        ui.tz = TimeZone::UTC;
        let mut terminal =
            Terminal::new(TestBackend::new(200, 50)).unwrap_or_else(|_| unreachable!());
        b.iter(|| {
            terminal
                .draw(|frame| view::render(frame, &state, &mut ui))
                .unwrap_or_else(|_| unreachable!());
        });
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
