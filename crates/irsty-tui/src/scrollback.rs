//! The scrollback widget (ADR 0003).
//!
//! Lines are laid out from the bottom up, so only the lines on screen are
//! wrapped: drawing cost depends on the window size, not the buffer size.
//! Each line has a timestamp, a right-aligned nick column and a separator;
//! wrapped text hangs under the text column.

use irsty_core::state::{Buffer, Delivery, Line, LineKind};
use jiff::tz::TimeZone;
use ratatui::buffer::Buffer as Cells;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Widget;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::theme::Theme;
use crate::{theme, urls};

/// Separator between the nick column and the text.
const SEPARATOR: &str = " │ ";
/// The same in ASCII, for `[ui] accessible`.
const SEPARATOR_ASCII: &str = " | ";

/// The separator for a theme.
fn separator(theme: &Theme) -> &'static str {
    if theme.accessible {
        SEPARATOR_ASCII
    } else {
        SEPARATOR
    }
}

/// Draws a buffer's lines.
#[derive(Debug)]
pub struct Scrollback<'a> {
    /// The buffer to draw.
    pub buffer: &'a Buffer,
    /// Lines scrolled back from the bottom.
    pub offset: usize,
    /// `strftime` format for timestamps.
    pub time_format: &'a str,
    /// Time zone for timestamps.
    pub tz: &'a TimeZone,
    /// Width of the nick column.
    pub nick_width: usize,
    /// How to colour nicks.
    pub colours: &'a theme::NickColours,
    /// Styles.
    pub theme: &'a Theme,
    /// The number of the last line read before the buffer came on screen:
    /// a separator goes above the first line after it.
    pub marker: Option<u64>,
}

/// Columns for the time, the nick and the text in an area.
struct Geometry {
    time: usize,
    nick: usize,
    text: usize,
}

impl Scrollback<'_> {
    fn geometry(&self, area: Rect) -> Geometry {
        let sample_time = self
            .buffer
            .lines()
            .back()
            .map(|l| {
                l.time
                    .to_zoned(self.tz.clone())
                    .strftime(self.time_format)
                    .to_string()
            })
            .unwrap_or_default();
        let time_width = sample_time.width();
        let full = usize::from(area.width);
        // On narrow screens, shrink the nick column before giving up on it.
        let nick_width = self.nick_width.min(full / 4).max(3);
        let gutter_width = time_width + 1 + nick_width + SEPARATOR.width();
        Geometry {
            time: time_width,
            nick: nick_width,
            text: full.saturating_sub(gutter_width).max(1),
        }
    }

    /// How far to scroll back so the line numbered `first` (with the unread
    /// separator above it) is as near the top of `area` as the lines after
    /// it allow.
    pub fn offset_showing(&self, area: Rect, first: u64) -> usize {
        let lines = self.buffer.lines();
        let held = u64::try_from(lines.len()).unwrap_or(u64::MAX);
        let oldest = (self.buffer.total + 1).saturating_sub(held);
        let skip = usize::try_from(first.saturating_sub(oldest)).unwrap_or(usize::MAX);
        let width = self.geometry(area).text;
        let rows: Vec<usize> = lines
            .iter()
            .skip(skip)
            .map(|line| {
                let counts = reaction_counts(line);
                wrap(&pieces(line, self.colours, self.theme, &counts), width).len()
            })
            .collect();
        let height = usize::from(area.height);
        let mut used = 1 + rows.iter().sum::<usize>();
        let mut offset = 0;
        while used > height && offset + 1 < rows.len() {
            used -= rows.get(rows.len() - 1 - offset).copied().unwrap_or(0);
            offset += 1;
        }
        offset
    }
}

/// One piece of text with a style.
type Piece<'a> = (&'a str, Style);

/// The nick column for a line: text and style.
fn gutter(line: &Line, colours: &theme::NickColours, theme: &Theme) -> (String, Style) {
    let nick = line.sender.as_deref().unwrap_or("");
    if line.highlight && matches!(line.kind, LineKind::Message | LineKind::Notice) {
        let shown = if line.kind == LineKind::Notice {
            format!("-{nick}-")
        } else {
            nick.to_owned()
        };
        // Not by style alone: a mark anyone can see or hear.
        let shown = if theme.accessible {
            format!("!{shown}")
        } else {
            shown
        };
        return (shown, theme.highlight);
    }
    match line.kind {
        LineKind::Message => (nick.to_owned(), Style::default().fg(colours.colour(nick))),
        LineKind::Action => ("*".to_owned(), theme.action),
        LineKind::Notice => (format!("-{nick}-"), theme.notice),
        LineKind::Join => ("-->".to_owned(), theme.join),
        LineKind::Part | LineKind::Quit | LineKind::Kick => ("<--".to_owned(), theme.part),
        LineKind::Error => ("!!".to_owned(), theme.error.add_modifier(Modifier::BOLD)),
        LineKind::Nick | LineKind::Mode | LineKind::Topic | LineKind::Info => {
            ("--".to_owned(), theme.dim)
        }
    }
}

