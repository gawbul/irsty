//! mIRC formatting and sanitisation of untrusted text (ADR 0005).
//!
//! [`StyledText`] is the only type the UI renders, and every way of building one
//! sanitises its input: nothing from the network can reach the terminal as a
//! control character or escape sequence.

/// A colour from a formatting code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Color {
    /// mIRC palette index, `0..=98`.
    Index(u8),
    /// 24-bit colour from the `\x04` hex code.
    Rgb(u8, u8, u8),
}

/// Text attributes for a span.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors the independent mIRC toggles"
)]
pub struct Style {
    /// Bold (`\x02`).
    pub bold: bool,
    /// Italics (`\x1d`).
    pub italic: bool,
    /// Underline (`\x1f`).
    pub underline: bool,
    /// Strikethrough (`\x1e`).
    pub strikethrough: bool,
    /// Monospace (`\x11`).
    pub monospace: bool,
    /// Reverse video (`\x16`).
    pub reverse: bool,
    /// Foreground colour.
    pub fg: Option<Color>,
    /// Background colour.
    pub bg: Option<Color>,
}

/// A run of text with one style. The text is always sanitised.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    text: String,
    style: Style,
}

impl Span {
    /// The sanitised text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The style.
    pub fn style(&self) -> Style {
        self.style
    }
}

/// Sanitised, styled text: the only text type the UI renders.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StyledText {
    spans: Vec<Span>,
}

const BOLD: char = '\x02';
const COLOR: char = '\x03';
const HEX_COLOR: char = '\x04';
const RESET: char = '\x0f';
const MONOSPACE: char = '\x11';
const REVERSE: char = '\x16';
const ITALIC: char = '\x1d';
const STRIKETHROUGH: char = '\x1e';
const UNDERLINE: char = '\x1f';

/// Directional formatting characters that open a scope, and the kind of
/// character that closes it.
fn bidi_open(c: char) -> Option<BidiScope> {
    match c {
        '\u{202A}'..='\u{202E}' if c != '\u{202C}' => Some(BidiScope::Embedding),
        '\u{2066}'..='\u{2068}' => Some(BidiScope::Isolate),
        _ => None,
    }
}

const PDF: char = '\u{202C}';
const PDI: char = '\u{2069}';

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BidiScope {
    Embedding,
    Isolate,
}

/// Characters that are invisible or change direction, which have no place in
/// identifiers and can be used to impersonate someone.
fn is_invisible_format(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{2069}'
            | '\u{FEFF}'
    )
}

/// Makes one non-formatting character safe for a terminal, writing the result to
/// `out`. Returns `false` if the character was not a control character.
fn neutralise_control(c: char, out: &mut String) -> bool {
    match c {
        // Tabs have no fixed width in a cell grid, and line and paragraph
        // separators can act as newlines in some terminals.
        '\t' | '\u{2028}' | '\u{2029}' => out.push(' '),
        // C0 controls become their visible Control Pictures (U+2400 block).
        '\0'..='\x1f' => {
            out.push(char::from_u32(0x2400 + u32::from(c)).unwrap_or(char::REPLACEMENT_CHARACTER));
        }
        '\x7f' => out.push('\u{2421}'),
        '\u{80}'..='\u{9f}' => out.push(char::REPLACEMENT_CHARACTER),
        _ => return false,
    }
    true
}

/// Reports whether `c` can be copied to the output as it is: printable ASCII, or
/// anything above the C1 range that is not a separator or bidi control. This is
/// the fast path for almost all text.
fn is_plain(c: char) -> bool {
    matches!(c, ' '..='~')
        || (c > '\u{9f}'
            && !matches!(c, '\u{2028}' | '\u{2029}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'))
}

/// Tracks bidi scopes in a line so that every opener is closed by the end and
/// stray closers are dropped. Nothing can leak into the next line.
#[derive(Debug, Default)]
struct BidiBalancer {
    stack: Vec<BidiScope>,
}

impl BidiBalancer {
    /// Handles a bidi control. Returns `true` if `c` was one.
    fn push(&mut self, c: char, out: &mut String) -> bool {
        if let Some(scope) = bidi_open(c) {
            // UAX #9 caps nesting at 125; refuse to go deeper.
            if self.stack.len() < 125 {
                self.stack.push(scope);
                out.push(c);
            }
            return true;
        }
        match c {
            PDF => {
                if self.stack.last() == Some(&BidiScope::Embedding) {
                    self.stack.pop();
                    out.push(c);
                }
                true
            }
            PDI => {
                if let Some(pos) = self.stack.iter().rposition(|&s| s == BidiScope::Isolate) {
                    // Closing an isolate closes any embeddings opened inside it.
                    for _ in pos + 1..self.stack.len() {
                        out.push(PDF);
                    }
                    self.stack.truncate(pos);
                    out.push(c);
                }
                true
            }
            _ => false,
        }
    }

