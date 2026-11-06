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
use crate::panes::{Direction, Pane, Panes, Place};

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
    /// Styles (`[theme]`).
    pub theme: crate::theme::Theme,
    /// How many colours the terminal shows; the view fits each frame to it.
    pub depth: crate::theme::Depth,
    /// The terminal has focus (from focus events; assumed until told).
    pub focused: bool,
    /// Split windows. The focused pane's buffer is [`Ui::active`].
    pub panes: Panes,
    /// Where each pane was drawn, for the mouse.
    pub pane_areas: Vec<(usize, Rect)>,
    /// The focused buffer's unread marker: the number (see
    /// [`irsty_core::state::Effect::Line`]) of the last line read before it
    /// was shown, or `u64::MAX` if nothing was unread then. Set by the view
    /// when the buffer comes on screen.
    pub marker: Option<(BufferId, u64)>,
    /// Scroll to the first unread line at the next draw.
    pub jump_unread: bool,
    /// Loaded scripts.
    pub scripts: crate::script::Scripts,
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
            nick_colours: crate::theme::NickColours::default(),
            theme: crate::theme::Theme::default(),
            depth: crate::theme::Depth::Truecolor,
            panes: Panes::default(),
            pane_areas: Vec::new(),
            marker: None,
            jump_unread: false,
            scripts: crate::script::Scripts::default(),
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

    /// Takes the UI's settings from a config: the nick list, timestamps,
    /// keys, theme, nick colours and colour depth (from the environment
    /// too). Returns anything wrong with the keys or the theme, which are
    /// skipped. The layout and mouse are applied separately.
    pub fn apply_config(&mut self, config: &Config) -> Vec<String> {
        self.show_nicklist = config.ui.nicklist;
        self.time_format.clone_from(&config.ui.timestamp_format);
        let (keymap, mut problems) = Keymap::new(&config.ui.keymap, &config.keys);
        self.keymap = keymap;
        let (theme, theme_problems) =
            crate::theme::Theme::from_config(&config.theme, config.ui.accessible);
        problems.extend(theme_problems);
        self.nick_colours = crate::theme::NickColours::new(
            config.ui.nick_colours,
            &config.nick_colours,
            &theme.nicks,
        );
        self.theme = theme;
        self.depth = crate::theme::Depth::from_env(config.theme.colours);
        self.config = config.clone();
        problems
    }

    /// Lines scrolled back in the active buffer.
    pub fn offset(&self) -> usize {
        self.active.map_or(0, |id| self.offset_of(id))
    }

    /// Lines scrolled back in a buffer.
    pub fn offset_of(&self, id: BufferId) -> usize {
        self.scroll.get(&id).copied().unwrap_or(0)
    }

    /// Scrolls a buffer to `offset` lines back from the newest.
    pub fn scroll_to(&mut self, id: BufferId, offset: usize) {
        if offset == 0 {
            self.scroll.remove(&id);
        } else {
            self.scroll.insert(id, offset);
        }
    }

    /// Scrolls the active buffer back (positive) or forward (negative) by lines.
    pub fn scroll_by(&mut self, state: &State, delta: isize) {
        if let Some(id) = self.active {
            self.scroll_buffer(state, id, delta);
        }
    }

    /// Scrolls a buffer back (positive) or forward (negative) by lines.
    pub fn scroll_buffer(&mut self, state: &State, id: BufferId, delta: isize) {
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
        self.resolve_panes(state);
        self.totals.retain(|id, _| state.buffer(*id).is_some());
        self.scroll.retain(|id, _| state.buffer(*id).is_some());
    }

    /// The buffer a pane shows.
    pub fn pane_buffer(&self, pane: usize) -> Option<BufferId> {
        if pane == self.panes.focus() {
            self.active
        } else {
            self.panes.get(pane)?.buffer
        }
    }

    /// The buffers on screen, the focused pane's first, each once.
    pub fn visible(&self) -> Vec<BufferId> {
        let mut out: Vec<BufferId> = self.active.into_iter().collect();
        for id in self.panes.ids() {
            if let Some(b) = self.pane_buffer(id)
                && !out.contains(&b)
            {
                out.push(b);
            }
        }
        out
    }

    /// Keeps the focused pane's record of its buffer up to date before focus
    /// moves.
    fn store_focus(&mut self) {
        let active = self.active;
        let focus = self.panes.focus();
        if let Some(pane) = self.panes.get_mut(focus) {
            pane.buffer = active;
        }
    }

    fn take_focus(&mut self) {
        let focus = self.panes.focus();
        if let Some(pane) = self.panes.get(focus) {
            self.active = pane.buffer.or(self.active);
        }
    }

    /// Splits the focused pane; the new pane shows the same buffer and takes
    /// focus. Returns false at the most panes.
    pub fn split(&mut self, direction: Direction) -> bool {
        self.store_focus();
        self.panes.split(direction, self.active).is_some()
    }

    /// Closes the focused pane. Returns false if it is the only one.
    pub fn close_pane(&mut self) -> bool {
        self.store_focus();
        let closed = self.panes.close();
        self.take_focus();
        closed
    }

    /// Keeps only the focused pane.
    pub fn only_pane(&mut self) {
        self.panes.only();
    }

    /// Moves focus to the next or previous pane.
    pub fn cycle_pane(&mut self, forward: bool) {
        self.store_focus();
        self.panes.cycle(forward);
        self.take_focus();
    }

    /// Focuses a pane.
    pub fn focus_pane(&mut self, pane: usize) {
        if pane != self.panes.focus() {
            self.store_focus();
            if self.panes.set_focus(pane) {
                self.take_focus();
            }
        }
    }

    /// The pane at a screen position, if any.
    pub fn pane_at(&self, x: u16, y: u16) -> Option<usize> {
        self.pane_areas
            .iter()
            .find(|(_, r)| r.contains(ratatui::layout::Position::new(x, y)))
            .map(|&(id, _)| id)
    }

    /// The layout as `ui.layout` text.
    pub fn layout_spec(&self, state: &State) -> String {
        let mut panes = self.panes.clone();
        if let Some(pane) = panes.get_mut(panes.focus()) {
            pane.buffer = self.active;
        }
        panes.to_spec(|pane: &Pane| {
            let buffer = state.buffer(pane.buffer?)?;
            let network = state.network(buffer.network)?;
            Some(Place {
                network: network.name.clone(),
                buffer: (buffer.kind != BufferKind::Server).then(|| buffer.name.clone()),
            })
        })
    }

    /// Replaces the panes with a saved layout. Panes show their buffers as
    /// soon as they exist; until then, the focused one keeps what it shows.
    pub fn apply_layout(&mut self, spec: &str, state: &State) -> Result<(), String> {
        let panes = Panes::from_spec(spec)?;
        self.panes = panes;
        self.resolve_panes(state);
        Ok(())
    }

    /// Fills panes waiting for buffers that now exist, and gives panes whose
    /// buffer closed the first buffer instead.
    fn resolve_panes(&mut self, state: &State) {
        let focus = self.panes.focus();
        let first = self.order(state).first().copied();
        for id in self.panes.ids() {
            let Some(pane) = self.panes.get_mut(id) else {
                continue;
            };
            if let Some(found) = pane.want.as_ref().and_then(|p| find_place(state, p)) {
                pane.want = None;
                pane.buffer = Some(found);
                if id == focus {
                    self.active = Some(found);
                }
            } else if id != focus
                && pane.want.is_none()
                && pane.buffer.and_then(|b| state.buffer(b)).is_none()
            {
                pane.buffer = first;
            }
        }
    }

    /// The channel or query name messages typed now go to, if any.
    pub fn target<'a>(&self, state: &'a State) -> Option<(NetworkId, &'a str)> {
        let buffer = state.buffer(self.active?)?;
        (buffer.kind != BufferKind::Server).then_some((buffer.network, buffer.name.as_str()))
    }
}

