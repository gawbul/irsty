//! Colours and styles: the theme (`[theme]`), and fitting colours to what the
//! terminal can show.
//!
//! Styles in the config are written as words: modifiers (`bold`, `dim`,
//! `italic`, `underline`, `reversed`, `crossed`), a colour, and `on` another
//! colour for the background, such as `"bold white on blue"`. A colour is a
//! name (`red`, `light_blue`, `dark_gray`, `default`), `#rrggbb`, or
//! `colour0` to `colour255` from the 256-colour palette.

use irsty_core::config::{ColourMode, ThemeConfig};
use irsty_core::state::{Activity, LineKind};
use irsty_proto::format::{self, Color as IrcColor};
use ratatui::style::{Color, Modifier, Style};

/// mIRC colours 16 to 98 (modern.ircdocs.horse/formatting).
const EXTENDED: [u32; 83] = [
    0x47_0000, 0x47_2100, 0x47_4700, 0x32_4700, 0x00_4700, 0x00_472c, 0x00_4747, 0x00_2747,
    0x00_0047, 0x2e_0047, 0x47_0047, 0x47_002a, 0x74_0000, 0x74_3a00, 0x74_7400, 0x51_7400,
    0x00_7400, 0x00_7449, 0x00_7474, 0x00_4074, 0x00_0074, 0x4b_0074, 0x74_0074, 0x74_0045,
    0xb5_0000, 0xb5_6300, 0xb5_b500, 0x7d_b500, 0x00_b500, 0x00_b571, 0x00_b5b5, 0x00_63b5,
    0x00_00b5, 0x75_00b5, 0xb5_00b5, 0xb5_006b, 0xff_0000, 0xff_8c00, 0xff_ff00, 0xb2_ff00,
    0x00_ff00, 0x00_ffa0, 0x00_ffff, 0x00_8cff, 0x00_00ff, 0xa5_00ff, 0xff_00ff, 0xff_0098,
    0xff_5959, 0xff_b459, 0xff_ff71, 0xcf_ff60, 0x6f_ff6f, 0x65_ffc9, 0x6d_ffff, 0x59_b4ff,
    0x59_59ff, 0xc4_59ff, 0xff_66ff, 0xff_59bc, 0xff_9c9c, 0xff_d39c, 0xff_ff9c, 0xe2_ff9c,
    0x9c_ff9c, 0x9c_ffdb, 0x9c_ffff, 0x9c_d3ff, 0x9c_9cff, 0xdc_9cff, 0xff_9cff, 0xff_94d3,
    0x00_0000, 0x13_1313, 0x28_2828, 0x36_3636, 0x4d_4d4d, 0x65_6565, 0x81_8181, 0x9f_9f9f,
    0xbc_bcbc, 0xe2_e2e2, 0xff_ffff,
];

/// Maps an IRC colour to a terminal colour. Codes 0 to 15 use the terminal's
/// own palette so the user's terminal theme applies.
pub fn color(c: IrcColor) -> Color {
    match c {
        IrcColor::Rgb(r, g, b) => Color::Rgb(r, g, b),
        IrcColor::Index(n) => match n {
            0 => Color::White,
            1 => Color::Black,
            2 => Color::Blue,
            3 => Color::Green,
            4 => Color::LightRed,
            5 => Color::Red,
            6 => Color::Magenta,
            7 => Color::Indexed(208),
            8 => Color::LightYellow,
            9 => Color::LightGreen,
            10 => Color::Cyan,
            11 => Color::LightCyan,
            12 => Color::LightBlue,
            13 => Color::LightMagenta,
            14 => Color::DarkGray,
            15 => Color::Gray,
            n => EXTENDED
                .get(usize::from(n) - 16)
                .map_or(Color::Reset, |&rgb| {
                    let [_, red, green, blue] = rgb.to_be_bytes();
                    Color::Rgb(red, green, blue)
                }),
        },
    }
}