/// Fits `text` into `width` columns, right-aligned, marking truncation with `+`.
fn fit_right(text: &str, width: usize) -> String {
    let w = text.width();
    if w <= width {
        return format!("{}{text}", " ".repeat(width - w));
    }
    let mut out = String::new();
    let mut used = 0;
    for g in text.graphemes(true) {
        if used + g.width() + 1 > width {
            break;
        }
        used += g.width();
        out.push_str(g);
    }
    format!("{}{out}+", " ".repeat(width.saturating_sub(used + 1)))
}

/// How many reacted with each of a line's reactions, as text, for
/// [`pieces`] to borrow.
fn reaction_counts(line: &Line) -> Vec<String> {
    line.reactions
        .iter()
        .map(|r| r.nicks.len().to_string())
        .collect()
}

/// The styled pieces of a line's text: what it replies to, the text, then
/// its reactions with how many reacted (`counts`, from
/// [`reaction_counts`]). Our messages still waiting for the server's echo
/// are dimmed; ones that never made it say why.
fn pieces<'a>(
    line: &'a Line,
    colours: &theme::NickColours,
    theme: &Theme,
    counts: &'a [String],
) -> Vec<Piece<'a>> {
    let mut base = theme.line_style(line.kind);
    if matches!(line.delivery, Delivery::Pending { .. }) {
        base = base.patch(theme.pending);
    }
    let mut out = Vec::new();
    if let Some(reply) = &line.reply {
        let dim = theme.dim;
        out.push((if theme.accessible { "re: " } else { "↪ " }, dim));
        match &reply.parent {
            Some((nick, excerpt)) => {
                out.push((nick.as_str(), dim.fg(colours.colour(nick))));
                out.push((": ", dim));
                out.push((excerpt.as_str(), dim.add_modifier(Modifier::ITALIC)));
            }
            None => out.push(("an earlier message", dim.add_modifier(Modifier::ITALIC))),
        }
        out.push(("  ", base));
    }
    if line.kind == LineKind::Action
        && let Some(nick) = &line.sender
    {
        let style = if line.highlight {
            theme.highlight
        } else {
            Style::default().fg(colours.colour(nick))
        };
        out.push((nick.as_str(), style));
        out.push((" ", base));
    }
    out.extend(
        line.text
            .spans()
            .iter()
            .map(|s| (s.text(), base.patch(theme::span_style(s.style())))),
    );
    if theme.accessible && matches!(line.delivery, Delivery::Pending { .. }) {
        out.push((" (sending)", base));
    }
    if let Delivery::Failed { reason } = &line.delivery {
        let red = theme.error;
        out.extend([(" [not sent: ", red), (reason.as_str(), red), ("]", red)]);
    }
    let mut out = underline_urls(out);
    for (reaction, count) in line.reactions.iter().zip(counts) {
        let style = Style::default().add_modifier(Modifier::REVERSED);
        out.extend([
            ("  ", Style::default()),
            (reaction.text.as_str(), style),
            (" ", style),
            (count.as_str(), style),
        ]);
    }
    out
}

/// Splits pieces so URLs in them are underlined.
fn underline_urls(pieces: Vec<Piece<'_>>) -> Vec<Piece<'_>> {
    let mut out = Vec::with_capacity(pieces.len());
    for (text, style) in pieces {
        let mut at = 0;
        for range in urls::find(text) {
            if range.start > at {
                out.push((&text[at..range.start], style));
            }
            out.push((
                &text[range.clone()],
                style.add_modifier(Modifier::UNDERLINED),
            ));
            at = range.end;
        }
        if at < text.len() {
            out.push((&text[at..], style));
        }
    }
    out
}

/// Word-wraps pieces into rows of at most `width` columns. Never splits a
/// grapheme; breaks at the last space when it can, mid-word when it must.
pub fn wrap<'a>(pieces: &[Piece<'a>], width: usize) -> Vec<Vec<Piece<'a>>> {
    let width = width.max(1);
    let mut rows: Vec<Vec<Piece<'a>>> = Vec::new();
    let mut row: Vec<Piece<'a>> = Vec::new();
    let mut used = 0;
    // Index into `row` of the last space, for word breaks.
    let mut last_space: Option<usize> = None;
    for &(text, style) in pieces {
        for g in text.graphemes(true) {
            let w = g.width();
            if used + w > width {
                if g == " " {
                    rows.push(std::mem::take(&mut row));
                    used = 0;
                    last_space = None;
                    continue;
                }
                match last_space {
                    // A space at the very start would leave an empty row.
                    Some(space) if space > 0 => {
                        let tail = row.split_off(space + 1);
                        row.pop();
                        rows.push(std::mem::replace(&mut row, tail));
                        used = row.iter().map(|(t, _)| t.width()).sum();
                    }
                    _ => {
                        rows.push(std::mem::take(&mut row));
                        used = 0;
                    }
                }
                last_space = None;
            }
            if g == " " {
                last_space = Some(row.len());
            }
            row.push((g, style));
            used += w;
        }
    }
    if !row.is_empty() || rows.is_empty() {
        rows.push(row);
    }
    rows
}

