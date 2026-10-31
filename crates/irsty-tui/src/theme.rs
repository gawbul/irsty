//! Colours and styles. Themes arrive in M12; until then these are the defaults.

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

/// Colours for nicks, chosen so each nick keeps its colour.
const NICK_COLOURS: [Color; 12] = [
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

/// A stable colour for a nick, ignoring ASCII case.
/// How nicks are coloured: by a hash of the name (unless turned off), with
/// overrides for particular nicks.
#[derive(Clone, Debug)]
pub struct NickColours {
    enabled: bool,
    overrides: std::collections::HashMap<String, Color>,
}

impl Default for NickColours {
    fn default() -> Self {
        Self {
            enabled: true,
            overrides: std::collections::HashMap::new(),
        }
    }
}

impl NickColours {
    /// From `[ui] nick_colours` and the `[nick_colours]` table. Unknown
    /// colour names are skipped (the config is validated first).
    pub fn new(enabled: bool, overrides: &std::collections::BTreeMap<String, String>) -> Self {
        Self {
            enabled,
            overrides: overrides
                .iter()
                .filter_map(|(nick, name)| Some((nick.to_lowercase(), colour_named(name)?)))
                .collect(),
        }
    }

    /// The colour for a nick.
    pub fn colour(&self, nick: &str) -> Color {
        match self.overrides.get(&nick.to_lowercase()) {
            Some(&colour) => colour,
            None if self.enabled => nick_color(nick),
            None => Color::Reset,
        }
    }
}

/// A colour from its name in the config.
pub fn colour_named(name: &str) -> Option<Color> {
    Some(match name.to_lowercase().as_str() {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "gray" => Color::Gray,
        "dark_gray" => Color::DarkGray,
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

/// The nick of a highlighted line: dark on yellow, so mentions stand out.
pub fn highlight() -> Style {
    Style::default().fg(Color::Black).bg(Color::Yellow)
}

pub fn nick_color(nick: &str) -> Color {
    // FNV-1a: stable across runs and platforms, unlike the std hasher.
    let hash = nick
        .bytes()
        .map(|b| b.to_ascii_lowercase())
        .fold(0x811c_9dc5_u32, |h, b| {
            (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
        });
    NICK_COLOURS
        .get(usize::try_from(hash).unwrap_or(0) % NICK_COLOURS.len())
        .copied()
        .unwrap_or(Color::Reset)
}

/// Dim text for timestamps, separators and events.
pub fn dim() -> Style {
    Style::default().fg(Color::DarkGray)
}

/// Style for the text of a line of a given kind.
pub fn line_style(kind: LineKind) -> Style {
    match kind {
        LineKind::Message | LineKind::Action | LineKind::Notice | LineKind::Topic => {
            Style::default()
        }
        LineKind::Error => Style::default().fg(Color::LightRed),
        _ => dim(),
    }
}

/// Bars along the top and bottom.
pub fn bar() -> Style {
    Style::default().fg(Color::White).bg(Color::Blue)
}

/// Buffer list entries by activity.
pub fn activity(level: Activity) -> Style {
    match level {
        Activity::None => Style::default().fg(Color::Gray),
        Activity::Events => Style::default().fg(Color::White),
        Activity::Messages => Style::default().fg(Color::LightYellow),
        Activity::Highlight => Style::default()
            .fg(Color::LightMagenta)
            .add_modifier(Modifier::BOLD),
    }
}

/// The selected buffer in the buffer list.
pub fn selected() -> Style {
    Style::default().fg(Color::Black).bg(Color::Cyan)
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
        assert_eq!(nick_color("Alice"), nick_color("alice"));
        let distinct: std::collections::HashSet<_> = ["a", "b", "c", "d", "e", "f", "g", "h"]
            .iter()
            .map(|n| nick_color(n))
            .collect();
        assert!(distinct.len() > 3);
        assert_eq!(line_style(LineKind::Error).fg, Some(Color::LightRed));
        assert!(
            activity(Activity::Highlight)
                .add_modifier
                .contains(Modifier::BOLD)
        );
    }
}
