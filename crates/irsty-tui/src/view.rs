//! Drawing the whole screen from [`State`] and [`Ui`]. Pure: no I/O, no
//! mutation of client state, so it is snapshot-tested with `TestBackend`.

use irsty_core::state::{BufferKind, Network, State, Status};
use irsty_proto::casemap::CaseMapping;
use irsty_proto::format::sanitise_identifier;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::scrollback::Scrollback;
use crate::theme;
use crate::ui::Ui;

/// Width of the buffer list.
const BUFFER_LIST: u16 = 18;
/// Width of the nick list.
const NICK_LIST: u16 = 16;
/// Width of the nick column in the scrollback.
const NICK_COLUMN: usize = 10;

/// Draws everything.
pub fn render(frame: &mut Frame<'_>, state: &State, ui: &mut Ui) {
    let area = frame.area();
    if area.width < 20 || area.height < 5 {
        frame.render_widget(
            Paragraph::new("Terminal too small").wrap(Wrap { trim: true }),
            area,
        );
        return;
    }
    let [topic, middle, status, input] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);

    let active = ui.active.and_then(|id| state.buffer(id));
    let network = active.and_then(|b| state.network(b.network));
    let channel = active
        .filter(|b| b.kind == BufferKind::Channel)
        .and_then(|b| network.and_then(|n| n.channel(&b.name)));

    let list_width = if area.width >= 60 { BUFFER_LIST } else { 0 };
    let nick_width = if channel.is_some() && ui.show_nicklist && area.width >= 100 {
        NICK_LIST
    } else {
        0
    };
    let [list, main, nicks] = Layout::horizontal([
        Constraint::Length(list_width),
        Constraint::Min(1),
        Constraint::Length(nick_width),
    ])
    .areas(middle);

    render_topic(frame, topic, state, ui);
    render_buffer_list(frame, list, state, ui);
    ui.page = usize::from(main.height);
    match active {
        Some(buffer) => frame.render_widget(
            Scrollback {
                buffer,
                offset: ui.offset(),
                time_format: &ui.time_format,
                tz: &ui.tz,
                nick_width: NICK_COLUMN,
            },
            main,
        ),
        None => frame.render_widget(first_run(ui), main),
    }
    if let (Some(channel), Some(net)) = (channel, network)
        && nick_width > 0
    {
        render_nicks(frame, nicks, channel, net);
    }
    render_status(frame, status, state, ui);
    render_input(frame, input, state, ui);
    if ui.switcher.is_some() {
        render_switcher(frame, middle, state, ui);
    }
}

/// The fuzzy buffer switcher: a box over the middle of the screen with the
/// query and the best matches.
fn render_switcher(frame: &mut Frame<'_>, area: Rect, state: &State, ui: &Ui) {
    let Some(switcher) = &ui.switcher else {
        return;
    };
    let matches = ui.switcher_matches(state);
    let width = area.width.saturating_sub(4).min(60);
    let rows = u16::try_from(matches.len())
        .unwrap_or(u16::MAX)
        .clamp(1, 12);
    let height = (rows + 3).min(area.height);
    let popup = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + area.height.saturating_sub(height) / 3,
        width,
        height,
    );
    frame.render_widget(ratatui::widgets::Clear, popup);
    let block = ratatui::widgets::Block::bordered()
        .title(" Go to buffer (Enter, Esc) ")
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    let mut lines = vec![Line::from(vec![
        Span::styled("> ", Style::default().fg(Color::Cyan)),
        Span::raw(sanitise_identifier(&switcher.query)),
    ])];
    let visible = usize::from(inner.height.saturating_sub(1));
    let skip = (switcher.selected + 1).saturating_sub(visible);
    for (row, (number, id, label)) in matches.iter().enumerate().skip(skip).take(visible) {
        let activity = state.buffer(*id).map(|b| b.activity).unwrap_or_default();
        let mut style = theme::activity(activity);
        if row == switcher.selected {
            style = theme::selected();
        }
        lines.push(Line::from(Span::styled(
            format!("{number:>3}. {}", sanitise_identifier(label)),
            style,
        )));
    }
    if matches.is_empty() {
        lines.push(Line::from(Span::styled("No match", theme::dim())));
    }
    frame.render_widget(Paragraph::new(lines), inner);
    let cursor = u16::try_from(2 + switcher.query.width()).unwrap_or(0);
    frame.set_cursor_position(Position::new(
        (inner.x + cursor).min(inner.right().saturating_sub(1)),
        inner.y,
    ));
}