/// Converts formatting from an IRC span to a terminal style.
pub fn span_style(style: format::Style) -> Style {
    let mut out = Style::default();
    let flags = [
        (style.bold, Modifier::BOLD),
        (style.italic, Modifier::ITALIC),
        (style.underline, Modifier::UNDERLINED),
        (style.strikethrough, Modifier::CROSSED_OUT),
        (style.reverse, Modifier::REVERSED),
    ];
    for (on, modifier) in flags {
        if on {
            out = out.add_modifier(modifier);
        }
    }
    if let Some(fg) = style.fg {
        out = out.fg(color(fg));
    }
    if let Some(bg) = style.bg {
        out = out.bg(color(bg));
    }
    out
}

/// The dark preset's nick colours, chosen so each nick keeps its colour.
const DARK_NICKS: [Color; 12] = [
    Color::Cyan,
    Color::Magenta,
    Color::Green,
    Color::Yellow,
    Color::LightBlue,
    Color::LightRed,
    Color::LightCyan,
    Color::LightMagenta,
    Color::LightGreen,
    Color::LightYellow,
    Color::Blue,
    Color::Red,
];

/// The light preset's nick colours: darker, for a light background.
const LIGHT_NICKS: [Color; 12] = [
    Color::Blue,
    Color::Magenta,
    Color::Green,
    Color::Red,
    Color::Cyan,
    Color::Indexed(94),
    Color::Indexed(130),
    Color::Indexed(25),
    Color::Indexed(90),
    Color::Indexed(28),
    Color::Indexed(124),
    Color::Indexed(31),
];

/// The theme presets.
pub const PRESETS: &[&str] = &["dark", "light", "high-contrast"];

/// Every element a theme can style, with what it is, in `[theme]` order.
pub const ELEMENTS: &[(&str, &str)] = &[
    (
        "bar",
        "the topic and status bars, and the focused pane's title",
    ),
    (
        "dim",
        "timestamps, separators, and joins, parts and other events",
    ),
    ("pending", "your messages until the server confirms them"),
    ("error", "errors"),
    ("highlight", "the nick on a line that mentions you"),
    ("selected", "the current buffer in the buffer list"),
    ("unread", "the rule above the first unread line"),
    ("join", "the --> of a join"),
    ("part", "the <-- of a part, kick or quit"),
    ("notice", "the -nick- of a notice"),
    ("action", "the * of an action"),
    ("prompt", "the input prompt and the buffer switcher"),
    ("hint", "examples on the welcome screen"),
    ("nick_mode", "@ and + in the nick list"),
    ("warning", "the INSECURE TLS warning"),
    ("question", "questions such as the paste prompt"),
    ("activity_none", "buffers with nothing new"),
    ("activity_events", "buffers with new events"),
    ("activity_messages", "buffers with new messages"),
    (
        "activity_highlight",
        "buffers with a mention or a private message",
    ),
    ("nicks", "the colours nicks are given, space-separated"),
];

/// Styles for everything the UI draws that isn't IRC formatting.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(
    missing_docs,
    reason = "each field is an element in ELEMENTS, documented there"
)]
pub struct Theme {
    pub bar: Style,
    pub dim: Style,
    pub pending: Style,
    pub error: Style,
    pub highlight: Style,
    pub selected: Style,
    pub unread: Style,
    pub join: Style,
    pub part: Style,
    pub notice: Style,
    pub action: Style,
    pub prompt: Style,
    pub hint: Style,
    pub nick_mode: Style,
    pub warning: Style,
    pub question: Style,
    pub activity_none: Style,
    pub activity_events: Style,
    pub activity_messages: Style,
    pub activity_highlight: Style,
    /// Colours for nicks, by a hash of the nick.
    pub nicks: Vec<Color>,
    /// `[ui] accessible`: draw in ASCII, and put text beside anything shown
    /// only by colour or style.
    pub accessible: bool,
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}

fn fg(colour: Color) -> Style {
    Style::default().fg(colour)
}