    fn close_all(&mut self, out: &mut String) {
        while let Some(scope) = self.stack.pop() {
            out.push(match scope {
                BidiScope::Embedding => PDF,
                BidiScope::Isolate => PDI,
            });
        }
    }
}

/// Reads up to two decimal digits.
fn take_digits(s: &str) -> (Option<u8>, &str) {
    let len = s.bytes().take(2).take_while(u8::is_ascii_digit).count();
    let (digits, rest) = s.split_at(len);
    (digits.parse().ok(), rest)
}

/// Reads exactly six hex digits.
fn take_hex(s: &str) -> (Option<Color>, &str) {
    let Some(hex) = s
        .get(..6)
        .filter(|h| h.bytes().all(|b| b.is_ascii_hexdigit()))
    else {
        return (None, s);
    };
    let channel = |i: usize| {
        hex.get(i..i + 2)
            .and_then(|c| u8::from_str_radix(c, 16).ok())
            .unwrap_or(0)
    };
    (
        Some(Color::Rgb(channel(0), channel(2), channel(4))),
        s.get(6..).unwrap_or(""),
    )
}

fn index_color(n: u8) -> Option<Color> {
    // 99 means "default colour".
    (n < 99).then_some(Color::Index(n))
}

/// Parses an mIRC colour code body after `\x03`: `[fg[,bg]]`. The comma is only
/// consumed when digits follow it.
fn parse_color(s: &str, style: &mut Style) -> usize {
    let (fg, rest) = take_digits(s);
    let Some(fg) = fg else {
        style.fg = None;
        style.bg = None;
        return 0;
    };
    style.fg = index_color(fg);
    let mut consumed = s.len() - rest.len();
    if let Some(after_comma) = rest.strip_prefix(',')
        && let (Some(bg), after_bg) = take_digits(after_comma)
    {
        style.bg = index_color(bg);
        consumed = s.len() - after_bg.len();
    }
    consumed
}

/// Parses a hex colour code body after `\x04`: `[RRGGBB[,RRGGBB]]`.
fn parse_hex_color(s: &str, style: &mut Style) -> usize {
    let (fg, rest) = take_hex(s);
    let Some(fg) = fg else {
        style.fg = None;
        style.bg = None;
        return 0;
    };
    style.fg = Some(fg);
    let mut consumed = s.len() - rest.len();
    if let Some(after_comma) = rest.strip_prefix(',')
        && let (Some(bg), after_bg) = take_hex(after_comma)
    {
        style.bg = Some(bg);
        consumed = s.len() - after_bg.len();
    }
    consumed
}

impl StyledText {
    /// Parses mIRC formatting in untrusted text and sanitises everything else.
    pub fn parse(input: &str) -> Self {
        let mut builder = Builder::default();
        let mut style = Style::default();
        let mut rest = input;
        while let Some(c) = rest.chars().next() {
            rest = rest.get(c.len_utf8()..).unwrap_or("");
            let toggled = match c {
                BOLD => Some(&mut style.bold),
                ITALIC => Some(&mut style.italic),
                UNDERLINE => Some(&mut style.underline),
                STRIKETHROUGH => Some(&mut style.strikethrough),
                MONOSPACE => Some(&mut style.monospace),
                REVERSE => Some(&mut style.reverse),
                _ => None,
            };
            if let Some(flag) = toggled {
                *flag = !*flag;
                continue;
            }
            match c {
                COLOR => {
                    let used = parse_color(rest, &mut style);
                    rest = rest.get(used..).unwrap_or("");
                }
                HEX_COLOR => {
                    let used = parse_hex_color(rest, &mut style);
                    rest = rest.get(used..).unwrap_or("");
                }
                RESET => style = Style::default(),
                c => builder.push_char(c, style),
            }
        }
        builder.finish()
    }

    /// Sanitises untrusted text and ignores formatting: codes are removed.
    pub fn plain(input: &str) -> Self {
        let mut text = Self::parse(input);
        for span in &mut text.spans {
            span.style = Style::default();
        }
        text.merge_adjacent();
        text
    }

    /// Builds text from sanitised pieces with explicit styles, for client-generated
    /// lines that include remote data.
    pub fn builder() -> Builder {
        Builder::default()
    }

    /// The spans, in order.
    pub fn spans(&self) -> &[Span] {
        &self.spans
    }

    /// The sanitised text without styles.
    pub fn to_plain(&self) -> String {
        self.spans.iter().map(|s| s.text.as_str()).collect()
    }

