//! Client state and the single update step (ADR 0002).
//!
//! [`State::apply`] turns a network event into state changes and a list of
//! [`Effect`]s. It performs no I/O, so it is tested with plain values.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::time::Duration;

use irsty_proto::casemap::{CaseKey, CaseMapping};
use irsty_proto::ctcp::Ctcp;
use irsty_proto::format::{self, Builder, Style, StyledText};
use irsty_proto::isupport::ModeKind;
use irsty_proto::{ISupport, MAX_BODY_LEN, Message, Source};
use jiff::Timestamp;

use crate::batch::{Batch, Batches, Routed};
use crate::config::{CtcpConfig, HighlightConfig, IgnoreKind, IgnoreRule, PerformLine};
use crate::event::{NetEvent, NetworkId};

/// Identifies a buffer for the lifetime of the process.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BufferId(pub u64);

/// What a buffer holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BufferKind {
    /// Server messages for a network.
    Server,
    /// A channel.
    Channel,
    /// A private conversation.
    Query,
}

/// How interesting a buffer's unread lines are, for the buffer list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Activity {
    /// Nothing new.
    #[default]
    None,
    /// Joins, parts, modes and other events.
    Events,
    /// Messages.
    Messages,
    /// A private message, or a message mentioning our nick.
    Highlight,
}

/// What a line represents, for styling and log formatting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    /// `PRIVMSG`.
    Message,
    /// CTCP `ACTION` (`/me`).
    Action,
    /// `NOTICE`.
    Notice,
    /// Someone joined.
    Join,
    /// Someone left.
    Part,
    /// Someone quit.
    Quit,
    /// Someone was kicked.
    Kick,
    /// Someone changed nick.
    Nick,
    /// A mode change.
    Mode,
    /// A topic change or the topic on join.
    Topic,
    /// Informational: numerics, connection progress.
    Info,
    /// Something went wrong.
    Error,
}

/// Whether a line we sent has reached the server.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Delivery {
    /// Shown as normal. Everything not sent by us, and our own lines when the
    /// server doesn't echo them.
    #[default]
    Delivered,
    /// Sent, waiting for the server's echo (`echo-message`). Not logged yet.
    Pending {
        /// Ties the echo or an error to this line (`labeled-response`).
        label: Option<String>,
    },
    /// The server refused it, or the connection dropped first.
    Failed {
        /// Why, sanitised.
        reason: String,
    },
}

/// One line of scrollback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// When it happened (from `server-time` if available).
    pub time: Timestamp,
    /// What it is.
    pub kind: LineKind,
    /// The sanitised nick or server that sent it, if any.
    pub sender: Option<String>,
    /// The sanitised, styled text.
    pub text: StyledText,
    /// The original text, formatting codes included, for logs.
    pub raw: String,
    /// Whether it reached the server, for our own messages.
    pub delivery: Delivery,
    /// It mentions our nick or a highlight word (set when added).
    pub highlight: bool,
    /// The server's id for the message (`message-tags`), used to avoid
    /// showing a message twice when history overlaps what we have.
    pub msgid: Option<String>,
}

/// A buffer: a window of scrollback.
#[derive(Clone, Debug)]
pub struct Buffer {
    /// Its id.
    pub id: BufferId,
    /// The network it belongs to.
    pub network: NetworkId,
    /// What it holds.
    pub kind: BufferKind,
    /// The channel or nick name as the server knows it (the network name for
    /// server buffers). Sanitise before display.
    pub name: String,
    lines: VecDeque<Line>,
    /// Lines ever added, so a viewer can tell which are new.
    pub total: u64,
    /// Lines added since the buffer was last read.
    pub unread: usize,
    /// The most interesting kind of unread line.
    pub activity: Activity,
    /// The last read time the server knows (`draft/read-marker`).
    pub read_marker: Option<Timestamp>,
    /// Older history has been asked for and hasn't arrived yet.
    pub history_pending: bool,
    /// The server has no older history for this buffer.
    pub history_done: bool,
}

impl Buffer {
    /// The lines currently held, oldest first.
    pub fn lines(&self) -> &VecDeque<Line> {
        &self.lines
    }

    /// The line numbered `seq` (see [`Effect::Line`]), if it is still held.
    pub fn line(&self, seq: u64) -> Option<&Line> {
        let back = usize::try_from(self.total.checked_sub(seq)?).ok()?;
        let index = self.lines.len().checked_sub(back + 1)?;
        self.lines.get(index)
    }
}

/// What we know about a user, from IRCv3 extensions and their messages.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct User {
    /// The nick as sent.
    pub nick: String,
    /// `user@host`, if known.
    pub userhost: Option<String>,
    /// The services account they are logged in to (`account-tag`,
    /// `account-notify`, `extended-join`); `None` if logged out or unknown.
    pub account: Option<String>,
    /// Real name (`extended-join`, `setname`).
    pub realname: Option<String>,
    /// Away message (`away-notify`); `Some` while away.
    pub away: Option<String>,
}

/// Whether a monitored nick is online (`MONITOR`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Watch {
    /// The nick as configured.
    pub nick: String,
    /// `None` until the server says.
    pub online: Option<bool>,
}

/// A channel member.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    /// The nick as sent.
    pub nick: String,
    /// Membership modes, highest rank first (for example `ov`).
    pub modes: String,
}

/// A channel we know about.
#[derive(Clone, Debug)]
pub struct Channel {
    /// The name as sent.
    pub name: String,
    /// Its buffer.
    pub buffer: BufferId,
    /// We are in it.
    pub joined: bool,
    /// We were in it when the connection dropped; rejoin on reconnect.
    pub rejoin: bool,
    /// The topic.
    pub topic: Option<StyledText>,
    /// Who set the topic.
    pub topic_setter: Option<String>,
    members: HashMap<CaseKey, Member>,
    names_pending: bool,
}

impl Channel {
    /// Members, in no particular order.
    pub fn members(&self) -> impl Iterator<Item = &Member> {
        self.members.values()
    }

    /// Looks up a member.
    pub fn member(&self, mapping: CaseMapping, nick: &str) -> Option<&Member> {
        self.members.get(&CaseKey::new(mapping, nick))
    }
}

/// Connection status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Not connected and not trying.
    Disconnected,
    /// Opening the connection.
    Connecting,
    /// Connected; registration in progress.
    Registering,
    /// Registered and usable.
    Registered,
    /// Waiting to reconnect.
    Waiting,
}

/// A network and everything we know about it.
#[derive(Clone, Debug)]
pub struct Network {
    /// Its id.
    pub id: NetworkId,
    /// Name from the config.
    pub name: String,
    /// Our current nick.
    pub nick: String,
    /// Connection status.
    pub status: Status,
    /// Server-advertised features.
    pub isupport: ISupport,
    /// Enabled capabilities.
    pub caps: Vec<String>,
    /// Latest round-trip time.
    pub lag: Option<Duration>,
    /// Account we are logged in as.
    pub account: Option<String>,
    /// Certificate verification is off for the current server.
    pub insecure: bool,
    /// The server buffer.
    pub server_buffer: BufferId,
    /// Channels to join on every registration.
    pub autojoin: Vec<String>,
    /// Raw lines sent on every registration, before autojoin.
    pub perform: Vec<PerformLine>,
    /// Nicks to watch with `MONITOR`.
    pub watches: Vec<Watch>,
    channels: HashMap<CaseKey, Channel>,
    queries: HashMap<CaseKey, BufferId>,
    users: HashMap<CaseKey, User>,
    batches: Batches,
    /// Labels awaiting a response, oldest first, with the buffer of the line
    /// each belongs to.
    labels: VecDeque<(String, BufferId)>,
    next_label: u64,
    awaiting: Vec<(Awaiting, BufferId)>,
    ctcp_sent: VecDeque<Timestamp>,
    /// Buffers waiting for older history, by target.
    history_before: HashMap<CaseKey, BufferId>,
    monitor_sent: bool,
}

impl Network {
    fn key(&self, name: &str) -> CaseKey {
        CaseKey::new(self.isupport.casemapping, name)
    }

    /// Reports whether `nick` is us.
    pub fn is_me(&self, nick: &str) -> bool {
        self.isupport.casemapping.eq(nick, &self.nick)
    }

    /// Looks up a channel.
    pub fn channel(&self, name: &str) -> Option<&Channel> {
        self.channels.get(&self.key(name))
    }

    /// All channels we know about.
    pub fn channels(&self) -> impl Iterator<Item = &Channel> {
        self.channels.values()
    }

    /// Re-keys every map after a casemapping change.
    /// Whether a capability is enabled.
    pub fn has_cap(&self, cap: &str) -> bool {
        self.caps.iter().any(|c| c == cap)
    }

    /// What we know about a user who shares a channel with us.
    pub fn user(&self, nick: &str) -> Option<&User> {
        self.users.get(&self.key(nick))
    }

    /// Forgets a user once they share no channel with us.
    fn forget_if_gone(&mut self, nick: &str) {
        let key = self.key(nick);
        if !self.channels.values().any(|c| c.members.contains_key(&key)) {
            self.users.remove(&key);
        }
    }

    fn rekey(&mut self) {
        let mapping = self.isupport.casemapping;
        self.users = std::mem::take(&mut self.users)
            .into_values()
            .map(|u| (CaseKey::new(mapping, &u.nick), u))
            .collect();
        let channels = std::mem::take(&mut self.channels);
        for (_, mut channel) in channels {
            let members = std::mem::take(&mut channel.members);
            channel.members = members
                .into_values()
                .map(|m| (CaseKey::new(mapping, &m.nick), m))
                .collect();
            self.channels
                .insert(CaseKey::new(mapping, &channel.name), channel);
        }
        // Query keys lose their original spelling, but the buffer keeps it.
        self.queries = std::mem::take(&mut self.queries)
            .into_iter()
            .map(|(key, id)| (CaseKey::new(mapping, key.as_str()), id))
            .collect();
    }
}

/// Something the app loop must do after an update.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Send a message on a network.
    Send(NetworkId, Message),
    /// A line was added to a buffer (log it, show it). The number is the
    /// line's position among every line the buffer has had
    /// ([`Buffer::total`] just after it was added), so the right line is
    /// logged even when more follow in the same update.
    Line(BufferId, u64),
    /// A buffer changed without gaining a loggable line (show it).
    Updated(BufferId),
    /// Log this line, which is already in the buffer (a confirmed echo).
    Log(BufferId, Box<Line>),
    /// Tell the user: a highlight or a private message.
    Notify(Box<Notification>),
    /// A soju bouncer reported one of its networks (`soju.im/bouncer-networks`).
    Bouncer(Box<BouncerNetwork>),
}

/// A network on a soju bouncer, from `BOUNCER NETWORK`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BouncerNetwork {
    /// The connection to the bouncer that reported it.
    pub parent: NetworkId,
    /// The bouncer's id for the network, for `BOUNCER BIND`.
    pub netid: String,
    /// Its attributes (`name`, `state`, `host` and so on), or `None` when it
    /// has been removed. Updates carry only what changed.
    pub attributes: Option<Vec<(String, String)>>,
}

/// Something worth a notification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notification {
    /// Where it happened.
    pub buffer: BufferId,
    /// The network's name.
    pub network: String,
    /// The channel or nick (sanitised).
    pub place: String,
    /// Who said it (sanitised).
    pub sender: String,
    /// What they said, without formatting.
    pub text: String,
    /// A private message rather than a highlight in a channel.
    pub private: bool,
}

/// The result of [`State::own_message`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OwnMessage {
    /// The buffer the line went to.
    pub buffer: Option<BufferId>,
    /// A label to tag the outgoing message with, if `labeled-response` is on.
    pub label: Option<String>,
    /// What to do next.
    pub effects: Vec<Effect>,
}

/// A command whose replies should go to the buffer it was typed in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Awaiting {
    /// `WHOIS <nick>`.
    Whois(String),
    /// `WHOWAS <nick>`.
    Whowas(String),
    /// `WHO <mask>`.
    Who(String),
    /// `NAMES <channel>`, shown as a list rather than just a count.
    Names(String),
    /// `LIST`.
    List,
}

/// CTCP replies allowed per window on one network.
const CTCP_BURST: usize = 3;
/// The CTCP rate-limit window.
const CTCP_WINDOW_SECS: i64 = 10;

/// What to do about a CTCP request.
enum CtcpAnswer {
    /// Reply with this.
    Sent(String),
    /// We would, but too many came in.
    Limited,
    /// We don't answer this one.
    No,
}

/// Most commands awaiting replies; older ones are forgotten.
const MAX_AWAITING: usize = 64;

/// Most labels awaiting a response; older ones are forgotten.
const MAX_LABELS: usize = 256;

fn remember_label(net: &mut Network, label: String, buffer: BufferId) {
    if net.labels.len() >= MAX_LABELS {
        net.labels.pop_front();
    }
    net.labels.push_back((label, buffer));
}

/// All client state.
#[derive(Clone, Debug)]
pub struct State {
    networks: BTreeMap<NetworkId, Network>,
    buffers: BTreeMap<BufferId, Buffer>,
    next_buffer: u64,
    scrollback: usize,
    clock: fn() -> Timestamp,
    ignores: Vec<IgnoreRule>,
    highlight_words: Vec<String>,
    highlight_regex: Vec<regex::Regex>,
    ctcp: CtcpConfig,
    /// Messages to fetch from history at a time; 0 turns it off.
    history_lines: u32,
    /// How lines are being added: live, or replayed from history.
    replay: Replay,
}

/// Whether lines being added are live or replayed history.
#[derive(Clone, Debug, Default)]
enum Replay {
    /// Live: notify, answer CTCP, count as new.
    #[default]
    Live,
    /// History appended after what we have: no notifications or CTCP
    /// replies, and nothing already shown is shown again.
    Append,
    /// Older history: collected here, then put at the top of its buffer.
    Prepend(Vec<(BufferId, Line)>),
}

impl State {
    /// Creates empty state keeping `scrollback` lines per buffer.
    pub fn new(scrollback: usize) -> Self {
        Self {
            networks: BTreeMap::new(),
            buffers: BTreeMap::new(),
            next_buffer: 0,
            scrollback: scrollback.max(1),
            clock: Timestamp::now,
            ignores: Vec::new(),
            highlight_words: Vec::new(),
            highlight_regex: Vec::new(),
            ctcp: CtcpConfig::default(),
            history_lines: 100,
            replay: Replay::Live,
        }
    }

    /// Replaces the clock used for lines without a `server-time` tag, so tests
    /// and replays are deterministic.
    pub fn set_clock(&mut self, clock: fn() -> Timestamp) {
        self.clock = clock;
    }

    /// Adds a network and its server buffer.
    pub fn add_network(
        &mut self,
        id: NetworkId,
        name: &str,
        nick: &str,
        autojoin: Vec<String>,
    ) -> BufferId {
        let server_buffer = self.new_buffer(id, BufferKind::Server, name);
        self.networks.insert(
            id,
            Network {
                id,
                name: name.to_owned(),
                nick: nick.to_owned(),
                status: Status::Disconnected,
                isupport: ISupport::default(),
                caps: Vec::new(),
                lag: None,
                account: None,
                insecure: false,
                server_buffer,
                autojoin,
                perform: Vec::new(),
                watches: Vec::new(),
                channels: HashMap::new(),
                queries: HashMap::new(),
                users: HashMap::new(),
                batches: Batches::default(),
                labels: VecDeque::new(),
                next_label: 0,
                awaiting: Vec::new(),
                ctcp_sent: VecDeque::new(),
                history_before: HashMap::new(),
                monitor_sent: false,
            },
        );
        server_buffer
    }

