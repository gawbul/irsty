//! Key bindings: named actions, key chords such as `ctrl-n` or `alt-left`,
//! the `irssi` and `weechat` presets, and overrides from `[keys]` in the
//! config (`/bind`).

use std::collections::{BTreeMap, HashMap};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Something a key can do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    /// Send the line or run the command.
    Send,
    /// Complete the word before the cursor, or the next match.
    Complete,
    /// The previous match.
    CompleteBack,
    /// Open the fuzzy buffer switcher.
    Switcher,
    /// Next buffer.
    NextBuffer,
    /// Previous buffer.
    PrevBuffer,
    /// The buffer with the most important unread activity.
    NextActive,
    /// The buffer shown before this one.
    Back,
    /// Undo [`Action::Back`].
    Forward,
    /// The buffer with this number (1 to 99).
    Buffer(u8),
    /// Wait for two digits naming a buffer (WeeChat's Alt+J).
    Jump,
    /// Scroll back a page.
    PageUp,
    /// Scroll forward a page.
    PageDown,
    /// Scroll back a few lines.
    ScrollUp,
    /// Scroll forward a few lines.
    ScrollDown,
    /// The oldest lines.
    ScrollTop,
    /// The newest lines.
    ScrollBottom,
    /// The previous line in history.
    HistoryPrev,
    /// The next line in history.
    HistoryNext,
    /// Cursor left.
    Left,
    /// Cursor right.
    Right,
    /// Back a word.
    WordLeft,
    /// Forward a word.
    WordRight,
    /// Start of the line.
    Home,
    /// End of the line.
    End,
    /// Delete before the cursor.
    Backspace,
    /// Delete under the cursor.
    Delete,
    /// Cut the previous word.
    KillWordBack,
    /// Cut the next word.
    KillWordForward,
    /// Cut to the start.
    KillToStart,
    /// Cut to the end.
    KillToEnd,
    /// Paste the last cut.
    Yank,
    /// Swap the characters around the cursor.
    Transpose,
    /// Clear and redraw the screen.
    Redraw,
    /// Suspend Irsty (resume with `fg`).
    Suspend,
    /// Do nothing (to unbind a preset key).
    None,
}

/// Every action with a fixed name, for `/bind` and completion.
const NAMED: &[(&str, Action)] = &[
    ("send", Action::Send),
    ("complete", Action::Complete),
    ("complete_back", Action::CompleteBack),
    ("switcher", Action::Switcher),
    ("next_buffer", Action::NextBuffer),
    ("prev_buffer", Action::PrevBuffer),
    ("next_active", Action::NextActive),
    ("back", Action::Back),
    ("forward", Action::Forward),
    ("jump", Action::Jump),
    ("page_up", Action::PageUp),
    ("page_down", Action::PageDown),
    ("scroll_up", Action::ScrollUp),
    ("scroll_down", Action::ScrollDown),
    ("scroll_top", Action::ScrollTop),
    ("scroll_bottom", Action::ScrollBottom),
    ("history_prev", Action::HistoryPrev),
    ("history_next", Action::HistoryNext),
    ("left", Action::Left),
    ("right", Action::Right),
    ("word_left", Action::WordLeft),
    ("word_right", Action::WordRight),
    ("home", Action::Home),
    ("end", Action::End),
    ("backspace", Action::Backspace),
    ("delete", Action::Delete),
    ("kill_word_back", Action::KillWordBack),
    ("kill_word_forward", Action::KillWordForward),
    ("kill_to_start", Action::KillToStart),
    ("kill_to_end", Action::KillToEnd),
    ("yank", Action::Yank),
    ("transpose", Action::Transpose),
    ("redraw", Action::Redraw),
    ("suspend", Action::Suspend),
    ("none", Action::None),
];