impl Theme {
    /// The default, for dark backgrounds.
    pub fn dark() -> Self {
        Self {
            bar: fg(Color::White).bg(Color::Blue),
            dim: fg(Color::DarkGray),
            pending: fg(Color::DarkGray),
            error: fg(Color::LightRed),
            highlight: fg(Color::Black).bg(Color::Yellow),
            selected: fg(Color::Black).bg(Color::Cyan),
            unread: fg(Color::LightRed),
            join: fg(Color::LightGreen),
            part: fg(Color::LightRed),
            notice: fg(Color::LightMagenta),
            action: Style::default().add_modifier(Modifier::BOLD),
            prompt: fg(Color::Cyan),
            hint: fg(Color::LightCyan),
            nick_mode: fg(Color::LightGreen),
            warning: fg(Color::White).bg(Color::Red).add_modifier(Modifier::BOLD),
            question: fg(Color::Black).bg(Color::Yellow),
            activity_none: fg(Color::Gray),
            activity_events: fg(Color::White),
            activity_messages: fg(Color::LightYellow),
            activity_highlight: fg(Color::LightMagenta).add_modifier(Modifier::BOLD),
            nicks: DARK_NICKS.to_vec(),
            accessible: false,
        }
    }

    /// For light backgrounds: no light colours on white.
    pub fn light() -> Self {
        Self {
            dim: fg(Color::DarkGray),
            error: fg(Color::Red),
            unread: fg(Color::Red),
            join: fg(Color::Green),
            part: fg(Color::Red),
            notice: fg(Color::Magenta),
            prompt: fg(Color::Blue),
            hint: fg(Color::Blue),
            nick_mode: fg(Color::Green),
            activity_none: fg(Color::DarkGray),
            activity_events: Style::default(),
            activity_messages: fg(Color::Blue).add_modifier(Modifier::BOLD),
            activity_highlight: fg(Color::Magenta).add_modifier(Modifier::BOLD),
            nicks: LIGHT_NICKS.to_vec(),
            ..Self::dark()
        }
    }

    /// For any background, and for low vision: no grey and no colour that
    /// carries meaning alone. Bold, italic and reverse video do the work, on
    /// the terminal's own background and foreground.
    pub fn high_contrast() -> Self {
        let plain = Style::default();
        let bold = plain.add_modifier(Modifier::BOLD);
        let reversed = plain.add_modifier(Modifier::REVERSED);
        let loud = reversed.add_modifier(Modifier::BOLD);
        Self {
            bar: reversed,
            dim: plain,
            pending: plain.add_modifier(Modifier::ITALIC),
            error: bold.add_modifier(Modifier::UNDERLINED),
            highlight: loud,
            selected: reversed,
            unread: bold,
            join: bold,
            part: bold,
            notice: bold,
            action: bold,
            prompt: bold,
            hint: bold,
            nick_mode: bold,
            warning: loud,
            question: reversed,
            activity_none: plain,
            activity_events: plain,
            activity_messages: bold,
            activity_highlight: loud,
            nicks: vec![Color::Reset],
            accessible: false,
        }
    }

    /// The theme `[theme]` describes, and anything wrong with it (the parts
    /// that are wrong are left as the preset has them). Without a preset,
    /// `accessible` picks high contrast.
    pub fn from_config(config: &ThemeConfig, accessible: bool) -> (Self, Vec<String>) {
        let mut problems = Vec::new();
        let fallback = if accessible { "high-contrast" } else { "dark" };
        let mut theme = match config.preset.as_deref().unwrap_or(fallback) {
            "dark" => Self::dark(),
            "light" => Self::light(),
            "high-contrast" => Self::high_contrast(),
            other => {
                problems.push(format!(
                    "[theme] preset {other:?} is not one of {}",
                    PRESETS.join(", ")
                ));
                Self::dark()
            }
        };
        for (name, value) in &config.styles {
            if name == "nicks" {
                match value
                    .split_whitespace()
                    .map(parse_colour)
                    .collect::<Result<Vec<_>, _>>()
                {
                    Ok(colours) if !colours.is_empty() => theme.nicks = colours,
                    Ok(_) => problems.push("[theme] nicks needs at least one colour".into()),
                    Err(e) => problems.push(format!("[theme] nicks: {e}")),
                }
                continue;
            }
            let Some(slot) = theme.slot(name) else {
                problems.push(format!(
                    "[theme] has no element {name:?}; see docs/CONFIG.md for the list"
                ));
                continue;
            };
            match parse_style(value) {
                Ok(style) => *slot = style,
                Err(e) => problems.push(format!("[theme] {name}: {e}")),
            }
        }
        theme.accessible = accessible;
        (theme, problems)
    }