    /// Sets the channels a network joins after registration.
    pub fn set_autojoin(&mut self, id: NetworkId, autojoin: Vec<String>) {
        if let Some(net) = self.networks.get_mut(&id) {
            net.autojoin = autojoin;
        }
    }

    /// Sets the lines a network sends after registration.
    pub fn set_perform(&mut self, id: NetworkId, perform: Vec<PerformLine>) {
        if let Some(net) = self.networks.get_mut(&id) {
            net.perform = perform;
        }
    }

    /// Sets the ignore rules and highlight words. Regexes that don't compile
    /// are skipped (the config is validated before it gets here).
    pub fn set_rules(&mut self, ignores: Vec<IgnoreRule>, highlight: &HighlightConfig) {
        self.ignores = ignores;
        self.highlight_words.clone_from(&highlight.words);
        self.highlight_regex = highlight
            .regex
            .iter()
            .filter_map(|r| {
                regex::RegexBuilder::new(r)
                    .case_insensitive(true)
                    .build()
                    .ok()
            })
            .collect();
    }

    /// Changes how many lines each buffer keeps, trimming any over the limit.
    pub fn set_scrollback(&mut self, lines: usize) {
        self.scrollback = lines.max(1);
        for buffer in self.buffers.values_mut() {
            while buffer.lines.len() > self.scrollback {
                buffer.lines.pop_front();
            }
        }
    }

    /// Empties a buffer's scrollback (`/clear`).
    pub fn clear_buffer(&mut self, id: BufferId) {
        if let Some(buffer) = self.buffers.get_mut(&id) {
            buffer.lines.clear();
            buffer.unread = 0;
            buffer.activity = Activity::None;
        }
    }

    /// Removes a network and all its buffers.
    pub fn remove_network(&mut self, id: NetworkId) {
        self.networks.remove(&id);
        self.buffers.retain(|_, b| b.network != id);
    }

    /// Notes that replies to a command typed in `buffer` should go there.
    pub fn expect_reply(&mut self, id: NetworkId, awaiting: Awaiting, buffer: BufferId) {
        if let Some(net) = self.networks.get_mut(&id) {
            if net.awaiting.len() >= MAX_AWAITING {
                net.awaiting.remove(0);
            }
            net.awaiting.push((awaiting, buffer));
        }
    }

    /// Whether `source` (`nick!user@host`) is ignored for `kind` in `channel`
    /// (`None` for private messages) on network `id`.
    pub fn is_ignored(
        &self,
        id: NetworkId,
        source: &str,
        channel: Option<&str>,
        kind: IgnoreKind,
    ) -> bool {
        let Some(net) = self.networks.get(&id) else {
            return false;
        };
        // Never hide our own lines.
        if source.is_empty() || net.is_me(Source::parse(source).nick) {
            return false;
        }
        let mapping = net.isupport.casemapping;
        self.ignores.iter().any(|rule| {
            rule.what.contains(&kind)
                && (rule.networks.is_empty()
                    || rule
                        .networks
                        .iter()
                        .any(|n| n.eq_ignore_ascii_case(&net.name)))
                && (rule.channels.is_empty()
                    || channel.is_some_and(|c| rule.channels.iter().any(|r| mapping.eq(r, c))))
                && irsty_proto::mask::matches(&rule.full_mask(), source, mapping)
        })
    }

    /// [`State::is_ignored`] for a message's source.
    fn hides(&self, id: NetworkId, msg: &Message, channel: Option<&str>, kind: IgnoreKind) -> bool {
        msg.source
            .as_deref()
            .is_some_and(|s| self.is_ignored(id, s, channel, kind))
    }

    /// Sets how CTCP requests are answered.
    pub fn set_ctcp(&mut self, ctcp: CtcpConfig) {
        self.ctcp = ctcp;
    }

    /// The reply to a CTCP request, if we answer it and the rate limit allows.
    fn ctcp_reply(&mut self, id: NetworkId, command: &str, params: Option<&str>) -> CtcpAnswer {
        if !self.ctcp.replies {
            return CtcpAnswer::No;
        }
        let clean = |text: &str, max: usize| -> String {
            text.chars()
                .filter(|c| !matches!(c, '\x01' | '\r' | '\n' | '\0'))
                .take(max)
                .collect()
        };
        let body = match command {
            "VERSION" => clean(&self.ctcp.version, 300),
            "PING" => clean(params.unwrap_or_default(), 100),
            "TIME" if self.ctcp.time => (self.clock)()
                .to_zoned(jiff::tz::TimeZone::system())
                .strftime("%a, %d %b %Y %H:%M:%S %z")
                .to_string(),
            "CLIENTINFO" => "ACTION CLIENTINFO PING SOURCE TIME VERSION".into(),
            "SOURCE" => "https://github.com/gawbul/irsty".into(),
            _ => return CtcpAnswer::No,
        };
        // At most CTCP_BURST replies per CTCP_WINDOW_SECS on each network, so
        // a flood of requests can't get us disconnected for flooding.
        let now = (self.clock)();
        let Some(net) = self.networks.get_mut(&id) else {
            return CtcpAnswer::No;
        };
        net.ctcp_sent
            .retain(|&t| now.duration_since(t).as_secs() < CTCP_WINDOW_SECS);
        if net.ctcp_sent.len() >= CTCP_BURST {
            return CtcpAnswer::Limited;
        }
        net.ctcp_sent.push_back(now);
        let params = (!body.is_empty()).then_some(body.as_str());
        CtcpAnswer::Sent(irsty_proto::ctcp::encode(command, params))
    }

    /// The channel name of a buffer, for channel-limited ignore rules.
    fn channel_of(&self, buffer: BufferId) -> Option<&str> {
        self.buffers
            .get(&buffer)
            .filter(|b| b.kind == BufferKind::Channel)
            .map(|b| b.name.as_str())
    }

    /// Whether `text` is a highlight: it mentions our nick, a highlight word,
    /// or matches a highlight regex.
    fn is_highlight(&self, network: NetworkId, text: &str) -> bool {
        let Some(net) = self.networks.get(&network) else {
            return false;
        };
        let mapping = net.isupport.casemapping;
        mentions_nick(text, &net.nick, mapping)
            || self
                .highlight_words
                .iter()
                .any(|w| mentions_nick(text, w, CaseMapping::Ascii))
            || {
                let plain = format::strip_formatting(text);
                self.highlight_regex.iter().any(|r| r.is_match(&plain))
            }
    }

    /// Sets the nicks a network watches with `MONITOR`.
    pub fn set_monitor(&mut self, id: NetworkId, nicks: Vec<String>) {
        if let Some(net) = self.networks.get_mut(&id) {
            net.watches = nicks
                .into_iter()
                .map(|nick| Watch { nick, online: None })
                .collect();
        }
    }

    /// Looks up a network.
    pub fn network(&self, id: NetworkId) -> Option<&Network> {
        self.networks.get(&id)
    }

    /// All networks.
    pub fn networks(&self) -> impl Iterator<Item = &Network> {
        self.networks.values()
    }

    /// Looks up a buffer.
    pub fn buffer(&self, id: BufferId) -> Option<&Buffer> {
        self.buffers.get(&id)
    }

    /// All buffers in creation order.
    pub fn buffers(&self) -> impl Iterator<Item = &Buffer> {
        self.buffers.values()
    }

    /// Adds a client-generated line to a network's server buffer.
    pub fn note(&mut self, id: NetworkId, kind: LineKind, text: &str) -> Vec<Effect> {
        match self.networks.get(&id).map(|n| n.server_buffer) {
            Some(buffer) => self.note_buffer(buffer, kind, text),
            None => Vec::new(),
        }
    }

    /// Adds a client-generated line to any buffer.
    pub fn note_buffer(&mut self, buffer: BufferId, kind: LineKind, text: &str) -> Vec<Effect> {
        let mut effects = Vec::new();
        if self.buffers.contains_key(&buffer) {
            let text = StyledText::plain(text);
            let line = Line {
                time: (self.clock)(),
                kind,
                sender: None,
                raw: text.to_plain(),
                text,
                delivery: Delivery::Delivered,
                highlight: false,
                msgid: None,
            };
            self.push_line(buffer, line, &mut effects);
        }
        effects
    }

    /// Marks a buffer as read.
    pub fn mark_read(&mut self, id: BufferId) {
        if let Some(buffer) = self.buffers.get_mut(&id) {
            buffer.unread = 0;
            buffer.activity = Activity::None;
        }
    }

    /// Records a message we are about to send, in the channel or query buffer
    /// for `target`. It does not count as unread.
    ///
    /// With `echo-message` the line stays pending until the server echoes it,
    /// and with `labeled-response` too, the returned label (to put on the
    /// outgoing message) ties the echo to it.
    pub fn own_message(
        &mut self,
        id: NetworkId,
        target: &str,
        kind: LineKind,
        text: &str,
    ) -> OwnMessage {
        let mut own = OwnMessage::default();
        let Some(net) = self.networks.get_mut(&id) else {
            return own;
        };
        let echo = net.has_cap("echo-message");
        if echo && net.has_cap("labeled-response") {
            net.next_label += 1;
            own.label = Some(format!("irsty{}", net.next_label));
        }
        let nick = format::sanitise_identifier(&net.nick);
        let is_channel = net
            .isupport
            .is_channel(net.isupport.split_statusmsg(target).1);
        let buffer = if is_channel {
            let channel = net.isupport.split_statusmsg(target).1.to_owned();
            self.channel_mut(id, &channel).map(|c| c.buffer)
        } else {
            self.query_buffer(id, target)
        };
        let Some(buffer) = buffer else {
            own.label = None;
            return own;
        };
        let line = Line {
            time: (self.clock)(),
            kind,
            sender: Some(nick),
            text: StyledText::parse(text),
            raw: text.to_owned(),
            delivery: if echo {
                Delivery::Pending {
                    label: own.label.clone(),
                }
            } else {
                Delivery::Delivered
            },
            highlight: false,
            msgid: None,
        };
        self.push_line(buffer, line, &mut own.effects);
        if echo {
            // Logged when the echo confirms it, not now.
            for effect in &mut own.effects {
                if matches!(effect, Effect::Line(b, _) if *b == buffer) {
                    *effect = Effect::Updated(buffer);
                }
            }
            if let (Some(label), Some(net)) = (&own.label, self.networks.get_mut(&id)) {
                remember_label(net, label.clone(), buffer);
            }
        }
        self.mark_read(buffer);
        own.buffer = Some(buffer);
        own
    }

    /// Marks the newest line in `buffer` as failed: the message
    /// [`State::own_message`] just recorded could not be queued.
    pub fn fail_newest(&mut self, buffer: BufferId, reason: &str) {
        if let Some(line) = self
            .buffers
            .get_mut(&buffer)
            .and_then(|b| b.lines.back_mut())
        {
            line.delivery = Delivery::Failed {
                reason: StyledText::plain(reason).to_plain(),
            };
        }
    }

    /// Confirms a pending line from its echo: by label, or else the oldest
    /// pending line of the same kind and text. Returns whether one matched.
    fn confirm_echo(
        &mut self,
        buffer: BufferId,
        (label, msgid): (Option<&str>, Option<&str>),
        kind: LineKind,
        raw: &str,
        time: Timestamp,
        effects: &mut Vec<Effect>,
    ) -> bool {
        let Some(buf) = self.buffers.get_mut(&buffer) else {
            return false;
        };
        let found = buf.lines.iter_mut().find(|l| match &l.delivery {
            Delivery::Pending { label: Some(own) } if label.is_some() => {
                label == Some(own.as_str())
            }
            Delivery::Pending { .. } => label.is_none() && l.kind == kind && l.raw == raw,
            _ => false,
        });
        let Some(line) = found else {
            return false;
        };
        line.delivery = Delivery::Delivered;
        line.time = time;
        line.msgid = msgid.map(str::to_owned);
        let logged = Box::new(line.clone());
        effects.push(Effect::Updated(buffer));
        effects.push(Effect::Log(buffer, logged));
        true
    }

    /// Marks the line a label belongs to as failed.
    fn fail_label(&mut self, id: NetworkId, label: &str, reason: &str, effects: &mut Vec<Effect>) {
        let Some(buffer) = self.label_buffer(id, label) else {
            return;
        };
        if let Some(line) = self.buffers.get_mut(&buffer).and_then(|b| {
            b.lines.iter_mut().find(
                |l| matches!(&l.delivery, Delivery::Pending { label: Some(own) } if own == label),
            )
        }) {
            line.delivery = Delivery::Failed {
                reason: StyledText::plain(reason).to_plain(),
            };
            effects.push(Effect::Updated(buffer));
        }
    }

    fn label_buffer(&self, id: NetworkId, label: &str) -> Option<BufferId> {
        self.networks
            .get(&id)?
            .labels
            .iter()
            .find(|(l, _)| l == label)
            .map(|&(_, b)| b)
    }

    fn forget_label(&mut self, id: NetworkId, label: &str) {
        if let Some(net) = self.networks.get_mut(&id) {
            net.labels.retain(|(l, _)| l != label);
        }
    }

    /// Removes a channel or query buffer. Server buffers stay. Returns whether
    /// anything was removed.
    pub fn remove_buffer(&mut self, id: BufferId) -> bool {
        let Some(buffer) = self.buffers.get(&id) else {
            return false;
        };
        let Some(net) = self.networks.get_mut(&buffer.network) else {
            return false;
        };
        if net.server_buffer == id {
            return false;
        }
        net.channels.retain(|_, c| c.buffer != id);
        net.queries.retain(|_, &mut b| b != id);
        self.buffers.remove(&id).is_some()
    }

    /// The buffer for a channel or query, if it exists.
    pub fn find_buffer(&self, id: NetworkId, name: &str) -> Option<BufferId> {
        let net = self.networks.get(&id)?;
        let key = net.key(name);
        net.channels
            .get(&key)
            .map(|c| c.buffer)
            .or_else(|| net.queries.get(&key).copied())
    }

    fn new_buffer(&mut self, network: NetworkId, kind: BufferKind, name: &str) -> BufferId {
        self.next_buffer += 1;
        let id = BufferId(self.next_buffer);
        self.buffers.insert(
            id,
            Buffer {
                id,
                network,
                kind,
                name: name.to_owned(),
                lines: VecDeque::new(),
                total: 0,
                unread: 0,
                activity: Activity::None,
                read_marker: None,
                history_pending: false,
                history_done: false,
            },
        );
        id
    }