impl Action {
    /// Parses a name such as `next_buffer` or `buffer_12`.
    pub fn from_name(name: &str) -> Option<Self> {
        let name = name.to_ascii_lowercase();
        if let Some(n) = name.strip_prefix("buffer_") {
            return n
                .parse()
                .ok()
                .filter(|n| (1..=99).contains(n))
                .map(Self::Buffer);
        }
        NAMED.iter().find(|(n, _)| *n == name).map(|&(_, a)| a)
    }

    /// The name used in config and `/bind`.
    pub fn name(self) -> String {
        match self {
            Self::Buffer(n) => format!("buffer_{n}"),
            other => NAMED
                .iter()
                .find(|&&(_, a)| a == other)
                .map_or_else(String::new, |(n, _)| (*n).to_owned()),
        }
    }

    /// Every action name, for completion (`buffer_N` stands for the numbered
    /// ones).
    pub fn names() -> impl Iterator<Item = &'static str> {
        NAMED
            .iter()
            .map(|(n, _)| *n)
            .chain(std::iter::once("buffer_N"))
    }
}

/// A key with its modifiers, normalised so the same key always looks the same.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Chord {
    code: ChordKey,
    ctrl: bool,
    alt: bool,
    shift: bool,
}

/// The key part of a chord.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum ChordKey {
    Char(char),
    F(u8),
    Named(&'static str),
}

const NAMED_KEYS: &[(&str, KeyCode)] = &[
    ("enter", KeyCode::Enter),
    ("tab", KeyCode::Tab),
    ("backspace", KeyCode::Backspace),
    ("delete", KeyCode::Delete),
    ("insert", KeyCode::Insert),
    ("esc", KeyCode::Esc),
    ("left", KeyCode::Left),
    ("right", KeyCode::Right),
    ("up", KeyCode::Up),
    ("down", KeyCode::Down),
    ("home", KeyCode::Home),
    ("end", KeyCode::End),
    ("pageup", KeyCode::PageUp),
    ("pagedown", KeyCode::PageDown),
];

impl Chord {
    /// The chord a key event represents, or `None` for keys we don't bind.
    pub fn from_event(key: &KeyEvent) -> Option<Self> {
        let mods = key.modifiers;
        let (code, shift) = match key.code {
            // Shift is already in the character ('A', '<'); Ctrl+letter may
            // arrive in either case.
            KeyCode::Char(c) => {
                let c = if mods.contains(KeyModifiers::CONTROL) {
                    c.to_ascii_lowercase()
                } else {
                    c
                };
                (ChordKey::Char(c), false)
            }
            KeyCode::BackTab => (ChordKey::Named("tab"), true),
            KeyCode::F(n) => (ChordKey::F(n), mods.contains(KeyModifiers::SHIFT)),
            code => {
                let name = NAMED_KEYS.iter().find(|(_, k)| *k == code)?.0;
                (ChordKey::Named(name), mods.contains(KeyModifiers::SHIFT))
            }
        };
        Some(Self {
            code,
            ctrl: mods.contains(KeyModifiers::CONTROL),
            alt: mods.contains(KeyModifiers::ALT),
            shift,
        })
    }

