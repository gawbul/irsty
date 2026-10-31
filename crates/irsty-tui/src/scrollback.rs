//! The scrollback widget (ADR 0003).
//!
//! Lines are laid out from the bottom up, so only the lines on screen are
//! wrapped: drawing cost depends on the window size, not the buffer size.
//! Each line has a timestamp, a right-aligned nick column and a separator;
//! wrapped text hangs under the text column.

use irsty_core::state::{Buffer, Line, LineKind};
use jiff::tz::TimeZone;
use ratatui::buffer::Buffer as Cells;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::Widget;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::theme;

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
}

/// One piece of text with a style.
type Piece<'a> = (&'a str, Style);

/// The nick column for a line: text and style.
fn gutter(line: &Line) -> (String, Style) {
    let nick = line.sender.as_deref().unwrap_or("");
    match line.kind {
        LineKind::Message => (
            nick.to_owned(),
            Style::default().fg(theme::nick_color(nick)),
        ),
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

/// The styled pieces of a line's text.
fn pieces(line: &Line) -> Vec<Piece<'_>> {
    let base = theme::line_style(line.kind);
    let mut out = Vec::new();
    if line.kind == LineKind::Action
        && let Some(nick) = &line.sender
    {
        out.push((nick.as_str(), Style::default().fg(theme::nick_color(nick))));
        out.push((" ", base));
    }
    out.extend(
        line.text
            .spans()
            .iter()
            .map(|s| (s.text(), base.patch(theme::span_style(s.style())))),
    );
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
            let pieces = pieces(line);
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
                } else {
                    let time = line
                        .time
                        .to_zoned(self.tz.clone())
                        .strftime(self.time_format)
                        .to_string();
                    let (nick, nick_style) = gutter(line);
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
