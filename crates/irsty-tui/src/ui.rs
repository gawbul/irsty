//! UI state that is not client state: which buffer is shown and the history of
//! those shown, scroll positions, the input line, completion, the switcher and
//! other prompts, the config file, and layout facts needed to handle the mouse.

use std::collections::HashMap;

use irsty_core::config::{Config, UiConfig};
use irsty_core::config_file::ConfigFile;
use irsty_core::event::NetworkId;
use irsty_core::state::LineKind;
use irsty_core::state::{BufferId, BufferKind, State};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use ratatui::layout::Rect;

use crate::complete::fuzzy_score;
use crate::input::InputLine;
use crate::keymap::Keymap;

/// Most buffers remembered for going back.
const HISTORY: usize = 50;

/// Tab completion in progress.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    /// Where the completed word starts in the input.
    pub start: usize,
    /// Candidates, best first.
    pub words: Vec<String>,
    /// What follows a chosen word.
    pub suffix: &'static str,
    /// Which candidate is showing, once one is.
    pub index: Option<usize>,
    /// The input as completion left it; any other edit ends completion.
    pub text: String,
}

/// The fuzzy buffer switcher, while open.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Switcher {
    /// What has been typed.
    pub query: String,
    /// The highlighted row.
    pub selected: usize,
}

/// Everything the view needs besides [`State`].
#[derive(Debug)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent flags (nick list, mouse, focus, a pending prefix) with nothing to merge"
)]
pub struct Ui {
    /// The buffer on screen.
    pub active: Option<BufferId>,
    /// Lines scrolled back from the bottom, per buffer.
    scroll: HashMap<BufferId, usize>,
    /// Lines each buffer had at the last update, to keep scrolled views still.
    totals: HashMap<BufferId, u64>,
    /// The input line.
    pub input: InputLine,
    /// Show the nick list in channels.
    pub show_nicklist: bool,
    /// Mouse capture is on.
    pub mouse: bool,
    /// `strftime` format for timestamps.
    pub time_format: String,
    /// Time zone for timestamps.
    pub tz: TimeZone,
    /// The current time, refreshed by the run loop (fixed in tests).
    pub now: Timestamp,
    /// A buffer to switch to once it exists (after `/join`).
    pub pending_switch: Option<(NetworkId, String)>,
    /// Where the buffer list was drawn, and which buffer each row shows.
    pub buffer_list: (Rect, Vec<BufferId>),
    /// Height of the scrollback area at the last draw, for paging.
    pub page: usize,
    /// The config as last loaded or saved.
    pub config: Config,
    /// The config file, for commands that change it. `None` in tests.
    pub config_file: Option<ConfigFile>,
    /// Messages shown when there is no buffer yet (before any network).
    pub status: Vec<(LineKind, String)>,
    /// Key bindings.
    pub keymap: Keymap,
    /// Tab completion in progress.
    pub completion: Option<Completion>,
    /// The buffer switcher, while open.
    pub switcher: Option<Switcher>,
    /// Lines of a multi-line paste waiting for confirmation.
    pub paste: Option<Vec<String>>,
    /// Digits typed after WeeChat's Alt+J.
    pub jump: Option<String>,
    /// WeeChat's Ctrl+C was pressed; the next letter picks a format code.
    pub format_prefix: bool,
    /// How nicks are coloured.
    pub nick_colours: crate::theme::NickColours,
    /// The terminal has focus (from focus events; assumed until told).
    pub focused: bool,
    back: Vec<BufferId>,
    forward: Vec<BufferId>,
}

impl Ui {
    /// Creates UI state showing the first buffer.
    pub fn new(config: &UiConfig, state: &State) -> Self {
        let mut ui = Self {
            active: None,
            scroll: HashMap::new(),
            totals: HashMap::new(),
            input: InputLine::default(),
            show_nicklist: config.nicklist,
            mouse: config.mouse,
            time_format: config.timestamp_format.clone(),
            tz: TimeZone::system(),
            now: Timestamp::now(),
            pending_switch: None,
            buffer_list: (Rect::default(), Vec::new()),
            page: 20,
            config: Config::default(),
            config_file: None,
            status: Vec::new(),
            keymap: Keymap::default(),
            completion: None,
            switcher: None,
            paste: None,
            jump: None,
            format_prefix: false,
            focused: true,
            nick_colours: crate::theme::NickColours::new(
                config.nick_colours,
                &std::collections::BTreeMap::new(),
            ),
            back: Vec::new(),
            forward: Vec::new(),
        };
        ui.active = ui.order(state).first().copied();
        ui
    }