    /// Reports whether there is no text.
    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    fn merge_adjacent(&mut self) {
        let mut merged: Vec<Span> = Vec::with_capacity(self.spans.len());
        for span in self.spans.drain(..) {
            match merged.last_mut() {
                Some(last) if last.style == span.style => last.text.push_str(&span.text),
                _ => merged.push(span),
            }
        }
        self.spans = merged;
    }
}

/// Builds [`StyledText`], sanitising every character pushed.
#[derive(Debug, Default)]
pub struct Builder {
    spans: Vec<Span>,
    bidi: BidiBalancer,
    scratch: String,
}

impl Builder {
    /// Appends sanitised `text` in `style`. Formatting codes in `text` are
    /// neutralised, not interpreted.
    #[must_use]
    pub fn push(mut self, text: &str, style: Style) -> Self {
        for c in text.chars() {
            self.push_char(c, style);
        }
        self
    }

    fn push_char(&mut self, c: char, style: Style) {
        if is_plain(c) {
            self.span_mut(style).push(c);
            return;
        }
        let mut buf = std::mem::take(&mut self.scratch);
        buf.clear();
        if !neutralise_control(c, &mut buf) && !self.bidi.push(c, &mut buf) {
            buf.push(c);
        }
        self.append(&buf, style);
        self.scratch = buf;
    }

    /// The text of the last span, starting a new span if the style differs.
    fn span_mut(&mut self, style: Style) -> &mut String {
        if self.spans.last().is_none_or(|last| last.style != style) {
            self.spans.push(Span {
                text: String::new(),
                style,
            });
        }
        match self.spans.last_mut() {
            Some(last) => &mut last.text,
            // Unreachable: a span was pushed above if there was none.
            None => &mut self.scratch,
        }
    }

    fn append(&mut self, text: &str, style: Style) {
        if text.is_empty() {
            return;
        }
        match self.spans.last_mut() {
            Some(last) if last.style == style => last.text.push_str(text),
            _ => self.spans.push(Span {
                text: text.to_owned(),
                style,
            }),
        }
    }

    /// Closes any open bidi scopes and returns the text.
    pub fn finish(mut self) -> StyledText {
        let mut closers = String::new();
        self.bidi.close_all(&mut closers);
        let style = self.spans.last().map(|s| s.style).unwrap_or_default();
        self.append(&closers, style);
        StyledText { spans: self.spans }
    }
}

/// Sanitises an identifier (nick, channel name, hostname): removes formatting
/// codes, invisible characters and all bidi controls, and neutralises controls.
pub fn sanitise_identifier(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        if is_invisible_format(c)
            || matches!(
                c,
                BOLD | COLOR
                    | HEX_COLOR
                    | RESET
                    | MONOSPACE
                    | REVERSE
                    | ITALIC
                    | STRIKETHROUGH
                    | UNDERLINE
            )
        {
            continue;
        }
        if !neutralise_control(c, &mut out) {
            out.push(c);
        }
    }
    out
}