    fn push_line(&mut self, buffer: BufferId, mut line: Line, effects: &mut Vec<Effect>) {
        let limit = self.scrollback;
        let Some(buf) = self.buffers.get(&buffer) else {
            return;
        };
        let replaying = !matches!(self.replay, Replay::Live);
        // History can overlap what is already here; the msgid says so.
        if replaying
            && let Some(id) = &line.msgid
            && buf.lines.iter().any(|l| l.msgid.as_ref() == Some(id))
        {
            return;
        }
        let net = self.networks.get(&buf.network);
        let from_others = line
            .sender
            .as_deref()
            .is_some_and(|s| net.is_some_and(|n| !n.is_me(s)));
        let talk = matches!(
            line.kind,
            LineKind::Message | LineKind::Action | LineKind::Notice
        );
        if talk && from_others {
            line.highlight = self.is_highlight(buf.network, &line.raw);
            let private = buf.kind == BufferKind::Query;
            if (line.highlight || private) && !replaying {
                effects.push(Effect::Notify(Box::new(Notification {
                    buffer,
                    network: net.map(|n| n.name.clone()).unwrap_or_default(),
                    place: format::sanitise_identifier(&buf.name),
                    sender: line.sender.clone().unwrap_or_default(),
                    text: line.text.to_plain(),
                    private,
                })));
            }
        }
        let activity = match line.kind {
            LineKind::Message | LineKind::Action | LineKind::Notice => {
                let mentions = self.is_highlight(buf.network, &line.raw);
                if buf.kind == BufferKind::Query || mentions {
                    Activity::Highlight
                } else {
                    Activity::Messages
                }
            }
            _ => Activity::Events,
        };
        if let Replay::Prepend(collected) = &mut self.replay {
            collected.push((buffer, line));
            return;
        }
        if let Some(buf) = self.buffers.get_mut(&buffer) {
            // Replayed history can arrive after newer live lines: it goes in
            // time order, and is logged directly, since it isn't at the end.
            let mut at = buf
                .lines
                .iter()
                .rposition(|l| l.time <= line.time)
                .map_or(0, |i| i + 1);
            if replaying && at < buf.lines.len() {
                if buf.lines.len() >= limit {
                    // Full: older than everything kept means it isn't kept.
                    if at == 0 {
                        return;
                    }
                    buf.lines.pop_front();
                    at -= 1;
                }
                effects.push(Effect::Log(buffer, Box::new(line.clone())));
                effects.push(Effect::Updated(buffer));
                buf.lines.insert(at, line);
                buf.activity = buf.activity.max(activity);
                return;
            }
            if buf.lines.len() >= limit {
                buf.lines.pop_front();
            }
            buf.lines.push_back(line);
            buf.total += 1;
            buf.unread += 1;
            buf.activity = buf.activity.max(activity);
            effects.push(Effect::Line(buffer, buf.total));
        }
    }

    /// The channel's buffer, creating the channel and buffer if needed.
    fn channel_mut(&mut self, id: NetworkId, name: &str) -> Option<&mut Channel> {
        let key = self.networks.get(&id)?.key(name);
        if !self.networks.get(&id)?.channels.contains_key(&key) {
            let buffer = self.new_buffer(id, BufferKind::Channel, name);
            self.networks.get_mut(&id)?.channels.insert(
                key.clone(),
                Channel {
                    name: name.to_owned(),
                    buffer,
                    joined: false,
                    rejoin: false,
                    topic: None,
                    topic_setter: None,
                    members: HashMap::new(),
                    names_pending: false,
                },
            );
        }
        self.networks.get_mut(&id)?.channels.get_mut(&key)
    }

    /// The query buffer for `nick`, creating it if needed.
    pub fn query_buffer(&mut self, id: NetworkId, nick: &str) -> Option<BufferId> {
        let key = self.networks.get(&id)?.key(nick);
        if let Some(&buffer) = self.networks.get(&id)?.queries.get(&key) {
            return Some(buffer);
        }
        let buffer = self.new_buffer(id, BufferKind::Query, nick);
        self.networks.get_mut(&id)?.queries.insert(key, buffer);
        Some(buffer)
    }

    /// Applies one network event.
    pub fn apply(&mut self, id: NetworkId, event: NetEvent) -> Vec<Effect> {
        let mut effects = Vec::new();
        let Some(server_buffer) = self.networks.get(&id).map(|n| n.server_buffer) else {
            return effects;
        };
        let clock = self.clock;
        let info = |kind, text: StyledText| Line {
            time: clock(),
            kind,
            sender: None,
            raw: text.to_plain(),
            text,
            delivery: Delivery::Delivered,
            highlight: false,
            msgid: None,
        };
        match event {
            NetEvent::Connecting { server } => {
                self.set_status(id, Status::Connecting);
                let text = StyledText::plain(&format!("Connecting to {server}"));
                self.push_line(server_buffer, info(LineKind::Info, text), &mut effects);
            }
            NetEvent::Connected { server, insecure } => {
                self.set_status(id, Status::Registering);
                if let Some(net) = self.networks.get_mut(&id) {
                    net.insecure = insecure;
                }
                let text = StyledText::plain(&format!("Connected to {server}"));
                self.push_line(server_buffer, info(LineKind::Info, text), &mut effects);
                if insecure {
                    let text = StyledText::plain(
                        "WARNING: TLS certificate verification is disabled for this server. \
                         The connection can be intercepted.",
                    );
                    self.push_line(server_buffer, info(LineKind::Error, text), &mut effects);
                }
            }
            NetEvent::Registered { nick } => self.on_registered(id, nick, &mut effects),
            NetEvent::Message(msg) => self.route(id, msg, &mut effects),
            NetEvent::Caps { enabled } => {
                if let Some(net) = self.networks.get_mut(&id) {
                    net.caps = enabled;
                }
            }
            NetEvent::Sasl(result) => {
                let line = match result {
                    Ok(account) => {
                        if let Some(net) = self.networks.get_mut(&id) {
                            net.account.clone_from(&account);
                        }
                        let who = account.as_deref().unwrap_or("your account");
                        info(
                            LineKind::Info,
                            StyledText::plain(&format!("SASL authentication succeeded as {who}")),
                        )
                    }
                    Err(reason) => info(
                        LineKind::Error,
                        StyledText::plain(&format!("SASL authentication failed: {reason}")),
                    ),
                };
                self.push_line(server_buffer, line, &mut effects);
            }
            NetEvent::Lag(lag) => {
                if let Some(net) = self.networks.get_mut(&id) {
                    net.lag = Some(lag);
                }
            }
            NetEvent::Error(error) => {
                self.push_line(
                    server_buffer,
                    info(LineKind::Error, StyledText::plain(&error)),
                    &mut effects,
                );
            }
            NetEvent::Disconnected {
                reason,
                reconnect_in,
            } => {
                self.on_disconnected(id, &reason, reconnect_in, &mut effects);
            }
        }
        effects
    }

    fn set_status(&mut self, id: NetworkId, status: Status) {
        if let Some(net) = self.networks.get_mut(&id) {
            net.status = status;
        }
    }

    fn on_registered(&mut self, id: NetworkId, nick: String, effects: &mut Vec<Effect>) {
        let Some(net) = self.networks.get_mut(&id) else {
            return;
        };
        net.status = Status::Registered;
        net.nick = nick;
        for line in &net.perform {
            // Validated at load, so a parse failure here is only possible for
            // configs built in code; skip such lines.
            if let Ok(msg) = Message::parse(&line.as_str().replace("$nick", &net.nick)) {
                effects.push(Effect::Send(id, msg));
            }
        }
        let mut targets: Vec<String> = net.autojoin.clone();
        for channel in net.channels.values_mut() {
            if std::mem::take(&mut channel.rejoin)
                && !targets
                    .iter()
                    .any(|t| net.isupport.casemapping.eq(t, &channel.name))
            {
                targets.push(channel.name.clone());
            }
        }
        for msg in join_messages(&targets) {
            effects.push(Effect::Send(id, msg));
        }
        self.catch_up(id, effects);
    }

    /// After registering: fetch what we missed in open queries, find private
    /// conversations we don't have open (`TARGETS`), and ask ZNC to play back
    /// only what came after the newest line we have.
    fn catch_up(&mut self, id: NetworkId, effects: &mut Vec<Effect>) {
        if self.history_lines == 0 {
            return;
        }
        let Some(net) = self.networks.get(&id) else {
            return;
        };
        let newest = self.newest_on_network(id);
        if net.has_cap("draft/chathistory") {
            let queries: Vec<BufferId> = net.queries.values().copied().collect();
            for buffer in queries {
                effects.extend(self.history_latest(buffer));
            }
            // Since the newest line we have, or a day back on a first connect.
            let now = (self.clock)();
            let since = newest.unwrap_or_else(|| now - jiff::SignedDuration::from_hours(24));
            let limit = self.history_limit(id).min(50);
            effects.push(Effect::Send(
                id,
                Message::new("CHATHISTORY")
                    .param("TARGETS")
                    .param(history_ref(since))
                    .param(history_ref(now))
                    .param(limit.to_string()),
            ));
        } else if net.has_cap("znc.in/playback") {
            // ZNC takes Unix seconds, with a fraction.
            let since = newest.map_or_else(
                || "0".to_owned(),
                |t| format!("{}.{:03}", t.as_second(), t.subsec_millisecond()),
            );
            effects.push(Effect::Send(
                id,
                Message::new("PRIVMSG")
                    .param("*playback")
                    .param(format!("PLAY * {since}")),
            ));
        }
    }

    /// The newest time of a message in any of a network's buffers.
    fn newest_on_network(&self, id: NetworkId) -> Option<Timestamp> {
        self.buffers
            .values()
            .filter(|b| b.network == id)
            .filter_map(newest_message)
            .max()
    }