fn render_topic(frame: &mut Frame<'_>, area: Rect, state: &State, ui: &Ui) {
    let buffer = ui.active.and_then(|id| state.buffer(id));
    let network = buffer.and_then(|b| state.network(b.network));
    let mut spans = Vec::new();
    match (buffer, network) {
        (Some(b), Some(net)) if b.kind == BufferKind::Channel => {
            if let Some(topic) = net.channel(&b.name).and_then(|c| c.topic.as_ref()) {
                spans.extend(
                    topic
                        .spans()
                        .iter()
                        .map(|s| Span::styled(s.text().to_owned(), theme::span_style(s.style()))),
                );
            }
        }
        (Some(b), Some(_)) if b.kind == BufferKind::Query => {
            spans.push(Span::raw(format!(
                "Private conversation with {}",
                sanitise_identifier(&b.name)
            )));
        }
        (Some(_), Some(net)) => spans.push(Span::raw(format!(
            "{}: {}",
            sanitise_identifier(&net.name),
            status_text(net)
        ))),
        _ => spans.push(Span::raw("Irsty")),
    }
    frame.render_widget(Paragraph::new(Line::from(spans)).style(theme::bar()), area);
}

fn status_text(net: &Network) -> &'static str {
    match net.status {
        Status::Disconnected => "disconnected",
        Status::Connecting => "connecting",
        Status::Registering => "registering",
        Status::Registered => "connected",
        Status::Waiting => "waiting to reconnect",
    }
}