/// Removes formatting codes and their colour arguments, leaving other characters
/// untouched. Use for matching (highlights, search), never for display.
pub fn strip_formatting(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut scratch = Style::default();
    let mut rest = input;
    while let Some(c) = rest.chars().next() {
        rest = rest.get(c.len_utf8()..).unwrap_or("");
        match c {
            COLOR => rest = rest.get(parse_color(rest, &mut scratch)..).unwrap_or(""),
            HEX_COLOR => {
                rest = rest
                    .get(parse_hex_color(rest, &mut scratch)..)
                    .unwrap_or("");
            }
            BOLD | RESET | MONOSPACE | REVERSE | ITALIC | STRIKETHROUGH | UNDERLINE => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(text: &StyledText) -> Vec<(&str, Style)> {
        text.spans().iter().map(|s| (s.text(), s.style())).collect()
    }

    fn bold() -> Style {
        Style {
            bold: true,
            ..Style::default()
        }
    }

    #[test]
    fn toggles_and_reset() {
        let t = StyledText::parse("a\x02b\x02c\x1d\x1fd\x0fe");
        let both = Style {
            italic: true,
            underline: true,
            ..Style::default()
        };
        assert_eq!(
            spans(&t),
            [
                ("a", Style::default()),
                ("b", bold()),
                ("c", Style::default()),
                ("d", both),
                ("e", Style::default())
            ]
        );
        let all = StyledText::parse("\x11\x16\x1ex");
        let s = all.spans()[0].style();
        assert!(s.monospace && s.reverse && s.strikethrough);
    }

    #[test]
    fn colours() {
        let t = StyledText::parse("\x034red\x034,12on blue\x03plain");
        assert_eq!(t.spans()[0].style().fg, Some(Color::Index(4)));
        assert_eq!(t.spans()[1].style().bg, Some(Color::Index(12)));
        assert_eq!(t.spans()[2].style(), Style::default());
        assert_eq!(t.to_plain(), "redon blueplain");

        // Leading zeros, three digits (only two consumed), comma without digits.
        let t = StyledText::parse("\x03041\x035,x");
        assert_eq!(spans(&t)[0].0, "1");
        assert_eq!(t.spans()[0].style().fg, Some(Color::Index(4)));
        assert_eq!(spans(&t)[1].0, ",x");

        // 99 is the default colour.
        assert_eq!(
            StyledText::parse("\x0399,99x").spans()[0].style(),
            Style::default()
        );
    }

    #[test]
    fn hex_colours() {
        let t = StyledText::parse("\x04FF8000a\x04000000,FFFFFFb\x04c\x04ABCx");
        assert_eq!(t.spans()[0].style().fg, Some(Color::Rgb(255, 128, 0)));
        assert_eq!(t.spans()[1].style().bg, Some(Color::Rgb(255, 255, 255)));
        assert_eq!(t.to_plain(), "abcABCx");
    }

    #[test]
    fn neutralises_escape_sequences() {
        let attacks = [
            "\x1b[2J\x1b[H",                                         // CSI clear screen
            "\x1b]0;pwned\x07",                                      // OSC 0 window title
            "\x1b]2;pwned\x1b\\",                                    // OSC 2 with ST
            "\x1b]52;c;cm0gLXJmIH4=\x07",                            // OSC 52 clipboard write
            "\x1b]8;;https://evil.example\x1b\\click\x1b]8;;\x1b\\", // OSC 8 hyperlink
            "\x1bP+q544e\x1b\\",                                     // DCS
            "\u{9b}31m",                                             // 8-bit CSI
            "\u{9d}0;title\u{9c}",                                   // 8-bit OSC and ST
            "a\rb\nc\x08d\x7f",                                      // CR, LF, backspace, DEL
            "\x1b[",                                                 // truncated
        ];
        for attack in attacks {
            for text in [StyledText::parse(attack), StyledText::plain(attack)] {
                let plain = text.to_plain();
                assert!(
                    !plain.chars().any(char::is_control),
                    "control survived in {plain:?} from {attack:?}"
                );
            }
        }
        assert_eq!(StyledText::parse("\x1b[31m").to_plain(), "\u{241b}[31m");
        assert_eq!(StyledText::parse("a\tb").to_plain(), "a b");
        assert_eq!(StyledText::parse("a\u{2028}b").to_plain(), "a b");
    }

    #[test]
    fn balances_bidi() {
        // An unclosed RLO is closed at the end of the line.
        assert_eq!(
            StyledText::parse("a\u{202E}b").to_plain(),
            "a\u{202E}b\u{202C}"
        );
        // Stray closers are dropped.
        assert_eq!(StyledText::parse("a\u{202C}\u{2069}b").to_plain(), "ab");
        // A PDI closes embeddings opened inside its isolate.
        assert_eq!(
            StyledText::parse("\u{2067}\u{202B}x\u{2069}").to_plain(),
            "\u{2067}\u{202B}x\u{202C}\u{2069}"
        );
        // Balanced text is untouched.
        assert_eq!(
            StyledText::parse("\u{2068}שלום\u{2069}").to_plain(),
            "\u{2068}שלום\u{2069}"
        );
        // Nesting is capped.
        let deep = "\u{202A}".repeat(200);
        let out = StyledText::parse(&deep).to_plain();
        assert_eq!(out.chars().filter(|&c| c == '\u{202A}').count(), 125);
        assert_eq!(out.chars().filter(|&c| c == PDF).count(), 125);
    }

    #[test]
    fn plain_drops_styles() {
        let t = StyledText::plain("\x02a\x02\x034b");
        assert_eq!(spans(&t), [("ab", Style::default())]);
        assert!(StyledText::plain("").is_empty());
    }

    #[test]
    fn builder_sanitises() {
        let t = StyledText::builder()
            .push("Joined ", Style::default())
            .push("evil\x1b]0;x\x07nick\x02", bold())
            .finish();
        assert_eq!(t.to_plain(), "Joined evil\u{241b}]0;x\u{2407}nick\u{2402}");
        assert_eq!(t.spans().len(), 2);
    }

    #[test]
    fn identifiers() {
        assert_eq!(sanitise_identifier("ad\u{202E}min\u{200B}"), "admin");
        assert_eq!(sanitise_identifier("\x02nick\x03"), "nick");
        assert_eq!(sanitise_identifier("a\x1bb"), "a\u{241b}b");
        assert_eq!(sanitise_identifier("n\x035w"), "n5w");
    }

    #[test]
    fn strip() {
        assert_eq!(
            strip_formatting("\x02hi\x02 \x034,5there\x04FFFFFF!\x0f"),
            "hi there!"
        );
        assert_eq!(strip_formatting("a\x1bb"), "a\x1bb");
    }
}