    fn slot(&mut self, name: &str) -> Option<&mut Style> {
        Some(match name {
            "bar" => &mut self.bar,
            "dim" => &mut self.dim,
            "pending" => &mut self.pending,
            "error" => &mut self.error,
            "highlight" => &mut self.highlight,
            "selected" => &mut self.selected,
            "unread" => &mut self.unread,
            "join" => &mut self.join,
            "part" => &mut self.part,
            "notice" => &mut self.notice,
            "action" => &mut self.action,
            "prompt" => &mut self.prompt,
            "hint" => &mut self.hint,
            "nick_mode" => &mut self.nick_mode,
            "warning" => &mut self.warning,
            "question" => &mut self.question,
            "activity_none" => &mut self.activity_none,
            "activity_events" => &mut self.activity_events,
            "activity_messages" => &mut self.activity_messages,
            "activity_highlight" => &mut self.activity_highlight,
            _ => return None,
        })
    }

    /// Style for the text of a line of a given kind.
    pub fn line_style(&self, kind: LineKind) -> Style {
        match kind {
            LineKind::Message | LineKind::Action | LineKind::Notice | LineKind::Topic => {
                Style::default()
            }
            LineKind::Error => self.error,
            _ => self.dim,
        }
    }

    /// Buffer list entries by activity.
    pub fn activity(&self, level: Activity) -> Style {
        match level {
            Activity::None => self.activity_none,
            Activity::Events => self.activity_events,
            Activity::Messages => self.activity_messages,
            Activity::Highlight => self.activity_highlight,
        }
    }
}

/// Parses a style: modifiers, a colour, and `on` a background colour.
pub fn parse_style(text: &str) -> Result<Style, String> {
    let mut style = Style::default();
    let mut words = text.split_whitespace();
    let mut seen_fg = false;
    while let Some(word) = words.next() {
        let modifier = match word.to_lowercase().as_str() {
            "bold" => Some(Modifier::BOLD),
            "dim" | "faint" => Some(Modifier::DIM),
            "italic" => Some(Modifier::ITALIC),
            "underline" | "underlined" => Some(Modifier::UNDERLINED),
            "reverse" | "reversed" => Some(Modifier::REVERSED),
            "crossed" | "strikethrough" => Some(Modifier::CROSSED_OUT),
            "on" => {
                let bg = words.next().ok_or("\"on\" needs a colour after it")?;
                style = style.bg(parse_colour(bg)?);
                None
            }
            _ if seen_fg => return Err(format!("{word:?}: only one foreground colour")),
            _ => {
                style = style.fg(parse_colour(word)?);
                seen_fg = true;
                None
            }
        };
        if let Some(modifier) = modifier {
            style = style.add_modifier(modifier);
        }
    }
    Ok(style)
}

/// Parses a colour: a name, `#rrggbb`, or `colour0` to `colour255`.
pub fn parse_colour(text: &str) -> Result<Color, String> {
    if let Some(colour) = colour_named(text) {
        return Ok(colour);
    }
    let lower = text.to_lowercase();
    if let Some(hex) = lower.strip_prefix('#')
        && hex.len() == 6
        && let Ok(rgb) = u32::from_str_radix(hex, 16)
    {
        let [_, r, g, b] = rgb.to_be_bytes();
        return Ok(Color::Rgb(r, g, b));
    }
    if let Some(n) = lower
        .strip_prefix("colour")
        .or_else(|| lower.strip_prefix("color"))
        .and_then(|n| n.parse::<u8>().ok())
    {
        return Ok(Color::Indexed(n));
    }
    Err(format!(
        "{text:?} is not a colour; use a name such as light_blue, #rrggbb, or colour0 to colour255"
    ))
}

/// How nicks are coloured: by a hash of the name over the theme's colours
/// (unless turned off), with overrides for particular nicks.
#[derive(Clone, Debug)]
pub struct NickColours {
    enabled: bool,
    palette: Vec<Color>,
    overrides: std::collections::HashMap<String, Color>,
}

impl Default for NickColours {
    fn default() -> Self {
        Self {
            enabled: true,
            palette: DARK_NICKS.to_vec(),
            overrides: std::collections::HashMap::new(),
        }
    }
}