fn render_buffer_list(frame: &mut Frame<'_>, area: Rect, state: &State, ui: &mut Ui) {
    let order = ui.order(state);
    ui.buffer_list = (area, order.clone());
    if area.width == 0 {
        return;
    }
    // Keep the active buffer visible when the list is taller than the screen.
    let height = usize::from(area.height);
    let active_index = ui
        .active
        .and_then(|a| order.iter().position(|&b| b == a))
        .unwrap_or(0);
    let skip = (active_index + 1).saturating_sub(height);
    let width = usize::from(area.width.saturating_sub(1));
    let lines: Vec<Line<'_>> = order
        .iter()
        .enumerate()
        .skip(skip)
        .take(height)
        .filter_map(|(index, &id)| {
            let buffer = state.buffer(id)?;
            let name = sanitise_identifier(&buffer.name);
            let label = match buffer.kind {
                BufferKind::Server => format!("{:>2}.{name}", index + 1),
                _ => format!("{:>2}.  {name}", index + 1),
            };
            let label = truncate(&label, width);
            let style = if Some(id) == ui.active {
                theme::selected()
            } else if buffer.kind == BufferKind::Server {
                theme::activity(buffer.activity).add_modifier(Modifier::BOLD)
            } else {
                theme::activity(buffer.activity)
            };
            Some(Line::from(Span::styled(format!("{label:<width$}"), style)))
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
    // A divider down the right-hand edge.
    let divider = Rect::new(area.right().saturating_sub(1), area.y, 1, area.height);
    let bar: Vec<Line<'_>> = (0..area.height).map(|_| Line::from("│")).collect();
    frame.render_widget(Paragraph::new(bar).style(theme::dim()), divider);
}

/// What to show before there is any network: how to add one, then the
/// output of commands typed so far.
fn first_run(ui: &Ui) -> Paragraph<'static> {
    let mut lines = vec![
        Line::from(Span::styled(
            "Welcome to Irsty.",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("No networks yet. Add one, for example:"),
        Line::from(Span::styled(
            "  /server add libera irc.libera.chat -nick yournick -autojoin #irsty",
            Style::default().fg(Color::LightCyan),
        )),
        Line::from(Span::styled(
            "  /help server for every option, /help for all commands.",
            theme::dim(),
        )),
        Line::from(""),
    ];
    lines.extend(ui.status.iter().map(|(kind, text)| {
        Line::from(Span::styled(
            irsty_proto::format::StyledText::plain(text).to_plain(),
            theme::line_style(*kind),
        ))
    }));
    Paragraph::new(lines).wrap(Wrap { trim: false })
}

fn render_nicks(
    frame: &mut Frame<'_>,
    area: Rect,
    channel: &irsty_core::state::Channel,
    net: &Network,
) {
    let mapping: CaseMapping = net.isupport.casemapping;
    let mut members: Vec<_> = channel.members().collect();
    members.sort_by_cached_key(|m| {
        let rank = m
            .modes
            .chars()
            .next()
            .map_or(usize::MAX, |c| net.isupport.mode_rank(c));
        (rank, mapping.fold(&m.nick).into_owned())
    });
    let height = usize::from(area.height);
    let overflow = members.len() > height;
    let shown = if overflow {
        height.saturating_sub(1)
    } else {
        height
    };
    let width = usize::from(area.width.saturating_sub(1));
    let mut lines: Vec<Line<'_>> = members
        .iter()
        .take(shown)
        .map(|m| {
            let symbol = m
                .modes
                .chars()
                .next()
                .and_then(|c| net.isupport.symbol_for_mode(c))
                .map_or(" ".to_owned(), String::from);
            let nick = sanitise_identifier(&m.nick);
            // Away users (away-notify) are dimmed.
            let style = if net.user(&m.nick).is_some_and(|u| u.away.is_some()) {
                theme::dim()
            } else {
                Style::default().fg(theme::nick_color(&nick))
            };
            Line::from(vec![
                Span::raw(" "),
                Span::styled(symbol, Style::default().fg(Color::LightGreen)),
                Span::styled(truncate(&nick, width.saturating_sub(2)), style),
            ])
        })
        .collect();
    if overflow {
        lines.push(Line::from(Span::styled(
            format!(" +{} more", members.len() - shown),
            theme::dim(),
        )));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_status(frame: &mut Frame<'_>, area: Rect, state: &State, ui: &Ui) {
    let buffer = ui.active.and_then(|id| state.buffer(id));
    let network = buffer.and_then(|b| state.network(b.network));
    let clock = ui.now.to_zoned(ui.tz.clone()).strftime("%H:%M").to_string();
    let mut spans = vec![Span::raw(format!("[{clock}] "))];
    if let Some(net) = network {
        spans.push(Span::raw(format!("[{}] ", sanitise_identifier(&net.nick))));
    }
    if let (Some(b), Some(net)) = (buffer, network) {
        let number = ui
            .order(state)
            .iter()
            .position(|&id| id == b.id)
            .map_or(0, |i| i + 1);
        let name = if b.kind == BufferKind::Server {
            String::new()
        } else {
            format!("/{}", sanitise_identifier(&b.name))
        };
        spans.push(Span::raw(format!(
            "[{number}:{}{name}] ",
            sanitise_identifier(&net.name)
        )));
    }
    let active: Vec<(usize, Style)> = ui
        .order(state)
        .iter()
        .enumerate()
        .filter(|&(_, &id)| Some(id) != ui.active)
        .filter_map(|(i, &id)| state.buffer(id).map(|b| (i + 1, b.activity)))
        .filter(|(_, a)| *a > irsty_core::state::Activity::Events)
        .map(|(i, a)| {
            (
                i,
                theme::activity(a).bg(theme::bar().bg.unwrap_or(Color::Reset)),
            )
        })
        .collect();
    if !active.is_empty() {
        spans.push(Span::raw("[Act: "));
        for (n, (number, style)) in active.iter().enumerate() {
            if n > 0 {
                spans.push(Span::raw(","));
            }
            spans.push(Span::styled(number.to_string(), *style));
        }
        spans.push(Span::raw("] "));
    }
    if let Some(net) = network {
        if net.status == Status::Registered {
            if let Some(lag) = net.lag.filter(|l| l.as_millis() >= 500) {
                spans.push(Span::raw(format!("[Lag: {:.1}s] ", lag.as_secs_f64())));
            }
        } else {
            spans.push(Span::raw(format!("[{}] ", status_text(net))));
        }
        if net.insecure {
            spans.push(Span::styled(
                "[INSECURE TLS] ",
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Red)
                    .add_modifier(Modifier::BOLD),
            ));
        }
    }
    if ui.offset() > 0 {
        spans.push(Span::styled(
            "-- more --",
            Style::default().add_modifier(Modifier::BOLD),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)).style(theme::bar()), area);
}

fn render_input(frame: &mut Frame<'_>, area: Rect, state: &State, ui: &Ui) {
    // A multi-line paste waits for an answer instead of the input.
    if let Some(lines) = &ui.paste {
        let target = ui
            .target(state)
            .map_or_else(|| "this buffer".to_owned(), |(_, n)| sanitise_identifier(n));
        let text = format!(
            "Paste {} lines to {target}? Enter sends them, J joins them into one line, Esc cancels",
            lines.len()
        );
        frame.render_widget(
            Paragraph::new(Span::styled(
                text,
                Style::default().fg(Color::Black).bg(Color::Yellow),
            )),
            area,
        );
        return;
    }
    if let Some(digits) = &ui.jump {
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!("Jump to buffer: {digits}_"),
                Style::default().fg(Color::Cyan),
            )),
            area,
        );
        return;
    }
    let prompt = match ui.target(state) {
        Some((_, name)) => format!("[{}] ", sanitise_identifier(name)),
        None => "> ".to_owned(),
    };
    let prompt = truncate(&prompt, usize::from(area.width / 3));
    let prompt_width = prompt.width();
    let room = usize::from(area.width).saturating_sub(prompt_width + 1);
    let (visible, cursor) = ui.input.view(room);
    // The typed text is the user's own, but formatting codes in it must not
    // reach the terminal raw.
    let shown: String = visible
        .chars()
        .map(|c| if c.is_control() { '·' } else { c })
        .collect();
    let line = Line::from(vec![
        Span::styled(prompt, Style::default().fg(Color::Cyan)),
        Span::raw(shown),
    ]);
    frame.render_widget(Paragraph::new(line), area);
    let x = area.x
        + u16::try_from(prompt_width + cursor)
            .unwrap_or(area.width)
            .min(area.width.saturating_sub(1));
    frame.set_cursor_position(Position::new(x, area.y));
}

/// Cuts `text` to `width` columns.
fn truncate(text: &str, width: usize) -> String {
    use unicode_segmentation::UnicodeSegmentation;
    let mut used = 0;
    text.graphemes(true)
        .take_while(|g| {
            used += g.width();
            used <= width
        })
        .collect()
}