    /// How many messages to ask for at once: our setting, capped by the
    /// server's `CHATHISTORY` limit (0 means it has none).
    fn history_limit(&self, id: NetworkId) -> u32 {
        let server = self
            .networks
            .get(&id)
            .and_then(|n| n.isupport.raw.get("CHATHISTORY"))
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|&n| n > 0)
            .unwrap_or(u32::MAX);
        self.history_lines.min(server)
    }

    /// `CHATHISTORY LATEST` for a buffer: everything after its newest message,
    /// or the latest messages if it has none.
    fn history_latest(&self, buffer: BufferId) -> Option<Effect> {
        let buf = self.buffers.get(&buffer)?;
        let net = self.networks.get(&buf.network)?;
        if self.history_lines == 0
            || !net.has_cap("draft/chathistory")
            || buf.kind == BufferKind::Server
        {
            return None;
        }
        let reference = buf
            .lines
            .iter()
            .rev()
            .find(|l| is_settled_message(l))
            .map_or_else(|| "*".to_owned(), |l| self.reference_for(buf.network, l));
        Some(Effect::Send(
            buf.network,
            Message::new("CHATHISTORY")
                .param("LATEST")
                .param(buf.name.as_str())
                .param(reference)
                .param(self.history_limit(buf.network).to_string()),
        ))
    }

    /// How to point at a message in a history request: by `msgid` where the
    /// line has one and the server accepts them (`MSGREFTYPES`, both when it
    /// doesn't say), since several messages can share a timestamp; otherwise
    /// by time.
    fn reference_for(&self, id: NetworkId, line: &Line) -> String {
        let msgid_ok = self
            .networks
            .get(&id)
            .and_then(|n| n.isupport.raw.get("MSGREFTYPES"))
            .is_none_or(|types| types.split(',').any(|t| t == "msgid"));
        match &line.msgid {
            Some(msgid) if msgid_ok => format!("msgid={msgid}"),
            _ => history_ref(line.time),
        }
    }

    /// Asks for history older than the oldest line in a buffer, when the user
    /// scrolls past it. Does nothing while a request is outstanding, once the
    /// server has said there is no more, or without `draft/chathistory`.
    pub fn request_older(&mut self, buffer: BufferId) -> Vec<Effect> {
        let Some(buf) = self.buffers.get(&buffer) else {
            return Vec::new();
        };
        let id = buf.network;
        let usable = self.history_lines > 0
            && buf.kind != BufferKind::Server
            && !buf.history_pending
            && !buf.history_done
            && self
                .networks
                .get(&id)
                .is_some_and(|n| n.has_cap("draft/chathistory"));
        if !usable {
            return Vec::new();
        }
        let reference = buf
            .lines
            .iter()
            .find(|l| is_settled_message(l))
            .map_or_else(
                || history_ref((self.clock)()),
                |l| self.reference_for(id, l),
            );
        let msg = Message::new("CHATHISTORY")
            .param("BEFORE")
            .param(buf.name.as_str())
            .param(reference)
            .param(self.history_limit(id).to_string());
        let key = self.networks.get(&id).map(|n| n.key(&buf.name));
        if let (Some(buf), Some(net), Some(key)) = (
            self.buffers.get_mut(&buffer),
            self.networks.get_mut(&id),
            key,
        ) {
            buf.history_pending = true;
            net.history_before.insert(key, buffer);
        }
        vec![Effect::Send(id, msg)]
    }

    /// A `chathistory` or `znc.in/playback` batch: replayed without
    /// notifications, and put at the top of the buffer if it answers a
    /// request for older history.
    fn on_history_batch(&mut self, id: NetworkId, batch: &Batch, effects: &mut Vec<Effect>) {
        let target = batch.params.first().cloned().unwrap_or_default();
        let older = self.networks.get_mut(&id).and_then(|net| {
            let key = net.key(&target);
            net.history_before.remove(&key)
        });
        self.replay = if older.is_some() {
            Replay::Prepend(Vec::new())
        } else {
            Replay::Append
        };
        for msg in &batch.messages {
            self.on_message(id, msg, None, effects);
        }
        let replay = std::mem::take(&mut self.replay);
        let Some(buffer) = older else {
            return;
        };
        let Replay::Prepend(collected) = replay else {
            return;
        };
        let limit = self.scrollback;
        let Some(buf) = self.buffers.get_mut(&buffer) else {
            return;
        };
        buf.history_pending = false;
        let lines: Vec<Line> = collected
            .into_iter()
            .filter(|(b, _)| *b == buffer)
            .map(|(_, l)| l)
            .collect();
        if lines.is_empty() {
            buf.history_done = true;
        }
        // Keep within scrollback: older lines that don't fit are dropped.
        let room = limit.saturating_sub(buf.lines.len());
        let skip = lines.len().saturating_sub(room);
        for line in lines.into_iter().skip(skip).rev() {
            buf.lines.push_front(line);
        }
        effects.push(Effect::Updated(buffer));
    }

    /// `CHATHISTORY TARGETS`: open and fill queries we don't have yet.
    fn on_history_targets(&mut self, id: NetworkId, batch: &Batch, effects: &mut Vec<Effect>) {
        for msg in &batch.messages {
            let target = msg.params.get(1).map_or("", String::as_str).to_owned();
            let is_channel = self
                .networks
                .get(&id)
                .is_some_and(|n| n.isupport.is_channel(&target));
            if target.is_empty() || is_channel || !msg.command.eq_ignore_ascii_case("CHATHISTORY") {
                continue;
            }
            if self.find_buffer(id, &target).is_none()
                && let Some(buffer) = self.query_buffer(id, &target)
            {
                effects.extend(self.history_latest(buffer));
            }
        }
    }

    /// `MARKREAD <target> timestamp=...`: another client (or the server)
    /// says where we have read up to. A buffer read to its newest line stops
    /// counting as unread.
    fn on_markread(&mut self, id: NetworkId, target: &str, value: &str, effects: &mut Vec<Effect>) {
        let Some(buffer) = self.find_buffer(id, target) else {
            return;
        };
        let Some(time) = value
            .strip_prefix("timestamp=")
            .and_then(|t| t.parse::<Timestamp>().ok())
        else {
            return;
        };
        let Some(buf) = self.buffers.get_mut(&buffer) else {
            return;
        };
        if buf.read_marker.is_some_and(|m| m >= time) {
            return;
        }
        buf.read_marker = Some(time);
        if newest_message(buf).is_none_or(|n| n <= time) && buf.unread > 0 {
            buf.unread = 0;
            buf.activity = Activity::None;
            effects.push(Effect::Updated(buffer));
        }
    }

    /// Tells the server (`draft/read-marker`) that a buffer has been read up
    /// to its newest message, if that is news.
    pub fn sync_read(&mut self, buffer: BufferId) -> Vec<Effect> {
        let Some(buf) = self.buffers.get(&buffer) else {
            return Vec::new();
        };
        let id = buf.network;
        let supported = self
            .networks
            .get(&id)
            .is_some_and(|n| n.has_cap("draft/read-marker"));
        let Some(newest) = newest_message(buf) else {
            return Vec::new();
        };
        if !supported
            || buf.kind == BufferKind::Server
            || buf.read_marker.is_some_and(|m| m >= newest)
        {
            return Vec::new();
        }
        let name = buf.name.clone();
        if let Some(buf) = self.buffers.get_mut(&buffer) {
            buf.read_marker = Some(newest);
        }
        vec![Effect::Send(
            id,
            Message::new("MARKREAD")
                .param(name)
                .param(history_ref(newest)),
        )]
    }

    /// Sets how many messages to fetch from history at a time.
    pub fn set_history(&mut self, lines: u32) {
        self.history_lines = lines;
    }

    fn on_disconnected(
        &mut self,
        id: NetworkId,
        reason: &str,
        reconnect_in: Option<Duration>,
        effects: &mut Vec<Effect>,
    ) {
        let Some(net) = self.networks.get_mut(&id) else {
            return;
        };
        net.status = if reconnect_in.is_some() {
            Status::Waiting
        } else {
            Status::Disconnected
        };
        net.lag = None;
        net.caps.clear();
        net.users.clear();
        net.batches.clear();
        net.labels.clear();
        net.awaiting.clear();
        net.history_before.clear();
        net.monitor_sent = false;
        for watch in &mut net.watches {
            watch.online = None;
        }
        let server_buffer = net.server_buffer;
        // Anything still waiting for its echo never made it.
        let network = net.id;
        for buf in self.buffers.values_mut().filter(|b| b.network == network) {
            let mut changed = false;
            for line in &mut buf.lines {
                if matches!(line.delivery, Delivery::Pending { .. }) {
                    line.delivery = Delivery::Failed {
                        reason: "not delivered: disconnected".into(),
                    };
                    changed = true;
                }
            }
            if changed {
                effects.push(Effect::Updated(buf.id));
            }
        }
        let Some(net) = self.networks.get_mut(&id) else {
            return;
        };
        let mut affected = Vec::new();
        for channel in net.channels.values_mut() {
            if channel.joined {
                channel.joined = false;
                channel.rejoin = true;
                channel.members.clear();
                affected.push(channel.buffer);
            }
        }
        let text = match reconnect_in {
            Some(delay) if delay.is_zero() => format!("Disconnected ({reason}); reconnecting"),
            Some(delay) => format!(
                "Disconnected ({reason}); reconnecting in {}s",
                delay.as_secs()
            ),
            None => format!("Disconnected ({reason})"),
        };
        for buffer in std::iter::once(server_buffer).chain(affected) {
            let text = StyledText::plain(&text);
            let line = Line {
                time: (self.clock)(),
                kind: LineKind::Error,
                sender: None,
                raw: text.to_plain(),
                text,
                delivery: Delivery::Delivered,
                highlight: false,
                msgid: None,
            };
            self.push_line(buffer, line, effects);
        }
    }

    /// Holds batched messages until their batch ends; handles the rest.
    fn route(&mut self, id: NetworkId, msg: Message, effects: &mut Vec<Effect>) {
        let Some(net) = self.networks.get_mut(&id) else {
            return;
        };
        match net.batches.route(msg) {
            Routed::Now(msg) => self.on_message(id, &msg, None, effects),
            Routed::Held => {}
            Routed::Ended(batch) => self.on_batch(id, &batch, effects),
        }
    }

    fn on_batch(&mut self, id: NetworkId, batch: &Batch, effects: &mut Vec<Effect>) {
        let param = |i: usize| batch.params.get(i).map_or("", String::as_str).to_owned();
        match batch.kind.as_str() {
            // One summary line per channel instead of a line per user.
            kind @ ("netsplit" | "netjoin") => {
                let split = kind == "netsplit";
                let mut seen: Vec<(BufferId, Vec<String>)> = Vec::new();
                let mut note = |buffer: BufferId, nick: &str| {
                    let nick = format::sanitise_identifier(nick);
                    match seen.iter_mut().find(|(b, _)| *b == buffer) {
                        Some((_, nicks)) => nicks.push(nick),
                        None => seen.push((buffer, vec![nick])),
                    }
                };
                let mut time = None;
                for msg in &batch.messages {
                    let nick = msg
                        .source
                        .as_deref()
                        .map(|s| Source::parse(s).nick.to_owned())
                        .unwrap_or_default();
                    time = time.or_else(|| msg_time(msg));
                    match msg.command.to_ascii_uppercase().as_str() {
                        "QUIT" if split => {
                            for buffer in self.remove_everywhere(id, &nick) {
                                note(buffer, &nick);
                            }
                            if let Some(net) = self.networks.get_mut(&id) {
                                net.users.remove(&net.key(&nick));
                            }
                        }
                        "JOIN" if !split => {
                            if let Some(buffer) = self.on_join(id, msg) {
                                note(buffer, &nick);
                            }
                        }
                        _ => self.on_message(id, msg, None, effects),
                    }
                }
                let (what, kind) = if split {
                    ("Netsplit", LineKind::Quit)
                } else {
                    ("Netjoin", LineKind::Join)
                };
                let servers =
                    format::sanitise_identifier(&format!("{} <-> {}", param(0), param(1)));
                for (buffer, nicks) in seen {
                    let text = StyledText::plain(&format!(
                        "{what} {servers}: {} ({})",
                        nicks.join(", "),
                        nicks.len()
                    ));
                    let line = Line {
                        time: time.unwrap_or_else(self.clock),
                        kind,
                        sender: None,
                        raw: text.to_plain(),
                        text,
                        delivery: Delivery::Delivered,
                        highlight: false,
                        msgid: None,
                    };
                    self.push_line(buffer, line, effects);
                }
            }
            "chathistory" | "znc.in/playback" => self.on_history_batch(id, batch, effects),
            "draft/chathistory-targets" => self.on_history_targets(id, batch, effects),
            "labeled-response" => {
                for msg in &batch.messages {
                    self.on_message(id, msg, batch.label.as_deref(), effects);
                }
                if let Some(label) = &batch.label {
                    self.forget_label(id, label);
                }
            }
            // Anything else (chathistory arrives in M9): handle in order.
            _ => {
                for msg in &batch.messages {
                    self.on_message(id, msg, None, effects);
                }
            }
        }
    }

    /// Handles a message. `batch_label` is the label of the labeled-response
    /// batch it came in, if any.
    #[allow(
        clippy::too_many_lines,
        reason = "one arm per command reads better than indirection"
    )]
    fn on_message(
        &mut self,
        id: NetworkId,
        msg: &Message,
        batch_label: Option<&str>,
        effects: &mut Vec<Effect>,
    ) {
        let label = msg.tags.get("label").or(batch_label).map(str::to_owned);
        self.learn_from_tags(id, msg);
        let command = msg.command.to_ascii_uppercase();
        if matches!(command.as_str(), "376" | "422") {
            self.monitor_unsupported(id, effects);
        }
        let Some(net) = self.networks.get(&id) else {
            return;
        };
        // A labelled reply goes where the labelled message was typed.
        let reply_buffer = label
            .as_deref()
            .and_then(|l| net.labels.iter().find(|(own, _)| own == l))
            .map_or(net.server_buffer, |&(_, b)| b);
        let server_buffer = net.server_buffer;
        let time = msg_time(msg).unwrap_or_else(self.clock);
        let source = msg.source.as_deref().map(Source::parse);
        let nick = source.map_or("", |s| s.nick).to_owned();
        let shown_nick = format::sanitise_identifier(&nick);
        let param = |i: usize| msg.params.get(i).map_or("", String::as_str);
        let line = |kind, text: StyledText, raw: String, sender: Option<String>| Line {
            time,
            kind,
            sender,
            text,
            raw,
            delivery: Delivery::Delivered,
            highlight: false,
            msgid: None,
        };
        let event = |text: StyledText| {
            let raw = text.to_plain();
            line(LineKind::Info, text, raw, None)
        };

        match command.as_str() {
            "PRIVMSG" | "NOTICE" => self.on_privmsg(id, msg, time, label.as_deref(), effects),
            "ACK" => {
                if let Some(label) = &label {
                    self.forget_label(id, label);
                }
            }
            "AWAY" | "ACCOUNT" | "CHGHOST" | "SETNAME" => {
                let Some(net) = self.networks.get_mut(&id) else {
                    return;
                };
                let key = net.key(&nick);
                let Some(user) = net.users.get_mut(&key) else {
                    return;
                };
                match command.as_str() {
                    "AWAY" => user.away = msg.params.first().cloned(),
                    "ACCOUNT" => user.account = Some(param(0).to_owned()).filter(|a| a != "*"),
                    "CHGHOST" => user.userhost = Some(format!("{}@{}", param(0), param(1))),
                    _ => user.realname = Some(param(0).to_owned()),
                }
            }
            "INVITE" if self.hides(id, msg, Some(param(1)), IgnoreKind::Invites) => {}
            "INVITE" => {
                let (target, channel) = (param(0).to_owned(), param(1).to_owned());
                let to_me = net.is_me(&target);
                let buffer = if to_me {
                    Some(server_buffer)
                } else {
                    self.find_buffer(id, &channel)
                };
                let Some(buffer) = buffer else {
                    return;
                };
                let whom = if to_me {
                    "you".to_owned()
                } else {
                    format::sanitise_identifier(&target)
                };
                let text = Builder::default()
                    .push(&shown_nick, bold())
                    .push(&format!(" invited {whom} to "), Style::default())
                    .push(&format::sanitise_identifier(&channel), bold())
                    .finish();
                self.push_line(buffer, event(text), effects);
            }
            "FAIL" | "WARN" | "NOTE" => {
                // <type> <command> <code> [<context>...] <description>
                let failed = command == "FAIL";
                if failed && let Some(label) = &label {
                    self.fail_label(
                        id,
                        label,
                        param(msg.params.len().saturating_sub(1)),
                        effects,
                    );
                }
                let about = match param(0) {
                    "*" | "" => "Server".to_owned(),
                    other => other.to_owned(),
                };
                let verdict = match command.as_str() {
                    "FAIL" => "failed",
                    "WARN" => "warning",
                    _ => "note",
                };
                let description = msg.params.last().map_or("", String::as_str);
                let raw = format!("{about} {verdict}: {description} ({})", param(1));
                let kind = if failed {
                    LineKind::Error
                } else {
                    LineKind::Info
                };
                self.push_line(
                    reply_buffer,
                    line(kind, StyledText::parse(&raw), raw.clone(), None),
                    effects,
                );
            }
            "730" | "731" => self.on_monitor(id, param(1), command == "730", time, effects),
            "MARKREAD" => self.on_markread(id, param(0), param(1), effects),
            "BOUNCER" if param(0).eq_ignore_ascii_case("NETWORK") => {
                // Only a connection not bound to one of the bouncer's networks
                // (no BOUNCER_NETID) opens the others.
                let unbound = net.has_cap("soju.im/bouncer-networks")
                    && !net.isupport.raw.contains_key("BOUNCER_NETID");
                if unbound && !param(1).is_empty() {
                    let attributes = (param(2) != "*").then(|| {
                        irsty_proto::Tags::from_raw(param(2))
                            .iter()
                            .map(|(k, v)| (k.to_owned(), v.to_owned()))
                            .collect()
                    });
                    effects.push(Effect::Bouncer(Box::new(BouncerNetwork {
                        parent: id,
                        netid: param(1).to_owned(),
                        attributes,
                    })));
                }
            }
            "734" => {
                let text = StyledText::plain(&format!(
                    "The server's MONITOR list is full (limit {}); not watching {}",
                    param(1),
                    param(2)
                ));
                let mut l = event(text);
                l.kind = LineKind::Error;
                self.push_line(server_buffer, l, effects);
            }
            "JOIN" => {
                let channel = param(0).to_owned();
                let host =
                    source.map(|s| format!("{}@{}", s.user.unwrap_or("?"), s.host.unwrap_or("?")));
                let Some(buffer) = self.on_join(id, msg) else {
                    return;
                };
                if self.networks.get(&id).is_some_and(|n| n.is_me(&nick))
                    && matches!(self.replay, Replay::Live)
                {
                    effects.extend(self.history_latest(buffer));
                }
                if self.hides(id, msg, Some(&channel), IgnoreKind::Events) {
                    return;
                }
                let text = Builder::default()
                    .push(&shown_nick, bold())
                    .push(
                        &format!(" ({}) has joined ", host.unwrap_or_default()),
                        Style::default(),
                    )
                    .push(&channel, bold())
                    .finish();
                let mut l = event(text);
                l.kind = LineKind::Join;
                self.push_line(buffer, l, effects);
            }
            "PART" => {
                let channel = param(0).to_owned();
                let reason = param(1);
                let is_me = net.is_me(&nick);
                let mapping = self.mapping(id);
                let Some(chan) = self.channel_mut(id, &channel) else {
                    return;
                };
                if is_me {
                    chan.joined = false;
                    chan.members.clear();
                } else {
                    chan.members.remove(&CaseKey::new(mapping, &nick));
                }
                let buffer = chan.buffer;
                if let Some(net) = self.networks.get_mut(&id) {
                    net.forget_if_gone(&nick);
                }
                if self.hides(id, msg, Some(&channel), IgnoreKind::Events) {
                    return;
                }
                let mut b = Builder::default()
                    .push(&shown_nick, bold())
                    .push(" has left ", Style::default())
                    .push(&channel, bold());
                if !reason.is_empty() {
                    b = append(b.push(" (", Style::default()), &StyledText::parse(reason))
                        .push(")", Style::default());
                }
                let mut l = event(b.finish());
                l.kind = LineKind::Part;
                self.push_line(buffer, l, effects);
            }
            "KICK" => {
                let channel = param(0).to_owned();
                let victim = param(1).to_owned();
                let reason = param(2);
                let mapping = self.mapping(id);
                let victim_is_me = net.is_me(&victim);
                let Some(chan) = self.channel_mut(id, &channel) else {
                    return;
                };
                chan.members.remove(&CaseKey::new(mapping, &victim));
                if victim_is_me {
                    chan.joined = false;
                    chan.members.clear();
                }
                let buffer = chan.buffer;
                if let Some(net) = self.networks.get_mut(&id) {
                    net.forget_if_gone(&victim);
                }
                if self.hides(id, msg, Some(&channel), IgnoreKind::Events) {
                    return;
                }
                let b = Builder::default()
                    .push(&format::sanitise_identifier(&victim), bold())
                    .push(" was kicked from ", Style::default())
                    .push(&channel, bold())
                    .push(" by ", Style::default())
                    .push(&shown_nick, bold())
                    .push(" (", Style::default());
                let mut l = event(
                    append(b, &StyledText::parse(reason))
                        .push(")", Style::default())
                        .finish(),
                );
                l.kind = LineKind::Kick;
                self.push_line(buffer, l, effects);
            }
            "QUIT" => {
                let reason = StyledText::parse(param(0));
                let buffers = self.remove_everywhere(id, &nick);
                if let Some(net) = self.networks.get_mut(&id) {
                    net.users.remove(&net.key(&nick));
                }
                for buffer in buffers {
                    if self.hides(id, msg, self.channel_of(buffer), IgnoreKind::Events) {
                        continue;
                    }
                    let b = Builder::default()
                        .push(&shown_nick, bold())
                        .push(" has quit (", Style::default());
                    let mut l = event(append(b, &reason).push(")", Style::default()).finish());
                    l.kind = LineKind::Quit;
                    self.push_line(buffer, l, effects);
                }
            }
            "NICK" => {
                let source = msg.source.clone().unwrap_or_default();
                self.on_nick(id, &nick, param(0), &source, time, effects);
            }
            "MODE" => self.on_mode(id, msg, &shown_nick, time, effects),
            "TOPIC" => {
                let channel = param(0).to_owned();
                let topic = StyledText::parse(param(1));
                let Some(chan) = self.channel_mut(id, &channel) else {
                    return;
                };
                chan.topic = Some(topic.clone());
                chan.topic_setter = Some(nick.clone());
                let buffer = chan.buffer;
                let b = Builder::default()
                    .push(&shown_nick, bold())
                    .push(" changed the topic of ", Style::default())
                    .push(&channel, bold())
                    .push(" to: ", Style::default());
                let mut l = event(append(b, &topic).finish());
                l.kind = LineKind::Topic;
                self.push_line(buffer, l, effects);
            }
            "005" => {
                let params: Vec<&str> = msg.params.iter().map(String::as_str).collect();
                if let Some(net) = self.networks.get_mut(&id) {
                    let before = net.isupport.casemapping;
                    net.isupport.apply(&params);
                    if net.isupport.casemapping != before {
                        net.rekey();
                    }
                }
                self.send_monitor(id, effects);
            }
            "332" => {
                let channel = param(1).to_owned();
                let topic = StyledText::parse(param(2));
                let Some(chan) = self.channel_mut(id, &channel) else {
                    return;
                };
                chan.topic = Some(topic.clone());
                let buffer = chan.buffer;
                let b = Builder::default()
                    .push("Topic for ", Style::default())
                    .push(&channel, bold())
                    .push(": ", Style::default());
                let mut l = event(append(b, &topic).finish());
                l.kind = LineKind::Topic;
                self.push_line(buffer, l, effects);
            }
            "333" => {
                let channel = param(1).to_owned();
                let setter = Source::parse(param(2)).nick.to_owned();
                if let Some(chan) = self.channel_mut(id, &channel) {
                    chan.topic_setter = Some(setter);
                }
            }
            "353" => {
                let channel = param(2).to_owned();
                let names = param(3).to_owned();
                let isupport = net.isupport.clone();
                let Some(chan) = self.channel_mut(id, &channel) else {
                    return;
                };
                if !chan.names_pending {
                    chan.names_pending = true;
                    chan.members.clear();
                }
                let mut users = Vec::new();
                for entry in names.split(' ').filter(|e| !e.is_empty()) {
                    let (symbols, rest) = isupport.split_prefixes(entry);
                    // userhost-in-names sends nick!user@host.
                    let nick = Source::parse(rest).nick;
                    if nick.is_empty() {
                        continue;
                    }
                    let parsed = Source::parse(rest);
                    let userhost = parsed
                        .user
                        .zip(parsed.host)
                        .map(|(u, h)| format!("{u}@{h}"));
                    let mut modes: Vec<char> = symbols
                        .chars()
                        .filter_map(|s| isupport.mode_for_symbol(s))
                        .collect();
                    modes.sort_by_key(|&m| isupport.mode_rank(m));
                    chan.members.insert(
                        CaseKey::new(isupport.casemapping, nick),
                        Member {
                            nick: nick.to_owned(),
                            modes: modes.into_iter().collect(),
                        },
                    );
                    users.push((nick.to_owned(), userhost));
                }
                if let Some(net) = self.networks.get_mut(&id) {
                    for (nick, userhost) in users {
                        let user = net.users.entry(net.key(&nick)).or_insert_with(|| User {
                            nick: nick.clone(),
                            ..User::default()
                        });
                        if userhost.is_some() {
                            user.userhost = userhost;
                        }
                    }
                }
            }
            "366" => {
                let channel = param(1).to_owned();
                let Some(chan) = self.channel_mut(id, &channel) else {
                    return;
                };
                chan.names_pending = false;
                let count = chan.members.len();
                let buffer = chan.buffer;
                let text = StyledText::plain(&format!("{channel}: {count} users"));
                self.push_line(buffer, event(text), effects);
                let mapping = self.mapping(id);
                let wanted = self.awaited(
                    id,
                    |a| matches!(a, Awaiting::Names(c) if mapping.eq(c, &channel)),
                );
                if let Some((target, index)) = wanted {
                    self.forget_awaiting(id, index);
                    let names = self.names_line(id, &channel);
                    self.push_line(target, event(StyledText::plain(&names)), effects);
                }
            }
            // Handled by the connection task; nothing to show.
            // BOUNCER replies other than NETWORK confirm what we asked for.
            "CAP" | "AUTHENTICATE" | "900" | "903" | "908" | "BOUNCER" => {}
            c if crate::replies::WHOIS.contains(&c)
                || crate::replies::WHOWAS.contains(&c)
                || c == crate::replies::END_OF_WHOIS
                || c == crate::replies::END_OF_WHOWAS =>
            {
                self.on_whois_reply(id, c, msg, reply_buffer, time, effects);
            }
            "401" | "406" if self.awaited_nick(id, param(1)).is_some() => {
                let nick = param(1).to_owned();
                if let Some((buffer, index)) = self.awaited_nick(id, &nick) {
                    self.forget_awaiting(id, index);
                    let text = StyledText::plain(&format!("No such nick: {nick}"));
                    let mut l = event(text);
                    l.kind = LineKind::Error;
                    self.push_line(buffer, l, effects);
                }
            }
            "352" => {
                let nick = param(5).to_owned();
                let (user, host, flags) = (param(2), param(3), param(6));
                if let Some(net) = self.networks.get_mut(&id)
                    && let Some(u) = net.users.get_mut(&net.key(&nick))
                {
                    u.userhost = Some(format!("{user}@{host}"));
                    if flags.starts_with('G') {
                        u.away.get_or_insert_with(String::new);
                    } else {
                        u.away = None;
                    }
                }
                let buffer = self
                    .awaited(id, |a| matches!(a, Awaiting::Who(_)))
                    .map_or(reply_buffer, |(b, _)| b);
                let text = StyledText::parse(&crate::replies::who(&msg.params));
                self.push_line(buffer, event(text), effects);
            }
            "315" => {
                if let Some((_, index)) = self.awaited(id, |a| matches!(a, Awaiting::Who(_))) {
                    self.forget_awaiting(id, index);
                }
            }
            "321" => {}
            "322" => {
                let buffer = self
                    .awaited(id, |a| *a == Awaiting::List)
                    .map_or(reply_buffer, |(b, _)| b);
                let text = StyledText::parse(&crate::replies::list_entry(&msg.params));
                self.push_line(buffer, event(text), effects);
            }
            "323" => {
                let awaited = self.awaited(id, |a| *a == Awaiting::List);
                if let Some((_, index)) = awaited {
                    self.forget_awaiting(id, index);
                }
                let buffer = awaited.map_or(reply_buffer, |(b, _)| b);
                self.push_line(
                    buffer,
                    event(StyledText::plain("End of channel list")),
                    effects,
                );
            }
            c @ ("324" | "329" | "331" | "341" | "367" | "368") => {
                if let Some((channel, text)) = crate::replies::channel_info(c, &msg.params) {
                    let buffer = self.find_buffer(id, &channel).unwrap_or(reply_buffer);
                    self.push_line(buffer, event(StyledText::parse(&text)), effects);
                }
            }
            command => {
                // Numerics and anything else: show in the server buffer.
                // 422 (no MOTD) is numbered like an error but isn't one.
                let is_error =
                    (command.starts_with(['4', '5']) && command != "422") || command == "ERROR";
                let skip = usize::from(command.bytes().all(|b| b.is_ascii_digit()));
                let raw = msg.params.get(skip..).unwrap_or_default().join(" ");
                if is_error && let Some(label) = &label {
                    self.fail_label(
                        id,
                        label,
                        msg.params.last().map_or("", String::as_str),
                        effects,
                    );
                }
                let kind = if is_error {
                    LineKind::Error
                } else {
                    LineKind::Info
                };
                let sender = (!shown_nick.is_empty()).then_some(shown_nick);
                self.push_line(
                    reply_buffer,
                    line(kind, StyledText::parse(&raw), raw.clone(), sender),
                    effects,
                );
            }
        }
    }

    /// Applies a JOIN to channel and user state. Returns the channel's buffer.
    fn on_join(&mut self, id: NetworkId, msg: &Message) -> Option<BufferId> {
        let source = msg.source.as_deref().map(Source::parse)?;
        let nick = source.nick.to_owned();
        let channel = msg.params.first()?.clone();
        let net = self.networks.get(&id)?;
        let is_me = net.is_me(&nick);
        let mapping = net.isupport.casemapping;
        // extended-join: JOIN <channel> <account> :<realname>
        let extended = net.has_cap("extended-join") && msg.params.len() >= 3;
        let chan = self.channel_mut(id, &channel)?;
        if is_me {
            chan.joined = true;
            chan.rejoin = false;
            chan.members.clear();
        }
        // A JOIN for someone already listed (a bouncer replay, or state that
        // drifted) must not wipe their modes.
        chan.members
            .entry(CaseKey::new(mapping, &nick))
            .or_insert_with(|| Member {
                nick: nick.clone(),
                modes: String::new(),
            });
        let buffer = chan.buffer;
        let net = self.networks.get_mut(&id)?;
        let user = net.users.entry(net.key(&nick)).or_insert_with(|| User {
            nick: nick.clone(),
            ..User::default()
        });
        if let (Some(u), Some(h)) = (source.user, source.host) {
            user.userhost = Some(format!("{u}@{h}"));
        }
        if extended {
            let account = msg.params.get(1).map_or("*", String::as_str);
            user.account = (account != "*").then(|| account.to_owned());
            user.realname = msg.params.get(2).cloned();
        }
        Some(buffer)
    }

    /// `account-tag`: any message may say which account its sender uses.
    fn learn_from_tags(&mut self, id: NetworkId, msg: &Message) {
        let Some(account) = msg.tags.get("account") else {
            return;
        };
        let Some(nick) = msg.source.as_deref().map(|s| Source::parse(s).nick) else {
            return;
        };
        let Some(net) = self.networks.get_mut(&id) else {
            return;
        };
        if let Some(user) = net
            .users
            .get_mut(&CaseKey::new(net.isupport.casemapping, nick))
        {
            user.account = (account != "*").then(|| account.to_owned());
        }
    }

    /// Sends `MONITOR +` for the watch list once the server says it supports it.
    fn send_monitor(&mut self, id: NetworkId, effects: &mut Vec<Effect>) {
        let Some(net) = self.networks.get_mut(&id) else {
            return;
        };
        let Some(limit) = net.isupport.monitor else {
            return;
        };
        if net.monitor_sent || net.watches.is_empty() {
            return;
        }
        net.monitor_sent = true;
        let nicks: Vec<&str> = net.watches.iter().map(|w| w.nick.as_str()).collect();
        let (wanted, dropped) = nicks.split_at(limit.unwrap_or(usize::MAX).min(nicks.len()));
        // Keep each line well inside 512 bytes.
        let mut chunk = String::new();
        let mut lines = Vec::new();
        for nick in wanted {
            if !chunk.is_empty() && chunk.len() + nick.len() > 400 {
                lines.push(std::mem::take(&mut chunk));
            }
            if !chunk.is_empty() {
                chunk.push(',');
            }
            chunk.push_str(nick);
        }
        if !chunk.is_empty() {
            lines.push(chunk);
        }
        let dropped = dropped.join(", ");
        let server_buffer = net.server_buffer;
        for line in lines {
            effects.push(Effect::Send(
                id,
                Message::new("MONITOR").param("+").param(line),
            ));
        }
        if !dropped.is_empty() {
            self.note_into(
                server_buffer,
                LineKind::Error,
                &format!("The server's MONITOR limit is too small to watch: {dropped}"),
                effects,
            );
        }
    }

    /// At the end of the MOTD, says so if the server can't watch nicks for us.
    fn monitor_unsupported(&mut self, id: NetworkId, effects: &mut Vec<Effect>) {
        let Some(net) = self.networks.get_mut(&id) else {
            return;
        };
        if net.monitor_sent || net.watches.is_empty() || net.isupport.monitor.is_some() {
            return;
        }
        net.monitor_sent = true;
        let server_buffer = net.server_buffer;
        self.note_into(
            server_buffer,
            LineKind::Error,
            "This server doesn't support MONITOR, so Irsty can't tell you when watched nicks come online",
            effects,
        );
    }

    /// 730 and 731: watched nicks came online or went offline.
    fn on_monitor(
        &mut self,
        id: NetworkId,
        targets: &str,
        online: bool,
        time: Timestamp,
        effects: &mut Vec<Effect>,
    ) {
        let Some(net) = self.networks.get_mut(&id) else {
            return;
        };
        let mapping = net.isupport.casemapping;
        let mut changed = Vec::new();
        for target in targets.split(',').filter(|t| !t.is_empty()) {
            let nick = Source::parse(target).nick;
            let Some(watch) = net.watches.iter_mut().find(|w| mapping.eq(&w.nick, nick)) else {
                continue;
            };
            let was = watch.online.replace(online);
            // Say when someone comes online, and when someone we saw online
            // leaves; an initial "offline" is not news.
            if was != Some(online) && (online || was == Some(true)) {
                changed.push(nick.to_owned());
            }
        }
        for nick in changed {
            let mut buffers = vec![self.networks.get(&id).map(|n| n.server_buffer)];
            buffers.push(
                self.networks
                    .get(&id)
                    .and_then(|n| n.queries.get(&n.key(&nick)).copied()),
            );
            let state = if online { "online" } else { "offline" };
            for buffer in buffers.into_iter().flatten() {
                let text = Builder::default()
                    .push(&format::sanitise_identifier(&nick), bold())
                    .push(&format!(" is {state}"), Style::default())
                    .finish();
                let line = Line {
                    time,
                    kind: LineKind::Info,
                    sender: None,
                    raw: text.to_plain(),
                    text,
                    delivery: Delivery::Delivered,
                    highlight: false,
                    msgid: None,
                };
                self.push_line(buffer, line, effects);
            }
        }
    }

    fn note_into(
        &mut self,
        buffer: BufferId,
        kind: LineKind,
        text: &str,
        effects: &mut Vec<Effect>,
    ) {
        let text = StyledText::plain(text);
        let line = Line {
            time: (self.clock)(),
            kind,
            sender: None,
            raw: text.to_plain(),
            text,
            delivery: Delivery::Delivered,
            highlight: false,
            msgid: None,
        };
        self.push_line(buffer, line, effects);
    }

    /// The first command awaiting replies that `wanted` accepts: its buffer and
    /// its index.
    fn awaited(
        &self,
        id: NetworkId,
        wanted: impl Fn(&Awaiting) -> bool,
    ) -> Option<(BufferId, usize)> {
        let net = self.networks.get(&id)?;
        net.awaiting
            .iter()
            .position(|(a, _)| wanted(a))
            .and_then(|i| net.awaiting.get(i).map(|&(_, b)| (b, i)))
    }

    /// A WHOIS or WHOWAS awaiting replies about `nick`.
    fn awaited_nick(&self, id: NetworkId, nick: &str) -> Option<(BufferId, usize)> {
        let mapping = self.mapping(id);
        self.awaited(
            id,
            |a| matches!(a, Awaiting::Whois(n) | Awaiting::Whowas(n) if mapping.eq(n, nick)),
        )
    }

    fn forget_awaiting(&mut self, id: NetworkId, index: usize) {
        if let Some(net) = self.networks.get_mut(&id)
            && index < net.awaiting.len()
        {
            net.awaiting.remove(index);
        }
    }

    /// `Names on #c: @op +voice user`, ranked as in the nick list.
    fn names_line(&self, id: NetworkId, channel: &str) -> String {
        let Some(net) = self.networks.get(&id) else {
            return String::new();
        };
        let Some(chan) = net.channel(channel) else {
            return String::new();
        };
        let isupport = &net.isupport;
        let mut members: Vec<&Member> = chan.members().collect();
        members.sort_by_cached_key(|m| {
            let rank = m
                .modes
                .chars()
                .next()
                .map_or(usize::MAX, |c| isupport.mode_rank(c));
            (rank, isupport.casemapping.fold(&m.nick).into_owned())
        });
        let names: Vec<String> = members
            .iter()
            .map(|m| {
                let symbol = m
                    .modes
                    .chars()
                    .next()
                    .and_then(|c| isupport.symbol_for_mode(c))
                    .map(String::from)
                    .unwrap_or_default();
                format!("{symbol}{}", format::sanitise_identifier(&m.nick))
            })
            .collect();
        format!("Names on {channel}: {}", names.join(" "))
    }

    /// A WHOIS or WHOWAS reply: shown in the buffer the command came from, and
    /// what it says about the user remembered.
    fn on_whois_reply(
        &mut self,
        id: NetworkId,
        command: &str,
        msg: &Message,
        reply_buffer: BufferId,
        time: Timestamp,
        effects: &mut Vec<Effect>,
    ) {
        let nick = msg.params.get(1).map_or("", String::as_str).to_owned();
        let awaited = self.awaited_nick(id, &nick);
        if let Some(net) = self.networks.get_mut(&id)
            && let Some(user) = net.users.get_mut(&net.key(&nick))
        {
            let p = |i: usize| msg.params.get(i).cloned().unwrap_or_default();
            match command {
                "311" => {
                    user.userhost = Some(format!("{}@{}", p(2), p(3)));
                    user.realname = msg.params.last().cloned();
                }
                "330" => user.account = Some(p(2)),
                "301" => user.away = msg.params.last().cloned(),
                _ => {}
            }
        }
        if command == crate::replies::END_OF_WHOIS || command == crate::replies::END_OF_WHOWAS {
            if let Some((_, index)) = awaited {
                self.forget_awaiting(id, index);
            }
            return;
        }
        let buffer = match awaited {
            Some((buffer, _)) => buffer,
            // 301 outside a WHOIS answers a message to someone away: say so
            // in the conversation with them.
            None if command == "301" => {
                let text = format!(
                    "{nick} is away: {}",
                    msg.params.last().map_or("", String::as_str)
                );
                let buffer = self.find_buffer(id, &nick).unwrap_or(reply_buffer);
                self.note_into(buffer, LineKind::Info, &text, effects);
                return;
            }
            None => reply_buffer,
        };
        if let Some(text) = crate::replies::whois(command, &msg.params) {
            let text = StyledText::parse(&text);
            let line = Line {
                time,
                kind: LineKind::Info,
                sender: None,
                raw: text.to_plain(),
                text,
                delivery: Delivery::Delivered,
                highlight: false,
                msgid: None,
            };
            self.push_line(buffer, line, effects);
        }
    }

    fn mapping(&self, id: NetworkId) -> CaseMapping {
        self.networks
            .get(&id)
            .map(|n| n.isupport.casemapping)
            .unwrap_or_default()
    }

    fn on_privmsg(
        &mut self,
        id: NetworkId,
        msg: &Message,
        time: Timestamp,
        label: Option<&str>,
        effects: &mut Vec<Effect>,
    ) {
        let Some(net) = self.networks.get(&id) else {
            return;
        };
        let source = msg.source.as_deref().map(Source::parse);
        let nick = source.map_or("", |s| s.nick).to_owned();
        let from_server = source.is_none_or(|s| s.is_server());
        let target = msg.params.first().map_or("", String::as_str).to_owned();
        let body = msg.params.get(1).map_or("", String::as_str);
        let notice = msg.command.eq_ignore_ascii_case("NOTICE");
        let (_, channel) = net.isupport.split_statusmsg(&target);
        let is_channel = net.isupport.is_channel(channel);
        let channel = channel.to_owned();
        let from_me = net.is_me(&nick);
        let server_buffer = net.server_buffer;

        let ctcp = Ctcp::parse(body);
        let (kind, raw) = match ctcp {
            Some(c) if c.is_action() => (LineKind::Action, c.params.unwrap_or_default().to_owned()),
            Some(_)
                if self.hides(
                    id,
                    msg,
                    is_channel.then_some(channel.as_str()),
                    IgnoreKind::Ctcps,
                ) =>
            {
                return;
            }
            // Our own CTCP requests and replies coming back (echo-message, a
            // bouncer): the line saying we answered, or /ctcp's own note,
            // already covers them.
            Some(_) if from_me => return,
            Some(c) => {
                let answer = !notice && !nick.is_empty();
                self.on_ctcp(id, &nick, &c, (notice, answer), time, effects);
                return;
            }
            None if notice => (LineKind::Notice, body.to_owned()),
            None => (LineKind::Message, body.to_owned()),
        };
        // Our own messages come back with echo-message or from a bouncer; never
        // show or log a services password we typed.
        let raw = if from_me {
            crate::redact::services_text(&target, &raw)
        } else {
            raw
        };
        let echo = from_me && net.has_cap("echo-message");
        let ignore_kind = match kind {
            LineKind::Notice => IgnoreKind::Notices,
            _ => IgnoreKind::Messages,
        };
        if self.hides(id, msg, is_channel.then_some(channel.as_str()), ignore_kind) {
            return;
        }

        let buffer = if is_channel {
            self.channel_mut(id, &channel).map(|c| c.buffer)
        } else if notice && (from_server || target == "*") {
            Some(server_buffer)
        } else if from_me {
            // Our own message echoed back (echo-message or a bouncer).
            self.query_buffer(id, &target)
        } else {
            self.query_buffer(id, &nick)
        };
        let Some(buffer) = buffer else {
            return;
        };
        let msgid = msg.tags.get("msgid");
        if echo && self.confirm_echo(buffer, (label, msgid), kind, &raw, time, effects) {
            if let Some(label) = label {
                self.forget_label(id, label);
            }
            return;
        }
        let sender = (!nick.is_empty()).then(|| format::sanitise_identifier(&nick));
        let line = Line {
            time,
            kind,
            sender,
            text: StyledText::parse(&raw),
            raw,
            delivery: Delivery::Delivered,
            highlight: false,
            msgid: msg.tags.get("msgid").map(str::to_owned),
        };
        self.push_line(buffer, line, effects);
    }

    /// A CTCP request or reply other than ACTION: answer requests we can, and
    /// note it in the server buffer.
    fn on_ctcp(
        &mut self,
        id: NetworkId,
        nick: &str,
        ctcp: &Ctcp<'_>,
        (notice, answer_it): (bool, bool),
        time: Timestamp,
        effects: &mut Vec<Effect>,
    ) {
        let mut answer = "";
        if answer_it && matches!(self.replay, Replay::Live) {
            let command = ctcp.command.to_ascii_uppercase();
            answer = match self.ctcp_reply(id, &command, ctcp.params) {
                CtcpAnswer::Sent(reply) => {
                    effects.push(Effect::Send(
                        id,
                        Message::new("NOTICE").param(nick).param(reply),
                    ));
                    " (answered)"
                }
                CtcpAnswer::Limited => " (not answered: too many requests)",
                CtcpAnswer::No => "",
            };
        }
        let what = if notice { "reply" } else { "request" };
        let text = StyledText::plain(&format!(
            "CTCP {} {what} from {nick}{}{answer}",
            ctcp.command,
            ctcp.params.map(|p| format!(": {p}")).unwrap_or_default()
        ));
        let Some(server_buffer) = self.networks.get(&id).map(|n| n.server_buffer) else {
            return;
        };
        let line = Line {
            time,
            kind: LineKind::Info,
            sender: None,
            raw: text.to_plain(),
            text,
            delivery: Delivery::Delivered,
            highlight: false,
            msgid: None,
        };
        self.push_line(server_buffer, line, effects);
    }

    /// Removes `nick` from every channel; returns the buffers it was seen in.
    fn remove_everywhere(&mut self, id: NetworkId, nick: &str) -> Vec<BufferId> {
        let Some(net) = self.networks.get_mut(&id) else {
            return Vec::new();
        };
        let key = net.key(nick);
        let mut buffers: Vec<BufferId> = net
            .channels
            .values_mut()
            .filter_map(|c| c.members.remove(&key).map(|_| c.buffer))
            .collect();
        if let Some(&query) = net.queries.get(&key) {
            buffers.push(query);
        }
        buffers
    }

    fn on_nick(
        &mut self,
        id: NetworkId,
        old: &str,
        new: &str,
        source: &str,
        time: Timestamp,
        effects: &mut Vec<Effect>,
    ) {
        let Some(net) = self.networks.get_mut(&id) else {
            return;
        };
        if new.is_empty() {
            return;
        }
        let was_me = net.is_me(old);
        if was_me {
            new.clone_into(&mut net.nick);
        }
        let mapping = net.isupport.casemapping;
        let (old_key, new_key) = (CaseKey::new(mapping, old), CaseKey::new(mapping, new));
        let mut buffers = Vec::new();
        for channel in net.channels.values_mut() {
            if let Some(mut member) = channel.members.remove(&old_key) {
                new.clone_into(&mut member.nick);
                channel.members.insert(new_key.clone(), member);
                buffers.push(channel.buffer);
            }
        }
        if let Some(mut user) = net.users.remove(&old_key) {
            new.clone_into(&mut user.nick);
            net.users.insert(new_key.clone(), user);
        }
        if let Some(query) = net.queries.remove(&old_key) {
            net.queries.insert(new_key, query);
            if let Some(buffer) = self.buffers.get_mut(&query) {
                new.clone_into(&mut buffer.name);
            }
            buffers.push(query);
        }
        if was_me {
            buffers.push(net.server_buffer);
        }
        for buffer in buffers {
            if self.is_ignored(id, source, self.channel_of(buffer), IgnoreKind::Events) {
                continue;
            }
            let text = Builder::default()
                .push(&format::sanitise_identifier(old), bold())
                .push(" is now known as ", Style::default())
                .push(&format::sanitise_identifier(new), bold())
                .finish();
            let line = Line {
                time,
                kind: LineKind::Nick,
                sender: None,
                raw: text.to_plain(),
                text,
                delivery: Delivery::Delivered,
                highlight: false,
                msgid: None,
            };
            self.push_line(buffer, line, effects);
        }
    }

    fn on_mode(
        &mut self,
        id: NetworkId,
        msg: &Message,
        setter: &str,
        time: Timestamp,
        effects: &mut Vec<Effect>,
    ) {
        let Some(net) = self.networks.get_mut(&id) else {
            return;
        };
        let target = msg.params.first().map_or("", String::as_str);
        let changes = msg.params.get(1..).unwrap_or_default().join(" ");
        let isupport = net.isupport.clone();
        let buffer = if isupport.is_channel(target) {
            let key = net.key(target);
            let Some(channel) = net.channels.get_mut(&key) else {
                return;
            };
            apply_member_modes(channel, &isupport, msg.params.get(1..).unwrap_or_default());
            channel.buffer
        } else {
            net.server_buffer
        };
        let text = Builder::default()
            .push("mode ", Style::default())
            .push(&format::sanitise_identifier(target), bold())
            .push(&format!(" [{changes}] by "), Style::default())
            .push(setter, bold())
            .finish();
        let line = Line {
            time,
            kind: LineKind::Mode,
            sender: None,
            raw: text.to_plain(),
            text,
            delivery: Delivery::Delivered,
            highlight: false,
            msgid: None,
        };
        self.push_line(buffer, line, effects);
    }
}