    /// Parses `ctrl-n`, `alt-left`, `shift-tab`, `f5`, `alt-1` and so on.
    pub fn parse(text: &str) -> Result<Self, String> {
        let bad = || format!("{text:?} is not a key; try ctrl-n, alt-left, shift-tab or f5");
        let mut chord = Self {
            code: ChordKey::Char(' '),
            ctrl: false,
            alt: false,
            shift: false,
        };
        let lower = text.to_ascii_lowercase();
        let mut rest = lower.as_str();
        // A trailing "-" is the minus key, as in "alt--".
        while let Some((modifier, after)) = rest.split_once('-').filter(|(_, a)| !a.is_empty()) {
            match modifier {
                "ctrl" | "c" => chord.ctrl = true,
                "alt" | "meta" | "m" => chord.alt = true,
                "shift" | "s" => chord.shift = true,
                _ => return Err(bad()),
            }
            rest = after;
        }
        chord.code = if let Some(n) = rest.strip_prefix('f').and_then(|n| n.parse().ok()) {
            ChordKey::F(n)
        } else if rest == "space" {
            ChordKey::Char(' ')
        } else if let Some(&(name, _)) = NAMED_KEYS.iter().find(|(n, _)| *n == rest) {
            ChordKey::Named(name)
        } else {
            let mut chars = text
                .chars()
                .skip(text.chars().count() - rest.chars().count());
            match (chars.next(), chars.next()) {
                (Some(c), None) => ChordKey::Char(if chord.ctrl {
                    c.to_ascii_lowercase()
                } else {
                    c
                }),
                _ => return Err(bad()),
            }
        };
        if matches!(chord.code, ChordKey::Char(_)) {
            chord.shift = false;
        }
        Ok(chord)
    }
}

impl std::fmt::Display for Chord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (on, name) in [
            (self.ctrl, "ctrl-"),
            (self.alt, "alt-"),
            (self.shift, "shift-"),
        ] {
            if on {
                f.write_str(name)?;
            }
        }
        match self.code {
            ChordKey::Char(' ') => f.write_str("space"),
            ChordKey::Char(c) => write!(f, "{c}"),
            ChordKey::F(n) => write!(f, "f{n}"),
            ChordKey::Named(name) => f.write_str(name),
        }
    }
}

/// Bindings shared by both presets: emacs-style editing and the common keys.
const COMMON: &[(&str, &str)] = &[
    ("enter", "send"),
    ("tab", "complete"),
    ("shift-tab", "complete_back"),
    ("ctrl-g", "switcher"),
    ("ctrl-n", "next_buffer"),
    ("ctrl-p", "prev_buffer"),
    ("alt-right", "next_buffer"),
    ("alt-left", "prev_buffer"),
    ("alt-a", "next_active"),
    ("alt-<", "back"),
    ("alt->", "forward"),
    ("pageup", "page_up"),
    ("pagedown", "page_down"),
    ("alt-pageup", "scroll_up"),
    ("alt-pagedown", "scroll_down"),
    ("alt-home", "scroll_top"),
    ("alt-end", "scroll_bottom"),
    ("ctrl-end", "scroll_bottom"),
    ("up", "history_prev"),
    ("down", "history_next"),
    ("ctrl-up", "history_prev"),
    ("ctrl-down", "history_next"),
    ("left", "left"),
    ("right", "right"),
    ("ctrl-b", "left"),
    ("ctrl-f", "right"),
    ("ctrl-left", "word_left"),
    ("ctrl-right", "word_right"),
    ("alt-b", "word_left"),
    ("alt-f", "word_right"),
    ("home", "home"),
    ("end", "end"),
    ("ctrl-a", "home"),
    ("ctrl-e", "end"),
    ("backspace", "backspace"),
    ("ctrl-h", "backspace"),
    ("delete", "delete"),
    ("ctrl-d", "delete"),
    ("ctrl-w", "kill_word_back"),
    ("alt-backspace", "kill_word_back"),
    ("alt-d", "kill_word_forward"),
    ("ctrl-u", "kill_to_start"),
    ("ctrl-k", "kill_to_end"),
    ("ctrl-y", "yank"),
    ("ctrl-t", "transpose"),
    ("ctrl-l", "redraw"),
    ("ctrl-z", "suspend"),
    ("alt-1", "buffer_1"),
    ("alt-2", "buffer_2"),
    ("alt-3", "buffer_3"),
    ("alt-4", "buffer_4"),
    ("alt-5", "buffer_5"),
    ("alt-6", "buffer_6"),
    ("alt-7", "buffer_7"),
    ("alt-8", "buffer_8"),
    ("alt-9", "buffer_9"),
    ("alt-0", "buffer_10"),
];

