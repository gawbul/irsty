//! UI state that is not client state: which buffer is shown, scroll positions,
//! the input line, and layout facts needed to handle the mouse.

use std::collections::HashMap;

use irsty_core::config::UiConfig;
use irsty_core::event::NetworkId;
use irsty_core::state::{BufferId, BufferKind, State};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use ratatui::layout::Rect;

use crate::input::InputLine;

/// Everything the view needs besides [`State`].
#[derive(Debug)]
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

    /// Switches to a buffer.
    pub fn switch(&mut self, id: BufferId) {
        self.active = Some(id);
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
        self.active = order.get(next).copied();
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
            self.active = Some(id);
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