impl NickColours {
    /// From `[ui] nick_colours`, the `[nick_colours]` table and the theme's
    /// palette. Unknown colour names are skipped (the config is validated
    /// first).
    pub fn new(
        enabled: bool,
        overrides: &std::collections::BTreeMap<String, String>,
        palette: &[Color],
    ) -> Self {
        Self {
            enabled,
            palette: if palette.is_empty() {
                DARK_NICKS.to_vec()
            } else {
                palette.to_vec()
            },
            overrides: overrides
                .iter()
                .filter_map(|(nick, name)| Some((nick.to_lowercase(), parse_colour(name).ok()?)))
                .collect(),
        }
    }

    /// The colour for a nick.
    pub fn colour(&self, nick: &str) -> Color {
        match self.overrides.get(&nick.to_lowercase()) {
            Some(&colour) => colour,
            None if self.enabled => self
                .palette
                .get(nick_hash(nick) % self.palette.len().max(1))
                .copied()
                .unwrap_or(Color::Reset),
            None => Color::Reset,
        }
    }
}

/// A colour from its name in the config.
pub fn colour_named(name: &str) -> Option<Color> {
    Some(match name.to_lowercase().replace('-', "_").as_str() {
        "default" | "reset" => Color::Reset,
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "gray" | "grey" => Color::Gray,
        "dark_gray" | "dark_grey" => Color::DarkGray,
        "light_red" => Color::LightRed,
        "light_green" => Color::LightGreen,
        "light_yellow" => Color::LightYellow,
        "light_blue" => Color::LightBlue,
        "light_magenta" => Color::LightMagenta,
        "light_cyan" => Color::LightCyan,
        "white" => Color::White,
        _ => return None,
    })
}

/// FNV-1a of the nick, ignoring ASCII case: stable across runs and
/// platforms, unlike the std hasher.
fn nick_hash(nick: &str) -> usize {
    let hash = nick
        .bytes()
        .map(|b| b.to_ascii_lowercase())
        .fold(0x811c_9dc5_u32, |h, b| {
            (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
        });
    usize::try_from(hash).unwrap_or(0)
}

/// How many colours the terminal can show.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Depth {
    /// 24-bit colour: everything as it is.
    #[default]
    Truecolor,
    /// The 256-colour palette.
    Palette256,
    /// The 16 basic colours.
    Basic16,
    /// None: colours are dropped, and anything with a background is shown
    /// in reverse so bars and highlights still stand out.
    None,
}

impl Depth {
    /// Works out the depth for `[theme] colours`. With `auto`: none if
    /// `NO_COLOR` is set and not empty (no-color.org), truecolor if
    /// `COLORTERM` says so, 256 colours if `TERM` does, and 16 otherwise.
    /// A setting other than `auto` overrides `NO_COLOR`, as the convention
    /// allows.
    pub fn detect(
        mode: ColourMode,
        no_color: Option<&str>,
        colorterm: Option<&str>,
        term: Option<&str>,
    ) -> Self {
        match mode {
            ColourMode::Truecolor => Self::Truecolor,
            ColourMode::Palette256 => Self::Palette256,
            ColourMode::None => Self::None,
            ColourMode::Auto if no_color.is_some_and(|v| !v.is_empty()) => Self::None,
            ColourMode::Auto if term == Some("dumb") => Self::None,
            ColourMode::Auto
                if colorterm.is_some_and(|c| {
                    c.eq_ignore_ascii_case("truecolor") || c.eq_ignore_ascii_case("24bit")
                }) =>
            {
                Self::Truecolor
            }
            ColourMode::Auto if term.is_some_and(|t| t.contains("256color")) => Self::Palette256,
            ColourMode::Basic16 | ColourMode::Auto => Self::Basic16,
        }
    }

    /// Detects from the environment.
    pub fn from_env(mode: ColourMode) -> Self {
        let var = |name| std::env::var(name).ok();
        Self::detect(
            mode,
            var("NO_COLOR").as_deref(),
            var("COLORTERM").as_deref(),
            var("TERM").as_deref(),
        )
    }