/// A message (not an event) the server has: not our own still pending or
/// failed.
fn is_settled_message(line: &Line) -> bool {
    matches!(
        line.kind,
        LineKind::Message | LineKind::Action | LineKind::Notice
    ) && !matches!(
        line.delivery,
        Delivery::Pending { .. } | Delivery::Failed { .. }
    )
}

/// The newest time of a message (not an event) in a buffer.
fn newest_message(buffer: &Buffer) -> Option<Timestamp> {
    buffer
        .lines
        .iter()
        .rev()
        .find(|l| is_settled_message(l))
        .map(|l| l.time)
}

/// A history reference: `timestamp=YYYY-MM-DDThh:mm:ss.sssZ`.
fn history_ref(time: Timestamp) -> String {
    format!("timestamp={}", time.strftime("%Y-%m-%dT%H:%M:%S%.3fZ"))
}

/// The `server-time` of a message, if it has one.
fn msg_time(msg: &Message) -> Option<Timestamp> {
    msg.tags.get("time").and_then(|t| t.parse().ok())
}

/// Reports whether `text` mentions `nick` as a whole word.
fn mentions_nick(text: &str, nick: &str, mapping: CaseMapping) -> bool {
    if nick.is_empty() {
        return false;
    }
    let text = format::strip_formatting(text);
    let is_nick_char = |c: char| c.is_alphanumeric() || "-[]\\`^{}|_".contains(c);
    text.split(|c: char| !is_nick_char(c))
        .any(|word| mapping.eq(word, nick))
}