/// The buffer a saved pane names, if it exists.
fn find_place(state: &State, place: &Place) -> Option<BufferId> {
    place.readings().into_iter().find_map(|(network, buffer)| {
        let net = state.networks().find(|n| n.name == network)?;
        match buffer {
            None => Some(net.server_buffer),
            Some(name) => state.find_buffer(net.id, &name),
        }
    })
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

    #[test]
    fn panes_keep_their_own_buffers() {
        let s = state();
        let a = s.find_buffer(NetworkId(0), "#a").unwrap();
        let b = s.find_buffer(NetworkId(1), "#b").unwrap();
        let mut ui = Ui::new(&UiConfig::default(), &s);
        ui.switch(a);
        assert!(ui.split(Direction::Stacked));
        // The new pane shows the same buffer and has focus.
        assert_eq!(ui.panes.focus(), 1);
        assert_eq!(ui.active, Some(a));
        ui.switch(b);
        assert_eq!(ui.visible(), [b, a]);
        ui.cycle_pane(true);
        assert_eq!(ui.active, Some(a), "the first pane kept #a");
        ui.cycle_pane(false);
        assert_eq!(ui.active, Some(b));
        assert!(ui.close_pane());
        assert_eq!(ui.active, Some(a));
        assert!(!ui.close_pane(), "the last pane stays");
    }

    #[test]
    fn closed_buffers_leave_panes_and_layouts_fill_in() {
        let mut s = state();
        let a = s.find_buffer(NetworkId(0), "#a").unwrap();
        let b = s.find_buffer(NetworkId(1), "#b").unwrap();
        let mut ui = Ui::new(&UiConfig::default(), &s);
        ui.switch(a);
        ui.split(Direction::SideBySide);
        ui.switch(b);
        ui.focus_pane(0);
        // #b closes while shown in the unfocused pane.
        s.remove_buffer(b);
        ui.on_update(&s);
        assert!(ui.visible().iter().all(|&id| s.buffer(id).is_some()));

        // A saved layout names buffers that don't exist yet.
        ui.apply_layout("v(one/#a,two/#later)", &s).unwrap();
        assert_eq!(ui.active, Some(a));
        assert_eq!(ui.pane_buffer(1), None);
        s.apply(
            NetworkId(1),
            NetEvent::Message(Message::parse(":me!u@h JOIN #later").unwrap()),
        );
        ui.on_update(&s);
        let later = s.find_buffer(NetworkId(1), "#later").unwrap();
        assert_eq!(ui.pane_buffer(1), Some(later));
        assert_eq!(ui.layout_spec(&s), "v(one/#a,two/#later)");
        assert!(ui.apply_layout("v(oops", &s).is_err());
    }
}