    /// A colour as this depth can show it.
    pub fn colour(self, colour: Color) -> Color {
        match (self, colour) {
            (Self::None, _) => Color::Reset,
            (Self::Palette256, Color::Rgb(r, g, b)) => Color::Indexed(rgb_to_256(r, g, b)),
            (Self::Basic16, Color::Rgb(r, g, b)) => nearest_basic(r, g, b),
            (Self::Basic16, Color::Indexed(n)) if n < 16 => basic(n),
            (Self::Basic16, Color::Indexed(n)) => {
                let (r, g, b) = rgb_of(n);
                nearest_basic(r, g, b)
            }
            // Named colours are the terminal's own, at any depth.
            (_, c) => c,
        }
    }

    /// Fits every cell of a drawn frame to this depth.
    pub fn apply(self, cells: &mut ratatui::buffer::Buffer) {
        if self == Self::Truecolor {
            return;
        }
        for cell in &mut cells.content {
            if self == Self::None {
                if cell.bg != Color::Reset {
                    cell.modifier.toggle(Modifier::REVERSED);
                }
                cell.fg = Color::Reset;
                cell.bg = Color::Reset;
                cell.underline_color = Color::Reset;
            } else {
                cell.fg = self.colour(cell.fg);
                cell.bg = self.colour(cell.bg);
                cell.underline_color = self.colour(cell.underline_color);
            }
        }
    }
}

/// The basic colours as xterm draws them by default, by index.
const BASIC: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (205, 0, 0),
    (0, 205, 0),
    (205, 205, 0),
    (0, 0, 238),
    (205, 0, 205),
    (0, 205, 205),
    (229, 229, 229),
    (127, 127, 127),
    (255, 0, 0),
    (0, 255, 0),
    (255, 255, 0),
    (92, 92, 255),
    (255, 0, 255),
    (0, 255, 255),
    (255, 255, 255),
];

/// The named colour with this basic index.
fn basic(n: u8) -> Color {
    match n {
        0 => Color::Black,
        1 => Color::Red,
        2 => Color::Green,
        3 => Color::Yellow,
        4 => Color::Blue,
        5 => Color::Magenta,
        6 => Color::Cyan,
        7 => Color::Gray,
        8 => Color::DarkGray,
        9 => Color::LightRed,
        10 => Color::LightGreen,
        11 => Color::LightYellow,
        12 => Color::LightBlue,
        13 => Color::LightMagenta,
        14 => Color::LightCyan,
        _ => Color::White,
    }
}

fn distance((r1, g1, b1): (u8, u8, u8), (r2, g2, b2): (u8, u8, u8)) -> u32 {
    let d = |a: u8, b: u8| u32::from(a.abs_diff(b)).pow(2);
    d(r1, r2) + d(g1, g2) + d(b1, b2)
}

fn nearest_basic(r: u8, g: u8, b: u8) -> Color {
    let best = (0u8..16)
        .min_by_key(|&i| {
            distance(
                BASIC.get(usize::from(i)).copied().unwrap_or_default(),
                (r, g, b),
            )
        })
        .unwrap_or(0);
    basic(best)
}

/// The levels of the 6×6×6 colour cube in the 256-colour palette.
const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];

/// The colour a 256-palette index stands for.
fn rgb_of(n: u8) -> (u8, u8, u8) {
    match n {
        0..=15 => BASIC.get(usize::from(n)).copied().unwrap_or_default(),
        16..=231 => {
            let i = n - 16;
            let level = |x: u8| CUBE.get(usize::from(x)).copied().unwrap_or(0);
            (level(i / 36), level(i / 6 % 6), level(i % 6))
        }
        _ => {
            let v = 8 + (n - 232) * 10;
            (v, v, v)
        }
    }
}