/// Applies membership (prefix) mode changes such as `+o-v alice bob`.
fn apply_member_modes(channel: &mut Channel, isupport: &ISupport, args: &[String]) {
    let Some((modes, mut rest)) = args.split_first() else {
        return;
    };
    let mut adding = true;
    for mode in modes.chars() {
        match mode {
            '+' => adding = true,
            '-' => adding = false,
            mode => {
                let takes_arg = match isupport.mode_kind(mode) {
                    ModeKind::List | ModeKind::Always => true,
                    ModeKind::OnSet => adding,
                    ModeKind::Never => false,
                };
                if !takes_arg {
                    continue;
                }
                let Some((arg, tail)) = rest.split_first() else {
                    return;
                };
                rest = tail;
                if isupport.symbol_for_mode(mode).is_none() {
                    continue;
                }
                let key = CaseKey::new(isupport.casemapping, arg);
                if let Some(member) = channel.members.get_mut(&key) {
                    let mut set: Vec<char> = member.modes.chars().filter(|&m| m != mode).collect();
                    if adding {
                        set.push(mode);
                    }
                    set.sort_by_key(|&m| isupport.mode_rank(m));
                    member.modes = set.into_iter().collect();
                }
            }
        }
    }
}

fn bold() -> Style {
    Style {
        bold: true,
        ..Style::default()
    }
}

/// Appends already-styled text to a builder; it is sanitised again on the way.
fn append(builder: Builder, text: &StyledText) -> Builder {
    text.spans()
        .iter()
        .fold(builder, |b, span| b.push(span.text(), span.style()))
}