/// irssi: Alt+Q to Alt+P are buffers 11 to 20.
const IRSSI: &[(&str, &str)] = &[
    ("alt-q", "buffer_11"),
    ("alt-w", "buffer_12"),
    ("alt-e", "buffer_13"),
    ("alt-r", "buffer_14"),
    ("alt-t", "buffer_15"),
    ("alt-y", "buffer_16"),
    ("alt-u", "buffer_17"),
    ("alt-i", "buffer_18"),
    ("alt-o", "buffer_19"),
    ("alt-p", "buffer_20"),
];

/// WeeChat: Alt+J then two digits jumps to a buffer, Alt+Up and Alt+Down and
/// F5 and F6 move between buffers, and Alt+/ goes back.
const WEECHAT: &[(&str, &str)] = &[
    ("alt-j", "jump"),
    ("alt-up", "prev_buffer"),
    ("alt-down", "next_buffer"),
    ("f5", "prev_buffer"),
    ("f6", "next_buffer"),
    ("alt-/", "back"),
];

/// The preset names.
pub const PRESETS: &[&str] = &["irssi", "weechat"];

/// Key bindings in effect.
#[derive(Clone, Debug)]
pub struct Keymap {
    bindings: HashMap<Chord, Action>,
}

impl Default for Keymap {
    fn default() -> Self {
        Self::new("irssi", &BTreeMap::new()).0
    }
}

impl Keymap {
    /// Builds a preset with overrides applied. Returns problems with the
    /// overrides (which are skipped) so they can be reported.
    pub fn new(preset: &str, overrides: &BTreeMap<String, String>) -> (Self, Vec<String>) {
        let extra = if preset.eq_ignore_ascii_case("weechat") {
            WEECHAT
        } else {
            IRSSI
        };
        let mut bindings = HashMap::new();
        for (key, action) in COMMON.iter().chain(extra) {
            if let (Ok(chord), Some(action)) = (Chord::parse(key), Action::from_name(action)) {
                bindings.insert(chord, action);
            }
        }
        let mut problems = Vec::new();
        for (key, action) in overrides {
            match (Chord::parse(key), Action::from_name(action)) {
                (Ok(chord), Some(action)) => {
                    bindings.insert(chord, action);
                }
                (Err(error), _) => problems.push(format!("[keys] {error}")),
                (_, None) => problems.push(format!("[keys] {key}: no action named {action:?}")),
            }
        }
        (Self { bindings }, problems)
    }

    /// The action for a key event.
    pub fn action(&self, key: &KeyEvent) -> Option<Action> {
        Chord::from_event(key).and_then(|c| self.bindings.get(&c).copied())
    }

    /// The action bound to a chord.
    pub fn get(&self, chord: &Chord) -> Option<Action> {
        self.bindings.get(chord).copied()
    }