fn draw_pieces(cells: &mut Cells, x: u16, y: u16, max_x: u16, pieces: &[Piece<'_>]) {
    let mut x = x;
    for &(text, style) in pieces {
        if x >= max_x {
            return;
        }
        let (next, _) = cells.set_stringn(x, y, text, usize::from(max_x - x), style);
        x = next;
    }
}

impl Widget for Scrollback<'_> {
    fn render(self, area: Rect, cells: &mut Cells) {
        if area.is_empty() {
            return;
        }
        let lines = self.buffer.lines();
        let Geometry {
            time: time_width,
            nick: nick_width,
            text: text_width,
        } = self.geometry(area);
        let full = usize::from(area.width);
        let sep = separator(self.theme);
        let gutter_width = time_width + 1 + nick_width + sep.width();
        let text_x = area.x + u16::try_from(gutter_width.min(full)).unwrap_or(area.width);
        let max_x = area.right();
        // A separator only between read and unread lines: with nothing
        // unread, or nothing read above, it would say nothing.
        let held = u64::try_from(lines.len()).unwrap_or(u64::MAX);
        let oldest = (self.buffer.total + 1).saturating_sub(held);
        let first_unread = self
            .marker
            .filter(|&m| m < self.buffer.total && m >= oldest)
            .map(|m| m + 1);

        let mut y = area.bottom();
        for (back, line) in lines.iter().rev().enumerate().skip(self.offset) {
            if y <= area.top() {
                break;
            }
            let seq = self.buffer.total - u64::try_from(back).unwrap_or(0);
            let counts = reaction_counts(line);
            let pieces = pieces(line, self.colours, self.theme, &counts);
            let rows = wrap(&pieces, text_width);
            for (index, row) in rows.iter().enumerate().rev() {
                if y <= area.top() {
                    break;
                }
                y -= 1;
                draw_pieces(cells, text_x, y, max_x, row);
                if index > 0 {
                    // Continuation rows keep the separator, so the text column
                    // reads as one block.
                    let sep_x = text_x.saturating_sub(u16::try_from(sep.width()).unwrap_or(0));
                    draw_pieces(cells, sep_x, y, max_x, &[(sep, self.theme.dim)]);
                } else if line.continued {
                    // The rest of a multi-line message: one block, under
                    // the first line's time and nick.
                    let blank = " ".repeat(time_width + 1 + nick_width);
                    let gutter_pieces = [(blank.as_str(), Style::default()), (sep, self.theme.dim)];
                    draw_pieces(cells, area.x, y, max_x, &gutter_pieces);
                } else {
                    let time = line
                        .time
                        .to_zoned(self.tz.clone())
                        .strftime(self.time_format)
                        .to_string();
                    let (nick, nick_style) = gutter(line, self.colours, self.theme);
                    let nick = fit_right(&nick, nick_width);
                    let gutter_pieces = [
                        (time.as_str(), self.theme.dim),
                        (" ", Style::default()),
                        (nick.as_str(), nick_style),
                        (sep, self.theme.dim),
                    ];
                    draw_pieces(cells, area.x, y, max_x, &gutter_pieces);
                }
            }
            if Some(seq) == first_unread && y > area.top() {
                y -= 1;
                let label = " unread ";
                let rule_char = if self.theme.accessible { "-" } else { "─" };
                let side = full.saturating_sub(label.width()) / 2;
                let rule = format!(
                    "{}{label}{}",
                    rule_char.repeat(side),
                    rule_char.repeat(full.saturating_sub(side + label.width()))
                );
                draw_pieces(
                    cells,
                    area.x,
                    y,
                    max_x,
                    &[(rule.as_str(), self.theme.unread)],
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(rows: &[Vec<Piece<'_>>]) -> Vec<String> {
        rows.iter()
            .map(|r| r.iter().map(|(t, _)| *t).collect())
            .collect()
    }

    #[test]
    fn wraps_at_words_and_graphemes() {
        let s = Style::default();
        assert_eq!(
            text(&wrap(&[("hello brave new world", s)], 11)),
            ["hello brave", "new world"]
        );
        assert_eq!(text(&wrap(&[("abcdefghij", s)], 4)), ["abcd", "efgh", "ij"]);
        assert_eq!(text(&wrap(&[("漢字かな", s)], 5)), ["漢字", "かな"]);
        assert_eq!(text(&wrap(&[("ab ", s), ("cd", s)], 3)), ["ab", "cd"]);
        assert_eq!(text(&wrap(&[("", s)], 3)), [""]);
        assert_eq!(
            text(&wrap(&[("a e\u{301}e\u{301}e\u{301}", s)], 2)),
            ["a", "e\u{301}e\u{301}", "e\u{301}"]
        );
    }

    #[test]
    fn nick_column() {
        assert_eq!(fit_right("bob", 5), "  bob");
        assert_eq!(fit_right("averylongnick", 6), "avery+");
        assert_eq!(
            fit_right("漢字漢字", 5),
            "漢字+",
            "already five columns wide"
        );
    }
}
