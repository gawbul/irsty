//! The input line: grapheme-aware editing, emacs-style keys, a kill buffer and
//! history (ADR 0003).

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// History entries kept.
const HISTORY: usize = 500;

/// An editable line of text. The cursor is a byte offset that always sits on a
/// grapheme boundary.
#[derive(Clone, Debug, Default)]
pub struct InputLine {
    text: String,
    cursor: usize,
    killed: String,
    history: Vec<String>,
    /// Position while browsing history, and the line being edited before.
    browsing: Option<(usize, String)>,
}

impl InputLine {
    /// The current text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The cursor's byte offset.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Reports whether the line is empty.
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Inserts text at the cursor. Line breaks become spaces and NULs are
    /// dropped, so the line can always be sent as one message.
    pub fn insert(&mut self, text: &str) {
        let clean: String = text
            .chars()
            .filter(|&c| c != '\0')
            .map(|c| if matches!(c, '\r' | '\n') { ' ' } else { c })
            .collect();
        self.text.insert_str(self.cursor, &clean);
        self.cursor += clean.len();
        self.snap();
    }

    /// Moves the cursor forward to the next grapheme boundary if an insertion
    /// joined it onto a previous grapheme (a combining mark, say).
    fn snap(&mut self) {
        if !self.boundaries().any(|b| b == self.cursor) {
            self.cursor = self.next_boundary(self.cursor);
        }
    }