    /// Every binding, sorted by key, without the `none` ones.
    pub fn list(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = self
            .bindings
            .iter()
            .filter(|(_, a)| **a != Action::None)
            .map(|(c, a)| (c.to_string(), a.name()))
            .collect();
        out.sort();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn chords_parse_and_print() {
        for text in [
            "ctrl-n",
            "alt-left",
            "shift-tab",
            "f5",
            "alt-1",
            "alt-<",
            "ctrl-alt-x",
            "alt-pageup",
            "space",
            "alt--",
        ] {
            assert_eq!(Chord::parse(text).unwrap().to_string(), text, "{text}");
        }
        assert_eq!(Chord::parse("C-N").unwrap().to_string(), "ctrl-n");
        assert_eq!(Chord::parse("Meta-b").unwrap().to_string(), "alt-b");
        // Letters keep their case: alt-B is Alt+Shift+B.
        assert_eq!(Chord::parse("alt-B").unwrap().to_string(), "alt-B");
        assert!(Chord::parse("hyper-x").is_err());
        assert!(Chord::parse("ctrl-xy").is_err());
        assert!(Chord::parse("").is_err());
    }

    #[test]
    fn events_normalise() {
        let chord = |code, mods| Chord::from_event(&key(code, mods)).unwrap().to_string();
        assert_eq!(chord(KeyCode::Char('N'), KeyModifiers::CONTROL), "ctrl-n");
        assert_eq!(
            chord(KeyCode::Char('<'), KeyModifiers::ALT | KeyModifiers::SHIFT),
            "alt-<"
        );
        assert_eq!(chord(KeyCode::BackTab, KeyModifiers::SHIFT), "shift-tab");
        assert_eq!(chord(KeyCode::F(5), KeyModifiers::NONE), "f5");
        assert!(Chord::from_event(&key(KeyCode::Null, KeyModifiers::NONE)).is_none());
    }

    #[test]
    fn presets_differ_where_they_should() {
        let irssi = Keymap::default();
        let (weechat, problems) = Keymap::new("weechat", &BTreeMap::new());
        assert!(problems.is_empty());
        let alt = |c| key(KeyCode::Char(c), KeyModifiers::ALT);
        // irssi has no Alt+J; buffers 11 to 20 are on Alt+Q to Alt+P.
        assert_eq!(irssi.action(&alt('j')), None);
        assert_eq!(irssi.action(&alt('u')), Some(Action::Buffer(17)));
        assert_eq!(weechat.action(&alt('u')), None);
        assert_eq!(weechat.action(&alt('j')), Some(Action::Jump));
        assert_eq!(irssi.action(&key(KeyCode::F(6), KeyModifiers::NONE)), None);
        assert_eq!(
            weechat.action(&key(KeyCode::F(6), KeyModifiers::NONE)),
            Some(Action::NextBuffer)
        );
        for map in [&irssi, &weechat] {
            assert_eq!(map.action(&alt('1')), Some(Action::Buffer(1)));
            assert_eq!(
                map.action(&key(KeyCode::Char('n'), KeyModifiers::CONTROL)),
                Some(Action::NextBuffer)
            );
            assert_eq!(
                map.action(&key(KeyCode::Tab, KeyModifiers::NONE)),
                Some(Action::Complete)
            );
        }
        // Every preset entry is valid.
        for (k, a) in COMMON.iter().chain(IRSSI).chain(WEECHAT) {
            assert!(
                Chord::parse(k).is_ok() && Action::from_name(a).is_some(),
                "{k} = {a}"
            );
        }
    }

    #[test]
    fn overrides_apply_and_report_problems() {
        let overrides: BTreeMap<String, String> = [
            ("ctrl-g", "none"),
            ("alt-x", "switcher"),
            ("f12", "buffer_42"),
            ("bogus-key", "send"),
            ("alt-z", "fly"),
        ]
        .into_iter()
        .map(|(k, a)| (k.to_owned(), a.to_owned()))
        .collect();
        let (map, problems) = Keymap::new("irssi", &overrides);
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert_eq!(
            map.action(&key(KeyCode::Char('g'), KeyModifiers::CONTROL)),
            Some(Action::None)
        );
        assert_eq!(
            map.action(&key(KeyCode::Char('x'), KeyModifiers::ALT)),
            Some(Action::Switcher)
        );
        assert_eq!(
            map.action(&key(KeyCode::F(12), KeyModifiers::NONE)),
            Some(Action::Buffer(42))
        );
        assert!(
            !map.list().iter().any(|(k, _)| k == "ctrl-g"),
            "none is hidden"
        );
    }

    #[test]
    fn action_names_round_trip() {
        for name in Action::names().filter(|n| *n != "buffer_N") {
            assert_eq!(Action::from_name(name).unwrap().name(), name);
        }
        assert_eq!(Action::from_name("BUFFER_7"), Some(Action::Buffer(7)));
        assert_eq!(Action::from_name("buffer_0"), None);
        assert_eq!(Action::from_name("buffer_100"), None);
    }
}