/// The nearest colour in the 256-colour palette's cube or grey ramp (not
/// the basic 16, which terminals redefine).
fn rgb_to_256(r: u8, g: u8, b: u8) -> u8 {
    let nearest_level = |v: u8| {
        (0u8..6)
            .min_by_key(|&i| CUBE.get(usize::from(i)).copied().unwrap_or(0).abs_diff(v))
            .unwrap_or(0)
    };
    let (ri, gi, bi) = (nearest_level(r), nearest_level(g), nearest_level(b));
    let cube = 16 + 36 * ri + 6 * gi + bi;
    let average = u8::try_from((u16::from(r) + u16::from(g) + u16::from(b)) / 3).unwrap_or(0);
    let grey = 232 + (average.saturating_sub(3) / 10).min(23);
    if distance(rgb_of(grey), (r, g, b)) < distance(rgb_of(cube), (r, g, b)) {
        grey
    } else {
        cube
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_colours() {
        assert_eq!(color(IrcColor::Index(4)), Color::LightRed);
        assert_eq!(color(IrcColor::Index(16)), Color::Rgb(0x47, 0, 0));
        assert_eq!(color(IrcColor::Index(98)), Color::Rgb(0xff, 0xff, 0xff));
        assert_eq!(color(IrcColor::Rgb(1, 2, 3)), Color::Rgb(1, 2, 3));
    }

    #[test]
    fn styles() {
        let s = span_style(format::Style {
            bold: true,
            italic: true,
            underline: true,
            strikethrough: true,
            reverse: true,
            fg: Some(IrcColor::Index(2)),
            bg: Some(IrcColor::Index(1)),
            ..format::Style::default()
        });
        assert!(
            s.add_modifier
                .contains(Modifier::BOLD | Modifier::ITALIC | Modifier::REVERSED)
        );
        assert_eq!((s.fg, s.bg), (Some(Color::Blue), Some(Color::Black)));
    }

    #[test]
    fn nick_colours_are_stable_and_case_insensitive() {
        let colours = NickColours::default();
        assert_eq!(colours.colour("Alice"), colours.colour("alice"));
        let distinct: std::collections::HashSet<_> = ["a", "b", "c", "d", "e", "f", "g", "h"]
            .iter()
            .map(|n| colours.colour(n))
            .collect();
        assert!(distinct.len() > 3);
        let theme = Theme::dark();
        assert_eq!(theme.line_style(LineKind::Error).fg, Some(Color::LightRed));
        assert!(
            theme
                .activity(Activity::Highlight)
                .add_modifier
                .contains(Modifier::BOLD)
        );
        let mut overrides = std::collections::BTreeMap::new();
        overrides.insert("Bob".to_owned(), "#ff8000".to_owned());
        let custom = NickColours::new(true, &overrides, &[Color::Red]);
        assert_eq!(custom.colour("bob"), Color::Rgb(255, 128, 0));
        assert_eq!(custom.colour("anyone"), Color::Red);
        assert_eq!(
            NickColours::new(false, &overrides, &[]).colour("x"),
            Color::Reset
        );
    }

    #[test]
    fn parses_styles_and_colours() {
        assert_eq!(
            parse_style("bold white on blue").unwrap(),
            Style::default()
                .fg(Color::White)
                .bg(Color::Blue)
                .add_modifier(Modifier::BOLD)
        );
        assert_eq!(
            parse_style("#FF8000 on colour236 underline").unwrap(),
            Style::default()
                .fg(Color::Rgb(255, 128, 0))
                .bg(Color::Indexed(236))
                .add_modifier(Modifier::UNDERLINED)
        );
        assert_eq!(parse_style("").unwrap(), Style::default());
        assert_eq!(parse_colour("Dark-Grey"), Ok(Color::DarkGray));
        assert_eq!(parse_colour("color9"), Ok(Color::Indexed(9)));
        for bad in ["on", "red blue", "sparkly", "#12345", "colour256"] {
            assert!(parse_style(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn themes_from_config() {
        let (theme, problems) = Theme::from_config(&ThemeConfig::default(), false);
        assert!(problems.is_empty());
        assert_eq!(theme, Theme::dark());
        assert_eq!(
            Theme::from_config(&ThemeConfig::default(), true).0,
            Theme {
                accessible: true,
                ..Theme::high_contrast()
            },
            "accessible picks high contrast"
        );
        let mut config = ThemeConfig {
            preset: Some("light".into()),
            ..ThemeConfig::default()
        };
        config.styles.insert("bar".into(), "black on white".into());
        config.styles.insert("nicks".into(), "red #00ff00".into());
        config.styles.insert("sparkle".into(), "red".into());
        config.styles.insert("error".into(), "puce".into());
        assert_eq!(
            Theme::from_config(&config, true).0.notice,
            Theme::light().notice,
            "a chosen preset wins"
        );
        let (theme, problems) = Theme::from_config(&config, false);
        assert_eq!(theme.bar, fg(Color::Black).bg(Color::White));
        assert_eq!(theme.nicks, [Color::Red, Color::Rgb(0, 255, 0)]);
        assert_eq!(
            theme.error,
            Theme::light().error,
            "a bad style keeps the preset's"
        );
        assert_eq!(problems.len(), 2, "{problems:?}");
        config.preset = Some("neon".into());
        assert_eq!(Theme::from_config(&config, false).1.len(), 3);
        // Every element can be set, and has a slot.
        let mut theme = Theme::dark();
        for (name, _) in ELEMENTS.iter().filter(|(n, _)| *n != "nicks") {
            assert!(theme.slot(name).is_some(), "{name}");
        }
    }

    /// High contrast uses no colour of its own: only the terminal's
    /// foreground and background, with modifiers.
    #[test]
    fn high_contrast_has_no_colours() {
        let mut theme = Theme::high_contrast();
        assert_eq!(theme.nicks, [Color::Reset]);
        assert_ne!(
            theme.pending, theme.dim,
            "unconfirmed messages still stand out"
        );
        for (name, _) in ELEMENTS.iter().filter(|(n, _)| *n != "nicks") {
            let style = *theme.slot(name).unwrap();
            assert!(style.fg.is_none() && style.bg.is_none(), "{name}");
        }
    }

    #[test]
    fn detects_the_colour_depth() {
        let auto = ColourMode::Auto;
        assert_eq!(
            Depth::detect(auto, Some("1"), Some("truecolor"), None),
            Depth::None
        );
        assert_eq!(
            Depth::detect(auto, Some(""), Some("24bit"), None),
            Depth::Truecolor,
            "an empty NO_COLOR doesn't count"
        );
        assert_eq!(
            Depth::detect(auto, None, None, Some("xterm-256color")),
            Depth::Palette256
        );
        assert_eq!(
            Depth::detect(auto, None, None, Some("xterm")),
            Depth::Basic16
        );
        assert_eq!(Depth::detect(auto, None, None, Some("dumb")), Depth::None);
        assert_eq!(
            Depth::detect(ColourMode::Truecolor, Some("1"), None, None),
            Depth::Truecolor,
            "the config overrides NO_COLOR"
        );
    }

    #[test]
    fn fits_colours_to_the_depth() {
        let orange = Color::Rgb(255, 135, 0);
        assert_eq!(Depth::Truecolor.colour(orange), orange);
        assert_eq!(Depth::Palette256.colour(orange), Color::Indexed(208));
        assert_eq!(
            Depth::Palette256.colour(Color::Rgb(128, 128, 128)),
            Color::Indexed(244)
        );
        assert_eq!(
            Depth::Basic16.colour(Color::Rgb(250, 10, 10)),
            Color::LightRed
        );
        assert_eq!(Depth::Basic16.colour(Color::Indexed(4)), Color::Blue);
        assert_eq!(Depth::Basic16.colour(Color::Indexed(21)), Color::Blue);
        assert_eq!(Depth::Basic16.colour(Color::Cyan), Color::Cyan);
        assert_eq!(Depth::None.colour(Color::Cyan), Color::Reset);

        let mut cells = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, 2, 1));
        cells[(0, 0)].set_fg(Color::White).set_bg(Color::Blue);
        cells[(1, 0)].set_fg(orange);
        let mut basic = cells.clone();
        Depth::Basic16.apply(&mut basic);
        assert_eq!(basic[(1, 0)].fg, Color::Yellow);
        Depth::None.apply(&mut cells);
        assert_eq!(cells[(0, 0)].fg, Color::Reset);
        assert!(
            cells[(0, 0)].modifier.contains(Modifier::REVERSED),
            "a background becomes reverse video"
        );
        assert!(!cells[(1, 0)].modifier.contains(Modifier::REVERSED));
    }
}
