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
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::Widget;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::{theme, urls};

/// Separator between the nick column and the text.
const SEPARATOR: &str = " │ ";

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
}

/// One piece of text with a style.
type Piece<'a> = (&'a str, Style);

/// The nick column for a line: text and style.
fn gutter(line: &Line, colours: &theme::NickColours) -> (String, Style) {
    let nick = line.sender.as_deref().unwrap_or("");
    if line.highlight && matches!(line.kind, LineKind::Message | LineKind::Notice) {
        let shown = if line.kind == LineKind::Notice {
            format!("-{nick}-")
        } else {
            nick.to_owned()
        };
        return (shown, theme::highlight());
    }
    match line.kind {
        LineKind::Message => (nick.to_owned(), Style::default().fg(colours.colour(nick))),
        LineKind::Action => (
            "*".to_owned(),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        LineKind::Notice => (
            format!("-{nick}-"),
            Style::default().fg(Color::LightMagenta),
        ),
        LineKind::Join => ("-->".to_owned(), Style::default().fg(Color::LightGreen)),
        LineKind::Part | LineKind::Quit | LineKind::Kick => {
            ("<--".to_owned(), Style::default().fg(Color::LightRed))
        }
        LineKind::Error => (
            "!!".to_owned(),
            Style::default()
                .fg(Color::LightRed)
                .add_modifier(Modifier::BOLD),
        ),
        LineKind::Nick | LineKind::Mode | LineKind::Topic | LineKind::Info => {
            ("--".to_owned(), theme::dim())
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
    counts: &'a [String],
) -> Vec<Piece<'a>> {
    let mut base = theme::line_style(line.kind);
    if matches!(line.delivery, Delivery::Pending { .. }) {
        base = base.patch(theme::dim());
    }
    let mut out = Vec::new();
    if let Some(reply) = &line.reply {
        let dim = theme::dim();
        out.push(("↪ ", dim));
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
            theme::highlight()
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
    if let Delivery::Failed { reason } = &line.delivery {
        let red = Style::default().fg(Color::LightRed);
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
        let sample_time = lines
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
        let text_width = full.saturating_sub(gutter_width).max(1);
        let text_x = area.x + u16::try_from(gutter_width.min(full)).unwrap_or(area.width);
        let max_x = area.right();

        let mut y = area.bottom();
        for line in lines.iter().rev().skip(self.offset) {
            if y <= area.top() {
                break;
            }
            let counts = reaction_counts(line);
            let pieces = pieces(line, self.colours, &counts);
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
                    let sep_x =
                        text_x.saturating_sub(u16::try_from(SEPARATOR.width()).unwrap_or(0));
                    draw_pieces(cells, sep_x, y, max_x, &[(SEPARATOR, theme::dim())]);
                } else if line.continued {
                    // The rest of a multi-line message: one block, under
                    // the first line's time and nick.
                    let blank = " ".repeat(time_width + 1 + nick_width);
                    let gutter_pieces = [
                        (blank.as_str(), Style::default()),
                        (SEPARATOR, theme::dim()),
                    ];
                    draw_pieces(cells, area.x, y, max_x, &gutter_pieces);
                } else {
                    let time = line
                        .time
                        .to_zoned(self.tz.clone())
                        .strftime(self.time_format)
                        .to_string();
                    let (nick, nick_style) = gutter(line, self.colours);
                    let nick = fit_right(&nick, nick_width);
                    let gutter_pieces = [
                        (time.as_str(), theme::dim()),
                        (" ", Style::default()),
                        (nick.as_str(), nick_style),
                        (SEPARATOR, theme::dim()),
                    ];
                    draw_pieces(cells, area.x, y, max_x, &gutter_pieces);
                }
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