    /// Buffers in display order: each network's server buffer, then its
    /// channels and queries in the order they were opened.
    pub fn order(&self, state: &State) -> Vec<BufferId> {
        let mut out = Vec::new();
        for net in state.networks() {
            out.push(net.server_buffer);
            out.extend(
                state
                    .buffers()
                    .filter(|b| b.network == net.id && b.id != net.server_buffer)
                    .map(|b| b.id),
            );
        }
        out
    }

    /// The active buffer's network.
    pub fn network(&self, state: &State) -> Option<NetworkId> {
        self.active
            .and_then(|id| state.buffer(id))
            .map(|b| b.network)
    }

    /// Switches to a buffer, remembering the last one for going back.
    pub fn switch(&mut self, id: BufferId) {
        if let Some(current) = self.active.filter(|&c| c != id) {
            if self.back.len() >= HISTORY {
                self.back.remove(0);
            }
            self.back.push(current);
            self.forward.clear();
        }
        self.active = Some(id);
    }

    /// Goes back to the buffer shown before (`back` and `forward` skip buffers
    /// that have since closed).
    pub fn go_back(&mut self, state: &State) {
        while let Some(id) = self.back.pop() {
            if state.buffer(id).is_some() && Some(id) != self.active {
                if let Some(current) = self.active {
                    self.forward.push(current);
                }
                self.active = Some(id);
                return;
            }
        }
    }

    /// Undoes [`Ui::go_back`].
    pub fn go_forward(&mut self, state: &State) {
        while let Some(id) = self.forward.pop() {
            if state.buffer(id).is_some() && Some(id) != self.active {
                if let Some(current) = self.active {
                    self.back.push(current);
                }
                self.active = Some(id);
                return;
            }
        }
    }