/// `JOIN` messages for `channels`, packed as tightly as the line limit allows.
fn join_messages(channels: &[String]) -> Vec<Message> {
    let budget = MAX_BODY_LEN - "JOIN ".len();
    let mut out = Vec::new();
    let mut current = String::new();
    for channel in channels
        .iter()
        .filter(|c| !c.is_empty() && !c.contains([' ', ',', '\r', '\n', '\0']))
    {
        if !current.is_empty() && current.len() + 1 + channel.len() > budget {
            out.push(Message::new("JOIN").param(std::mem::take(&mut current)));
        }
        if !current.is_empty() {
            current.push(',');
        }
        current.push_str(channel);
    }
    if !current.is_empty() {
        out.push(Message::new("JOIN").param(current));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const NET: NetworkId = NetworkId(1);

    fn state() -> State {
        let mut state = State::new(100);
        state.add_network(NET, "test", "me", vec!["#auto".into()]);
        state
    }

    fn feed(state: &mut State, line: &str) -> Vec<Effect> {
        state.apply(NET, NetEvent::Message(Message::parse(line).unwrap()))
    }

    fn last_text(state: &State, buffer: BufferId) -> String {
        state
            .buffer(buffer)
            .unwrap()
            .lines()
            .back()
            .unwrap()
            .text
            .to_plain()
    }

    fn channel_buffer(state: &State, name: &str) -> BufferId {
        state.network(NET).unwrap().channel(name).unwrap().buffer
    }

    /// Members as a nicklist shows them: prefix symbols, then the nick.
    fn nicks(state: &State, channel: &str) -> Vec<String> {
        let net = state.network(NET).unwrap();
        let mut nicks: Vec<String> = net
            .channel(channel)
            .unwrap()
            .members()
            .map(|m| {
                let symbols: String = m
                    .modes
                    .chars()
                    .filter_map(|c| net.isupport.symbol_for_mode(c))
                    .collect();
                format!("{symbols}{}", m.nick)
            })
            .collect();
        nicks.sort();
        nicks
    }

    #[test]
    fn registration_joins_autojoin_and_previous_channels() {
        let mut s = state();
        let effects = s.apply(NET, NetEvent::Registered { nick: "me_".into() });
        assert_eq!(
            effects,
            [Effect::Send(NET, Message::new("JOIN").param("#auto"))]
        );
        assert_eq!(s.network(NET).unwrap().nick, "me_");
        assert_eq!(s.network(NET).unwrap().status, Status::Registered);

        feed(&mut s, ":me_!u@h JOIN #other");
        s.apply(
            NET,
            NetEvent::Disconnected {
                reason: "gone".into(),
                reconnect_in: Some(Duration::from_secs(3)),
            },
        );
        assert_eq!(s.network(NET).unwrap().status, Status::Waiting);
        let chan = s.network(NET).unwrap().channel("#other").unwrap();
        assert!(!chan.joined && chan.rejoin);
        assert_eq!(
            last_text(&s, chan.buffer),
            "Disconnected (gone); reconnecting in 3s"
        );
        let effects = s.apply(NET, NetEvent::Registered { nick: "me".into() });
        assert_eq!(
            effects,
            [Effect::Send(
                NET,
                Message::new("JOIN").param("#auto,#other")
            )]
        );
    }

    #[test]
    fn registration_sends_perform_before_joining() {
        let mut s = state();
        s.set_perform(
            NET,
            vec![
                PerformLine::new("MODE $nick +i"),
                PerformLine::new("PRIVMSG ChanServ :OP #auto $nick"),
            ],
        );
        let effects = s.apply(NET, NetEvent::Registered { nick: "me_".into() });
        assert_eq!(
            effects,
            [
                Effect::Send(NET, Message::new("MODE").param("me_").param("+i")),
                Effect::Send(
                    NET,
                    Message::new("PRIVMSG")
                        .param("ChanServ")
                        .param("OP #auto me_")
                ),
                Effect::Send(NET, Message::new("JOIN").param("#auto")),
            ]
        );
        // Lines that don't parse are skipped rather than sent.
        s.set_perform(NET, vec![PerformLine::new("")]);
        let effects = s.apply(NET, NetEvent::Registered { nick: "me".into() });
        assert_eq!(
            effects,
            [Effect::Send(NET, Message::new("JOIN").param("#auto"))]
        );
    }

    #[test]
    fn membership_tracking() {
        let mut s = state();
        feed(
            &mut s,
            ":srv 005 me PREFIX=(ov)@+ CHANTYPES=# :are supported",
        );
        feed(&mut s, ":me!u@h JOIN #c");
        assert!(s.network(NET).unwrap().channel("#c").unwrap().joined);
        feed(&mut s, ":srv 353 me = #c :@op +voice me plain!u@h");
        feed(&mut s, ":srv 366 me #c :End of /NAMES list.");
        assert_eq!(nicks(&s, "#c"), ["+voice", "@op", "me", "plain"]);
        assert_eq!(last_text(&s, channel_buffer(&s, "#c")), "#c: 4 users");

        feed(&mut s, ":alice!a@h JOIN #c");
        feed(&mut s, ":op!o@h MODE #c +ov-v alice alice voice");
        assert_eq!(nicks(&s, "#c"), ["@+alice", "@op", "me", "plain", "voice"]);
        // +k consumes its argument without touching membership.
        feed(&mut s, ":op!o@h MODE #c -o+k alice key");
        assert_eq!(nicks(&s, "#c"), ["+alice", "@op", "me", "plain", "voice"]);

        feed(&mut s, ":plain!u@h PART #c :bye");
        feed(&mut s, ":op!o@h KICK #c voice :out");
        feed(&mut s, ":alice!a@h NICK Alice2");
        assert_eq!(nicks(&s, "#c"), ["+Alice2", "@op", "me"]);
        feed(&mut s, ":Alice2!a@h QUIT :Ping timeout");
        assert_eq!(nicks(&s, "#c"), ["@op", "me"]);
        assert_eq!(
            last_text(&s, channel_buffer(&s, "#c")),
            "Alice2 has quit (Ping timeout)"
        );

        feed(&mut s, ":op!o@h KICK #c me :bye");
        let chan = s.network(NET).unwrap().channel("#c").unwrap();
        assert!(!chan.joined && chan.members().next().is_none());
    }

    #[test]
    fn messages_route_to_buffers() {
        let mut s = state();
        feed(&mut s, ":srv 005 me STATUSMSG=@ :are supported");
        feed(
            &mut s,
            "@time=2026-01-02T03:04:05.000Z :bob!b@h PRIVMSG #c :\x02hi\x02 there",
        );
        let chan = channel_buffer(&s, "#c");
        let line = s.buffer(chan).unwrap().lines().back().unwrap().clone();
        assert_eq!(line.sender.as_deref(), Some("bob"));
        assert_eq!(line.text.to_plain(), "hi there");
        assert_eq!(line.raw, "\x02hi\x02 there");
        assert_eq!(line.time.to_string(), "2026-01-02T03:04:05Z");

        feed(&mut s, ":bob!b@h PRIVMSG @#c :ops only");
        assert_eq!(last_text(&s, chan), "ops only");

        let effects = feed(&mut s, ":bob!b@h PRIVMSG me :\x01ACTION waves\x01");
        let query = effects
            .iter()
            .find_map(|e| match e {
                Effect::Line(b, _) => Some(*b),
                _ => None,
            })
            .unwrap();
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::Notify(n) if n.private)),
            "a private message notifies"
        );
        assert_eq!(s.buffer(query).unwrap().kind, BufferKind::Query);
        assert_eq!(
            s.buffer(query).unwrap().lines().back().unwrap().kind,
            LineKind::Action
        );
        // Our own echoed message goes to the query with the target.
        feed(&mut s, ":me!u@h PRIVMSG bob :hello");
        assert_eq!(last_text(&s, query), "hello");
        assert_eq!(s.buffer(query).unwrap().unread, 2);
        s.mark_read(query);
        assert_eq!(s.buffer(query).unwrap().unread, 0);

        let server = s.network(NET).unwrap().server_buffer;
        feed(
            &mut s,
            ":irc.example NOTICE * :*** Looking up your hostname",
        );
        assert_eq!(last_text(&s, server), "*** Looking up your hostname");
        feed(&mut s, ":bob!b@h PRIVMSG me :\x01VERSION\x01");
        assert_eq!(
            last_text(&s, server),
            "CTCP VERSION request from bob (answered)"
        );
        feed(&mut s, ":srv 422 me :MOTD File is missing");
        assert_eq!(
            s.buffer(server).unwrap().lines().back().unwrap().kind,
            LineKind::Info,
            "no MOTD isn't an error"
        );
        feed(&mut s, ":srv 433 * me :Nickname is already in use");
        assert_eq!(
            s.buffer(server).unwrap().lines().back().unwrap().kind,
            LineKind::Error
        );
        assert_eq!(last_text(&s, server), "me Nickname is already in use");
    }

    fn fixed() -> Timestamp {
        "2026-10-31T12:00:00Z".parse().unwrap()
    }

    /// CTCP requests are answered, a few at a time.
    #[test]
    fn ctcp_replies_and_their_limit() {
        let mut s = state();
        s.set_clock(fixed);
        let reply = |s: &mut State, line: &str| -> Option<String> {
            feed(s, line).into_iter().find_map(|e| match e {
                Effect::Send(_, m) if m.command == "NOTICE" => Some(m.to_wire().unwrap()),
                _ => None,
            })
        };
        let version = reply(&mut s, ":bob!b@h PRIVMSG me :\x01VERSION\x01").unwrap();
        assert!(
            version.starts_with("NOTICE bob :\x01VERSION Irsty "),
            "{version}"
        );
        // PING echoes, without anything that could break out of the CTCP.
        assert_eq!(
            reply(&mut s, ":bob!b@h PRIVMSG #c :\x01PING 12345\x01"),
            Some("NOTICE bob :\x01PING 12345\x01".into())
        );
        assert_eq!(
            reply(&mut s, ":carol!c@h PRIVMSG me :\x01CLIENTINFO\x01"),
            Some(
                "NOTICE carol :\x01CLIENTINFO ACTION CLIENTINFO PING SOURCE TIME VERSION\x01"
                    .into()
            )
        );
        // Three in ten seconds is the most; the rest are noted, not answered.
        assert_eq!(reply(&mut s, ":dave!d@h PRIVMSG me :\x01SOURCE\x01"), None);
        let server = s.network(NET).unwrap().server_buffer;
        assert_eq!(
            last_text(&s, server),
            "CTCP SOURCE request from dave (not answered: too many requests)"
        );

        let mut s = state();
        s.set_clock(fixed);
        assert_eq!(
            reply(&mut s, ":bob!b@h PRIVMSG me :\x01SOURCE\x01"),
            Some("NOTICE bob :\x01SOURCE https://github.com/gawbul/irsty\x01".into())
        );
        // Unknown requests, replies and our own echoes get no answer.
        assert_eq!(reply(&mut s, ":bob!b@h PRIVMSG me :\x01FINGER\x01"), None);
        assert_eq!(reply(&mut s, ":bob!b@h NOTICE me :\x01VERSION x\x01"), None);
        assert_eq!(reply(&mut s, ":me!u@h PRIVMSG bob :\x01VERSION\x01"), None);
        // Our own reply echoed back adds no line.
        let server = s.network(NET).unwrap().server_buffer;
        let before = s.buffer(server).unwrap().total;
        feed(&mut s, ":me!u@h NOTICE bob :\x01VERSION Irsty\x01");
        assert_eq!(s.buffer(server).unwrap().total, before);

        // Turned off, or TIME alone turned off.
        s.set_ctcp(CtcpConfig {
            time: false,
            ..CtcpConfig::default()
        });
        assert_eq!(reply(&mut s, ":bob!b@h PRIVMSG me :\x01TIME\x01"), None);
        s.set_ctcp(CtcpConfig {
            replies: false,
            ..CtcpConfig::default()
        });
        assert_eq!(reply(&mut s, ":bob!b@h PRIVMSG me :\x01VERSION\x01"), None);
    }

    /// Highlights and private messages notify and are marked; our own lines
    /// and channel chatter don't.
    #[test]
    fn highlights_notify_and_are_marked() {
        let mut s = state();
        feed(&mut s, ":me!u@h JOIN #c");
        let channel = channel_buffer(&s, "#c");
        let notified = |effects: &[Effect]| {
            effects.iter().find_map(|e| match e {
                Effect::Notify(n) => Some((**n).clone()),
                _ => None,
            })
        };
        assert!(notified(&feed(&mut s, ":a!a@h PRIVMSG #c :just chat")).is_none());
        let n = notified(&feed(&mut s, ":a!a@h PRIVMSG #c :me: look")).unwrap();
        assert_eq!(
            (
                n.place.as_str(),
                n.sender.as_str(),
                n.text.as_str(),
                n.private
            ),
            ("#c", "a", "me: look", false)
        );
        assert!(s.buffer(channel).unwrap().lines().back().unwrap().highlight);
        let n = notified(&feed(&mut s, ":a!a@h PRIVMSG me :psst")).unwrap();
        assert!(n.private);
        // Our own message mentioning our nick, and events, never notify.
        assert!(notified(&feed(&mut s, ":me!u@h PRIVMSG #c :me me me")).is_none());
        assert!(notified(&feed(&mut s, ":me!u@h JOIN #d")).is_none());
    }

    /// History that arrives after newer live lines goes in time order.
    #[test]
    fn history_is_inserted_in_time_order() {
        let mut s = state();
        s.set_clock(|| "2026-10-31T09:00:00Z".parse().unwrap());
        feed(&mut s, ":me!u@h JOIN #c");
        let channel = channel_buffer(&s, "#c");
        feed(
            &mut s,
            "@time=2026-10-31T10:05:00.000Z;msgid=live :a!a@h PRIVMSG #c :live",
        );
        let effects = feed(&mut s, "BATCH +h chathistory #c");
        assert!(effects.is_empty());
        feed(
            &mut s,
            "@batch=h;time=2026-10-31T10:01:00.000Z;msgid=h1 :b!b@h PRIVMSG #c :earlier",
        );
        feed(
            &mut s,
            "@batch=h;time=2026-10-31T10:05:00.000Z;msgid=live :a!a@h PRIVMSG #c :live",
        );
        let effects = feed(&mut s, "BATCH -h");
        let texts: Vec<String> = s
            .buffer(channel)
            .unwrap()
            .lines()
            .iter()
            .map(|l| l.text.to_plain())
            .collect();
        assert_eq!(texts, ["me (u@h) has joined #c", "earlier", "live"]);
        // The inserted line is logged as itself, not as "the newest line".
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::Log(_, l) if l.raw == "earlier"))
        );
        assert!(!effects.iter().any(|e| matches!(e, Effect::Line(..))));
    }

    /// Rejoining after a reconnect asks only for what came after the newest
    /// message we have; history off, or no server support, asks nothing.
    #[test]
    fn rejoining_fills_the_gap() {
        let mut s = state();
        s.apply(
            NET,
            NetEvent::Caps {
                enabled: vec!["draft/chathistory".into()],
            },
        );
        feed(&mut s, ":srv 005 me CHATHISTORY=50 :are supported");
        feed(&mut s, ":me!u@h JOIN #c");
        feed(
            &mut s,
            "@time=2026-10-31T10:00:00.123Z :a!a@h PRIVMSG #c :hi",
        );
        let latest = |effects: Vec<Effect>| -> Vec<String> {
            effects
                .into_iter()
                .filter_map(|e| match e {
                    Effect::Send(_, m) if m.command == "CHATHISTORY" => Some(m.to_wire().unwrap()),
                    _ => None,
                })
                .collect()
        };
        // The server's limit (50) caps our 100.
        assert_eq!(
            latest(feed(&mut s, ":me!u@h JOIN #c")),
            ["CHATHISTORY LATEST #c timestamp=2026-10-31T10:00:00.123Z 50"]
        );
        s.set_history(0);
        assert!(latest(feed(&mut s, ":me!u@h JOIN #c")).is_empty());
        s.set_history(100);
        s.apply(
            NET,
            NetEvent::Caps {
                enabled: Vec::new(),
            },
        );
        assert!(latest(feed(&mut s, ":me!u@h JOIN #c")).is_empty());
    }

    #[test]
    fn topics_and_nick_changes() {
        let mut s = state();
        feed(&mut s, ":me!u@h JOIN #c");
        feed(&mut s, ":srv 332 me #c :\x0304red\x03 topic");
        feed(&mut s, ":srv 333 me #c setter!u@h 1700000000");
        let chan = s.network(NET).unwrap().channel("#c").unwrap();
        assert_eq!(chan.topic.as_ref().unwrap().to_plain(), "red topic");
        assert_eq!(chan.topic_setter.as_deref(), Some("setter"));
        feed(&mut s, ":bob!b@h TOPIC #c :new");
        assert_eq!(
            last_text(&s, channel_buffer(&s, "#c")),
            "bob changed the topic of #c to: new"
        );

        feed(&mut s, ":bob!b@h PRIVMSG me :hi");
        feed(&mut s, ":bob!b@h NICK robert");
        let query = s.buffers().find(|b| b.kind == BufferKind::Query).unwrap();
        assert_eq!(query.name, "robert");
        feed(&mut s, ":me!u@h NICK myself");
        assert_eq!(s.network(NET).unwrap().nick, "myself");
    }

    #[test]
    fn casemapping_change_rekeys() {
        let mut s = state();
        feed(&mut s, ":me!u@h JOIN #Chan[1]");
        assert!(
            s.network(NET).unwrap().channel("#chan{1}").is_some(),
            "rfc1459 by default"
        );
        feed(&mut s, ":srv 005 me CASEMAPPING=ascii :are supported");
        let net = s.network(NET).unwrap();
        assert!(net.channel("#chan{1}").is_none());
        assert!(net.channel("#CHAN[1]").is_some());
        assert!(
            net.channel("#Chan[1]")
                .unwrap()
                .member(CaseMapping::Ascii, "ME")
                .is_some()
        );
    }

    #[test]
    fn hostile_text_is_sanitised() {
        let mut s = state();
        feed(&mut s, ":ev\u{202e}il!u@h PRIVMSG #c :\x1b]0;pwned\x07 hi");
        let chan = channel_buffer(&s, "#c");
        let line = s.buffer(chan).unwrap().lines().back().unwrap();
        assert_eq!(line.sender.as_deref(), Some("evil"));
        assert!(!line.text.to_plain().chars().any(char::is_control));
    }

    #[test]
    fn scrollback_is_capped() {
        let mut s = State::new(3);
        s.add_network(NET, "test", "me", Vec::new());
        for i in 0..10 {
            feed(&mut s, &format!(":b!b@h PRIVMSG #c :{i}"));
        }
        let buffer = s.buffer(channel_buffer(&s, "#c")).unwrap();
        assert_eq!(buffer.lines().len(), 3);
        assert_eq!(buffer.total, 10);
        assert_eq!(buffer.lines().front().unwrap().text.to_plain(), "7");
    }

    #[test]
    fn connection_events() {
        let mut s = state();
        let server = s.network(NET).unwrap().server_buffer;
        s.apply(
            NET,
            NetEvent::Connecting {
                server: "h:6697 (TLS)".into(),
            },
        );
        assert_eq!(s.network(NET).unwrap().status, Status::Connecting);
        s.apply(
            NET,
            NetEvent::Connected {
                server: "h:6697 (TLS)".into(),
                insecure: true,
            },
        );
        assert!(
            last_text(&s, server).starts_with("WARNING: TLS certificate verification is disabled")
        );
        s.apply(NET, NetEvent::Sasl(Ok(Some("acct".into()))));
        assert_eq!(s.network(NET).unwrap().account.as_deref(), Some("acct"));
        s.apply(NET, NetEvent::Sasl(Err("bad password".into())));
        assert_eq!(
            last_text(&s, server),
            "SASL authentication failed: bad password"
        );
        s.apply(
            NET,
            NetEvent::Caps {
                enabled: vec!["server-time".into()],
            },
        );
        s.apply(NET, NetEvent::Lag(Duration::from_millis(42)));
        assert_eq!(s.network(NET).unwrap().lag, Some(Duration::from_millis(42)));
        s.apply(NET, NetEvent::Error("send queue full".into()));
        assert_eq!(last_text(&s, server), "send queue full");
        s.apply(
            NET,
            NetEvent::Disconnected {
                reason: "quit".into(),
                reconnect_in: None,
            },
        );
        assert_eq!(s.network(NET).unwrap().status, Status::Disconnected);
        assert!(s.network(NET).unwrap().caps.is_empty());
        // Events for unknown networks are ignored.
        assert!(
            s.apply(NetworkId(99), NetEvent::Lag(Duration::ZERO))
                .is_empty()
        );
    }

    #[test]
    fn repeated_join_keeps_modes() {
        let mut s = state();
        feed(&mut s, ":me!u@h JOIN #c");
        feed(&mut s, ":srv 353 me = #c :@op me");
        feed(&mut s, ":srv 366 me #c :End");
        feed(&mut s, ":op!o@h JOIN #c");
        assert_eq!(nicks(&s, "#c"), ["@op", "me"]);
    }

    #[test]
    fn activity_levels() {
        let mut s = state();
        feed(&mut s, ":b!b@h JOIN #c");
        let chan = channel_buffer(&s, "#c");
        assert_eq!(s.buffer(chan).unwrap().activity, Activity::Events);
        feed(&mut s, ":b!b@h PRIVMSG #c :hello all");
        assert_eq!(s.buffer(chan).unwrap().activity, Activity::Messages);
        feed(&mut s, ":b!b@h PRIVMSG #c :\x02ME\x02: ping");
        assert_eq!(s.buffer(chan).unwrap().activity, Activity::Highlight);
        s.mark_read(chan);
        assert_eq!(s.buffer(chan).unwrap().activity, Activity::None);
        feed(&mut s, ":b!b@h PRIVMSG #c :meme time");
        assert_eq!(
            s.buffer(chan).unwrap().activity,
            Activity::Messages,
            "not a whole-word mention"
        );
        feed(&mut s, ":b!b@h PRIVMSG me :psst");
        let query = s.find_buffer(NET, "b").unwrap();
        assert_eq!(s.buffer(query).unwrap().activity, Activity::Highlight);
    }

    #[test]
    fn own_messages_and_closing() {
        let mut s = state();
        let own = s.own_message(NET, "#c", LineKind::Message, "hi \x02there");
        let buffer = own.buffer.unwrap();
        assert_eq!(own.effects, [Effect::Line(buffer, 1)]);
        assert_eq!(own.label, None, "no labels without echo-message");
        let b = s.buffer(buffer).unwrap();
        assert_eq!((b.unread, b.activity), (0, Activity::None));
        let line = b.lines().back().unwrap();
        assert_eq!(
            (line.sender.as_deref(), line.text.to_plain().as_str()),
            (Some("me"), "hi there")
        );
        let query = s
            .own_message(NET, "bob", LineKind::Action, "waves")
            .buffer
            .unwrap();
        assert_eq!(s.find_buffer(NET, "BOB"), Some(query));
        assert!(s.remove_buffer(query));
        assert!(!s.remove_buffer(query));
        assert_eq!(s.find_buffer(NET, "bob"), None);
        assert!(s.remove_buffer(buffer));
        assert!(s.network(NET).unwrap().channel("#c").is_none());
        let server = s.network(NET).unwrap().server_buffer;
        assert!(!s.remove_buffer(server), "server buffers stay");
        assert!(
            s.own_message(NetworkId(9), "#x", LineKind::Message, "x")
                .buffer
                .is_none()
        );
    }

    fn texts(state: &State, buffer: BufferId) -> Vec<String> {
        state
            .buffer(buffer)
            .unwrap()
            .lines()
            .iter()
            .map(|l| l.text.to_plain())
            .collect()
    }

    #[test]
    fn whois_replies_go_where_they_were_asked_for() {
        let mut s = state();
        feed(&mut s, ":me!u@h JOIN #c");
        feed(&mut s, ":alice!a@h JOIN #c");
        let channel = channel_buffer(&s, "#c");
        let server = s.network(NET).unwrap().server_buffer;
        s.expect_reply(NET, Awaiting::Whois("ALICE".into()), channel);
        for line in [
            ":srv 311 me alice a host.example * :Alice Liddell",
            ":srv 312 me alice irc.example.net :Example",
            ":srv 330 me alice alice_acct :is logged in as",
            ":srv 301 me alice :lunch",
            ":srv 318 me alice :End of /WHOIS list.",
        ] {
            feed(&mut s, line);
        }
        let lines = texts(&s, channel);
        assert_eq!(
            lines[lines.len() - 4..],
            [
                "alice is a@host.example (Alice Liddell)",
                "  server: irc.example.net (Example)",
                "  account: alice_acct",
                "  away: lunch",
            ]
        );
        let alice = s.network(NET).unwrap().user("alice").unwrap().clone();
        assert_eq!(alice.account.as_deref(), Some("alice_acct"));
        assert_eq!(alice.realname.as_deref(), Some("Alice Liddell"));
        // The WHOIS is over, so the next reply lands in the server buffer.
        feed(&mut s, ":srv 312 me alice irc.example.net :Example");
        assert_eq!(last_text(&s, server), "  server: irc.example.net (Example)");
        // An unknown nick ends the wait with an error where it was asked.
        s.expect_reply(NET, Awaiting::Whois("ghost".into()), channel);
        feed(&mut s, ":srv 401 me ghost :No such nick/channel");
        assert_eq!(last_text(&s, channel), "No such nick: ghost");
        // A 301 outside a WHOIS answers a message to someone away.
        s.query_buffer(NET, "alice");
        feed(&mut s, ":srv 301 me alice :gone fishing");
        let query = s.find_buffer(NET, "alice").unwrap();
        assert_eq!(last_text(&s, query), "alice is away: gone fishing");
    }

    #[test]
    fn names_and_list_replies() {
        let mut s = state();
        feed(&mut s, ":srv 005 me PREFIX=(ov)@+ :are supported");
        feed(&mut s, ":me!u@h JOIN #c");
        let channel = channel_buffer(&s, "#c");
        s.expect_reply(NET, Awaiting::Names("#C".into()), channel);
        feed(&mut s, ":srv 353 me = #c :+bob @me alice");
        feed(&mut s, ":srv 366 me #c :End of /NAMES list.");
        assert_eq!(last_text(&s, channel), "Names on #c: @me +bob alice");
        s.expect_reply(NET, Awaiting::List, channel);
        feed(&mut s, ":srv 321 me Channel :Users Name");
        feed(&mut s, ":srv 322 me #rust 1200 :All about Rust");
        feed(&mut s, ":srv 323 me :End of /LIST");
        let lines = texts(&s, channel);
        assert_eq!(
            lines[lines.len() - 2..],
            ["#rust (1200): All about Rust", "End of channel list"]
        );
        feed(&mut s, ":srv 324 me #c +nt");
        assert_eq!(last_text(&s, channel), "Modes for #c: +nt");
    }

    #[test]
    fn ignored_users_are_hidden_but_tracked() {
        let mut s = state();
        s.set_rules(
            vec![
                IgnoreRule {
                    mask: "spammer".into(),
                    what: IgnoreKind::ALL.to_vec(),
                    networks: Vec::new(),
                    channels: Vec::new(),
                },
                IgnoreRule {
                    mask: "*!*@noisy.example".into(),
                    what: vec![IgnoreKind::Events],
                    networks: vec!["TEST".into()],
                    channels: vec!["#c".into()],
                },
            ],
            &HighlightConfig::default(),
        );
        feed(&mut s, ":me!u@h JOIN #c");
        let channel = channel_buffer(&s, "#c");
        let before = texts(&s, channel).len();
        feed(&mut s, ":spammer!x@y JOIN #c");
        feed(&mut s, ":spammer!x@y PRIVMSG #c :buy now");
        feed(&mut s, ":spammer!x@y PRIVMSG me :psst");
        feed(&mut s, ":chatty!c@noisy.example JOIN #c");
        assert_eq!(texts(&s, channel).len(), before, "{:?}", texts(&s, channel));
        assert_eq!(s.find_buffer(NET, "spammer"), None, "no query opened");
        let members = s
            .network(NET)
            .unwrap()
            .channel("#c")
            .unwrap()
            .members()
            .count();
        assert_eq!(members, 3, "membership still tracked");
        // Only events are hidden for the second rule; messages still show.
        feed(&mut s, ":chatty!c@noisy.example PRIVMSG #c :hello");
        assert_eq!(last_text(&s, channel), "hello");
        // Our own lines are never hidden.
        assert!(!s.is_ignored(NET, "me!u@h", None, IgnoreKind::Messages));
    }

    #[test]
    fn highlight_words_and_patterns() {
        let mut s = state();
        s.set_rules(
            Vec::new(),
            &HighlightConfig {
                words: vec!["irsty".into()],
                regex: vec![r"\bsteve\b".into()],
            },
        );
        feed(&mut s, ":me!u@h JOIN #c");
        let channel = channel_buffer(&s, "#c");
        let activity = |s: &mut State, line: &str| {
            s.mark_read(channel);
            feed(s, line);
            s.buffer(channel).unwrap().activity
        };
        assert_eq!(
            activity(&mut s, ":a!a@h PRIVMSG #c :nothing here"),
            Activity::Messages
        );
        assert_eq!(
            activity(&mut s, ":a!a@h PRIVMSG #c :I love IRSTY!"),
            Activity::Highlight
        );
        assert_eq!(
            activity(&mut s, ":a!a@h PRIVMSG #c :irstyx is not it"),
            Activity::Messages
        );
        assert_eq!(
            activity(&mut s, ":a!a@h PRIVMSG #c :ask Steve"),
            Activity::Highlight
        );
        assert_eq!(
            activity(&mut s, ":a!a@h PRIVMSG #c :me: hi"),
            Activity::Highlight
        );
    }

    /// Two lines added to one buffer by one event are each logged: the effect
    /// names the line, not just the buffer.
    #[test]
    fn effects_name_the_line_they_added() {
        let mut s = state();
        feed(&mut s, ":me!u@h JOIN #c");
        let channel = channel_buffer(&s, "#c");
        s.expect_reply(NET, Awaiting::Names("#c".into()), channel);
        feed(&mut s, ":srv 353 me = #c :me");
        let effects = feed(&mut s, ":srv 366 me #c :End");
        let logged: Vec<String> = effects
            .iter()
            .filter_map(|e| match e {
                Effect::Line(b, seq) => s.buffer(*b).unwrap().line(*seq),
                _ => None,
            })
            .map(|l| l.text.to_plain())
            .collect();
        assert_eq!(logged, ["#c: 1 users", "Names on #c: me"]);
        assert!(s.buffer(channel).unwrap().line(0).is_none());
        assert!(s.buffer(channel).unwrap().line(999).is_none());
    }

    #[test]
    fn clearing_removing_and_shrinking() {
        let mut s = state();
        feed(&mut s, ":me!u@h JOIN #c");
        let channel = channel_buffer(&s, "#c");
        for i in 0..10 {
            feed(&mut s, &format!(":a!a@h PRIVMSG #c :{i}"));
        }
        s.set_scrollback(3);
        assert_eq!(texts(&s, channel), ["7", "8", "9"]);
        s.clear_buffer(channel);
        assert!(texts(&s, channel).is_empty());
        s.remove_network(NET);
        assert!(s.network(NET).is_none());
        assert_eq!(s.buffers().count(), 0);
    }

    #[test]
    fn joins_are_packed() {
        let many: Vec<String> = (0..100)
            .map(|i| format!("#channel-number-{i:03}"))
            .collect();
        let msgs = join_messages(&many);
        assert!(msgs.len() > 1);
        for msg in &msgs {
            assert!(msg.to_wire().unwrap().len() <= MAX_BODY_LEN);
        }
        let total: usize = msgs.iter().map(|m| m.params[0].split(',').count()).sum();
        assert_eq!(total, 100);
        assert!(join_messages(&["bad name".into(), String::new()]).is_empty());
    }
}