    fn boundaries(&self) -> impl Iterator<Item = usize> + '_ {
        self.text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .chain(std::iter::once(self.text.len()))
    }

    fn prev_boundary(&self, from: usize) -> usize {
        self.boundaries()
            .take_while(|&b| b < from)
            .last()
            .unwrap_or(0)
    }

    fn next_boundary(&self, from: usize) -> usize {
        self.boundaries()
            .find(|&b| b > from)
            .unwrap_or(self.text.len())
    }

    fn word_start(&self, from: usize) -> usize {
        let before = self.text.get(..from).unwrap_or_default();
        let trimmed = before.trim_end();
        trimmed.rfind(char::is_whitespace).map_or(0, |i| i + 1)
    }

    fn word_end(&self, from: usize) -> usize {
        let after = self.text.get(from..).unwrap_or_default();
        let skipped = after.len() - after.trim_start().len();
        let rest = after.get(skipped..).unwrap_or_default();
        from + skipped + rest.find(char::is_whitespace).unwrap_or(rest.len())
    }

    fn kill(&mut self, start: usize, end: usize) {
        if start < end {
            self.killed = self.text.drain(start..end).collect();
            self.cursor = start;
        }
    }

    /// Deletes the grapheme before the cursor.
    pub fn backspace(&mut self) {
        let start = self.prev_boundary(self.cursor);
        self.text.drain(start..self.cursor);
        self.cursor = start;
    }

    /// Deletes the grapheme under the cursor.
    pub fn delete(&mut self) {
        let end = self.next_boundary(self.cursor);
        self.text.drain(self.cursor..end);
    }

    /// Moves left one grapheme.
    pub fn left(&mut self) {
        self.cursor = self.prev_boundary(self.cursor);
    }

    /// Moves right one grapheme.
    pub fn right(&mut self) {
        self.cursor = self.next_boundary(self.cursor);
    }

    /// Moves to the start of the previous word.
    pub fn word_left(&mut self) {
        self.cursor = self.word_start(self.cursor);
    }

    /// Moves past the end of the next word.
    pub fn word_right(&mut self) {
        self.cursor = self.word_end(self.cursor);
    }

    /// Moves to the start.
    pub fn home(&mut self) {
        self.cursor = 0;
    }

    /// Moves to the end.
    pub fn end(&mut self) {
        self.cursor = self.text.len();
    }

    /// Cuts to the end of the line (Ctrl+K).
    pub fn kill_to_end(&mut self) {
        self.kill(self.cursor, self.text.len());
    }

    /// Cuts to the start of the line (Ctrl+U).
    pub fn kill_to_start(&mut self) {
        self.kill(0, self.cursor);
    }

    /// Cuts the word before the cursor (Ctrl+W, Alt+Backspace).
    pub fn kill_word_back(&mut self) {
        self.kill(self.word_start(self.cursor), self.cursor);
    }

    /// Cuts the word after the cursor (Alt+D).
    pub fn kill_word_forward(&mut self) {
        self.kill(self.cursor, self.word_end(self.cursor));
    }

    /// Pastes the last cut text (Ctrl+Y).
    pub fn yank(&mut self) {
        let killed = self.killed.clone();
        self.insert(&killed);
    }

    /// Takes the line for sending, recording it in history.
    pub fn submit(&mut self) -> String {
        let line = std::mem::take(&mut self.text);
        self.cursor = 0;
        self.browsing = None;
        if !line.trim().is_empty() && self.history.last() != Some(&line) {
            self.history.push(line.clone());
            if self.history.len() > HISTORY {
                self.history.remove(0);
            }
        }
        line
    }

    /// Recalls the previous history entry (Up).
    pub fn history_prev(&mut self) {
        let index = match &self.browsing {
            Some((0, _)) => return,
            Some((i, _)) => i - 1,
            None if self.history.is_empty() => return,
            None => {
                self.browsing = Some((self.history.len(), self.text.clone()));
                self.history.len() - 1
            }
        };
        if let Some((current, _)) = &mut self.browsing {
            *current = index;
        }
        self.show(self.history.get(index).cloned().unwrap_or_default());
    }

    /// Moves forward through history, back to the draft (Down).
    pub fn history_next(&mut self) {
        let Some((index, draft)) = self.browsing.clone() else {
            return;
        };
        if index + 1 >= self.history.len() {
            self.browsing = None;
            self.show(draft);
        } else {
            self.browsing = Some((index + 1, draft));
            self.show(self.history.get(index + 1).cloned().unwrap_or_default());
        }
    }

    fn show(&mut self, text: String) {
        self.text = text;
        self.cursor = self.text.len();
    }

    /// The slice of text to draw in `width` columns, and the cursor's column
    /// within it. Scrolls horizontally to keep the cursor visible.
    pub fn view(&self, width: usize) -> (&str, usize) {
        if width == 0 {
            return ("", 0);
        }
        let before = self.text.get(..self.cursor).unwrap_or_default();
        let cursor_col = before.width();
        // Leave room for the cursor itself at the right edge.
        let mut start = 0;
        if cursor_col >= width {
            let mut skipped = 0;
            for (i, g) in self.text.grapheme_indices(true) {
                if cursor_col - skipped < width {
                    break;
                }
                skipped += g.width();
                start = i + g.len();
            }
        }
        let skipped_width = self.text.get(..start).unwrap_or_default().width();
        let mut end = start;
        let mut used = 0;
        for (i, g) in self
            .text
            .get(start..)
            .unwrap_or_default()
            .grapheme_indices(true)
        {
            if used + g.width() > width {
                break;
            }
            used += g.width();
            end = start + i + g.len();
        }
        (
            self.text.get(start..end).unwrap_or_default(),
            cursor_col - skipped_width,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str) -> InputLine {
        let mut input = InputLine::default();
        input.insert(text);
        input
    }

    #[test]
    fn edits_by_grapheme() {
        let mut input = line("ae\u{301}👍🏽");
        input.backspace();
        assert_eq!(input.text(), "ae\u{301}");
        input.left();
        assert_eq!(input.cursor(), 1);
        input.delete();
        assert_eq!(input.text(), "a");
        input.home();
        input.insert("x");
        input.end();
        input.insert("e");
        // A combining mark typed after "e" joins it; the cursor stays after both.
        input.insert("\u{301}");
        assert_eq!(input.text(), "xae\u{301}");
        assert_eq!(input.cursor(), input.text().len());
        input.right();
        assert_eq!(input.cursor(), input.text().len());
    }

    #[test]
    fn paste_flattens_lines() {
        assert_eq!(line("one\r\ntwo\nthree\0").text(), "one  two three");
    }

    #[test]
    fn words_and_kill_ring() {
        let mut input = line("hello brave  new world");
        input.word_left();
        assert_eq!(input.cursor(), 17);
        input.word_left();
        input.word_left();
        assert_eq!(input.cursor(), 6);
        input.word_right();
        assert_eq!(input.cursor(), 11);
        input.kill_word_forward();
        assert_eq!(input.text(), "hello brave world");
        input.yank();
        assert_eq!(input.text(), "hello brave  new world");
        input.end();
        input.kill_word_back();
        assert_eq!(input.text(), "hello brave  new ");
        input.home();
        input.kill_to_end();
        assert!(input.is_empty());
        input.yank();
        input.end();
        input.left();
        input.kill_to_start();
        assert_eq!(input.text(), " ");
    }

    #[test]
    fn history() {
        let mut input = InputLine::default();
        input.history_prev();
        input.history_next();
        for l in ["one", "two", "two", "  "] {
            input.insert(l);
            assert_eq!(input.submit(), l);
        }
        input.insert("draft");
        input.history_prev();
        assert_eq!(input.text(), "two");
        input.history_prev();
        assert_eq!(input.text(), "one");
        input.history_prev();
        assert_eq!(input.text(), "one");
        input.history_next();
        assert_eq!(input.text(), "two");
        input.history_next();
        assert_eq!(input.text(), "draft");
        input.history_next();
        assert_eq!(input.text(), "draft");
    }

    #[test]
    fn history_is_capped() {
        let mut input = InputLine::default();
        for i in 0..(HISTORY + 10) {
            input.insert(&i.to_string());
            input.submit();
        }
        assert_eq!(input.history.len(), HISTORY);
        assert_eq!(input.history.first().map(String::as_str), Some("10"));
    }

    #[test]
    fn view_scrolls_to_the_cursor() {
        let input = line("0123456789");
        assert_eq!(input.view(20), ("0123456789", 10));
        let (text, col) = input.view(5);
        assert_eq!((text, col), ("6789", 4));
        let mut input = line("漢字かな");
        assert_eq!(input.view(5), ("かな", 4));
        input.home();
        assert_eq!(input.view(5), ("漢字", 0));
        assert_eq!(input.view(0), ("", 0));
    }
}