    /// The switcher's matches: buffers ranked by how well their name (or
    /// network) matches the query, with their list number.
    pub fn switcher_matches(&self, state: &State) -> Vec<(usize, BufferId, String)> {
        let query = self.switcher.as_ref().map_or("", |s| s.query.as_str());
        let mut scored: Vec<(i64, usize, BufferId, String)> = self
            .order(state)
            .into_iter()
            .enumerate()
            .filter_map(|(i, id)| {
                let buffer = state.buffer(id)?;
                let net = state.network(buffer.network)?;
                let label = if buffer.id == net.server_buffer {
                    net.name.clone()
                } else {
                    format!("{} ({})", buffer.name, net.name)
                };
                let score = fuzzy_score(query, &buffer.name)
                    .max(fuzzy_score(query, &label).map(|s| s - 5))?;
                // Unread activity breaks ties.
                let bonus = i64::from(buffer.activity as u8);
                Some((score * 4 + bonus, i + 1, id, label))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        scored
            .into_iter()
            .map(|(_, n, id, label)| (n, id, label))
            .collect()
    }

    /// Switches to the buffer at a 1-based position in the list.
    pub fn switch_to_number(&mut self, state: &State, number: usize) {
        if let Some(id) = number
            .checked_sub(1)
            .and_then(|i| self.order(state).get(i).copied())
        {
            self.switch(id);
        }
    }

    /// Moves to the next (or previous) buffer, wrapping around.
    pub fn cycle(&mut self, state: &State, forward: bool) {
        let order = self.order(state);
        if order.is_empty() {
            return;
        }
        let current = self
            .active
            .and_then(|a| order.iter().position(|&b| b == a))
            .unwrap_or(0);
        let next = if forward {
            (current + 1) % order.len()
        } else {
            (current + order.len() - 1) % order.len()
        };
        if let Some(&id) = order.get(next) {
            self.switch(id);
        }
    }

    /// Jumps to the buffer with the most interesting unread lines (Alt+A).
    pub fn next_active(&mut self, state: &State) {
        let order = self.order(state);
        let best = order
            .iter()
            .filter(|&&id| Some(id) != self.active)
            .filter_map(|&id| state.buffer(id).map(|b| (b.activity, id)))
            .filter(|(activity, _)| *activity > irsty_core::state::Activity::None)
            // The highest activity wins; among equals, the earliest in the list.
            .max_by_key(|&(activity, id)| {
                (
                    activity,
                    std::cmp::Reverse(order.iter().position(|&b| b == id)),
                )
            });
        if let Some((_, id)) = best {
            self.switch(id);
        }
    }

    /// Lines scrolled back in the active buffer.
    pub fn offset(&self) -> usize {
        self.active
            .and_then(|id| self.scroll.get(&id))
            .copied()
            .unwrap_or(0)
    }

    /// Scrolls the active buffer back (positive) or forward (negative) by lines.
    pub fn scroll_by(&mut self, state: &State, delta: isize) {
        let Some(id) = self.active else {
            return;
        };
        let max = state
            .buffer(id)
            .map_or(0, |b| b.lines().len().saturating_sub(1));
        let current = self.scroll.get(&id).copied().unwrap_or(0);
        let next = current.saturating_add_signed(delta).min(max);
        if next == 0 {
            self.scroll.remove(&id);
        } else {
            self.scroll.insert(id, next);
        }
    }

    /// Whether the oldest line of the active buffer is on screen, so scrolling
    /// further back should fetch older history.
    pub fn at_oldest(&self, state: &State) -> bool {
        self.active
            .and_then(|id| state.buffer(id))
            .is_some_and(|b| self.offset() + self.page >= b.lines().len())
    }

    /// Scrolls one page back or forward.
    pub fn page(&mut self, state: &State, back: bool) {
        let page = isize::try_from(self.page.saturating_sub(2).max(1)).unwrap_or(1);
        self.scroll_by(state, if back { page } else { -page });
    }

    /// Updates after the client applied events: keeps scrolled-back views still
    /// as lines arrive, follows a pending `/join`, and repairs the active buffer
    /// if it was closed.
    pub fn on_update(&mut self, state: &State) {
        for buffer in state.buffers() {
            let before = self
                .totals
                .insert(buffer.id, buffer.total)
                .unwrap_or(buffer.total);
            let added = usize::try_from(buffer.total.saturating_sub(before)).unwrap_or(usize::MAX);
            if added > 0
                && let Some(offset) = self.scroll.get_mut(&buffer.id)
            {
                *offset = offset
                    .saturating_add(added)
                    .min(buffer.lines().len().saturating_sub(1));
            }
        }
        if let Some((net, name)) = &self.pending_switch
            && let Some(id) = state.find_buffer(*net, name)
        {
            self.switch(id);
            self.pending_switch = None;
        }
        if self.active.and_then(|id| state.buffer(id)).is_none() {
            self.active = self.order(state).first().copied();
        }
        self.totals.retain(|id, _| state.buffer(*id).is_some());
        self.scroll.retain(|id, _| state.buffer(*id).is_some());
    }

    /// The channel or query name messages typed now go to, if any.
    pub fn target<'a>(&self, state: &'a State) -> Option<(NetworkId, &'a str)> {
        let buffer = state.buffer(self.active?)?;
        (buffer.kind != BufferKind::Server).then_some((buffer.network, buffer.name.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use irsty_core::event::NetEvent;
    use irsty_proto::Message;

    fn state() -> State {
        let mut s = State::new(1000);
        s.add_network(NetworkId(0), "one", "me", Vec::new());
        s.add_network(NetworkId(1), "two", "me", Vec::new());
        for line in [":me!u@h JOIN #b", ":x!u@h PRIVMSG #b :hi"] {
            s.apply(
                NetworkId(1),
                NetEvent::Message(Message::parse(line).unwrap()),
            );
        }
        s.apply(
            NetworkId(0),
            NetEvent::Message(Message::parse(":me!u@h JOIN #a").unwrap()),
        );
        s
    }

    /// Scrolling back fetches older history only once the oldest line is on
    /// screen.
    #[test]
    fn knows_when_the_oldest_line_is_on_screen() {
        let mut s = State::new(1000);
        s.add_network(NetworkId(0), "one", "me", Vec::new());
        s.apply(
            NetworkId(0),
            NetEvent::Message(Message::parse(":me!u@h JOIN #a").unwrap()),
        );
        for i in 0..50 {
            let line = format!(":x!u@h PRIVMSG #a :{i}");
            s.apply(
                NetworkId(0),
                NetEvent::Message(Message::parse(&line).unwrap()),
            );
        }
        let mut ui = Ui::new(&UiConfig::default(), &s);
        ui.switch_to_number(&s, 2);
        ui.page = 20;
        assert!(!ui.at_oldest(&s), "52 lines don't fit a 20-line page");
        ui.scroll_by(&s, 20);
        assert!(!ui.at_oldest(&s));
        ui.scroll_by(&s, 20);
        assert!(ui.at_oldest(&s));
    }

    #[test]
    fn orders_by_network_and_navigates() {
        let s = state();
        let mut ui = Ui::new(&UiConfig::default(), &s);
        let names: Vec<_> = ui
            .order(&s)
            .iter()
            .map(|&id| s.buffer(id).unwrap().name.clone())
            .collect();
        assert_eq!(names, ["one", "#a", "two", "#b"]);
        assert_eq!(ui.active, Some(ui.order(&s)[0]));
        ui.cycle(&s, false);
        assert_eq!(ui.active, Some(ui.order(&s)[3]));
        ui.cycle(&s, true);
        ui.switch_to_number(&s, 3);
        assert_eq!(ui.target(&s), None, "server buffers have no target");
        ui.switch_to_number(&s, 2);
        assert_eq!(ui.target(&s).map(|t| t.1), Some("#a"));
        ui.switch_to_number(&s, 99);
        assert_eq!(ui.target(&s).map(|t| t.1), Some("#a"));
        ui.next_active(&s);
        assert_eq!(
            ui.target(&s).map(|t| t.1),
            Some("#b"),
            "messages beat events"
        );
    }

    #[test]
    fn scrolling_stays_put_as_lines_arrive() {
        let mut s = state();
        let mut ui = Ui::new(&UiConfig::default(), &s);
        let b = s.find_buffer(NetworkId(1), "#b").unwrap();
        for i in 0..10 {
            s.apply(
                NetworkId(1),
                NetEvent::Message(Message::parse(&format!(":x!u@h PRIVMSG #b :{i}")).unwrap()),
            );
        }
        ui.switch(b);
        ui.on_update(&s);
        ui.scroll_by(&s, 3);
        assert_eq!(ui.offset(), 3);
        s.apply(
            NetworkId(1),
            NetEvent::Message(Message::parse(":x!u@h PRIVMSG #b :new").unwrap()),
        );
        ui.on_update(&s);
        assert_eq!(ui.offset(), 4);
        ui.scroll_by(&s, 1000);
        assert_eq!(ui.offset(), s.buffer(b).unwrap().lines().len() - 1);
        ui.page = 5;
        ui.page(&s, false);
        assert_eq!(ui.offset(), s.buffer(b).unwrap().lines().len() - 4);
        ui.scroll_by(&s, -1000);
        assert_eq!(ui.offset(), 0);
    }

    #[test]
    fn follows_joins_and_closed_buffers() {
        let mut s = state();
        let mut ui = Ui::new(&UiConfig::default(), &s);
        ui.pending_switch = Some((NetworkId(0), "#new".into()));
        ui.on_update(&s);
        assert!(ui.pending_switch.is_some());
        s.apply(
            NetworkId(0),
            NetEvent::Message(Message::parse(":me!u@h JOIN #new").unwrap()),
        );
        ui.on_update(&s);
        assert_eq!(ui.target(&s).map(|t| t.1), Some("#new"));
        let active = ui.active.unwrap();
        assert!(s.remove_buffer(active));
        ui.on_update(&s);
        assert_eq!(ui.active, Some(ui.order(&s)[0]));
    }
}
