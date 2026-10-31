//! The app loop's engine: owns [`State`], spawns a connection task per network,
//! applies events, and carries out effects. Frontends (the TUI, headless mode)
//! drive it with [`Client::next`] and read [`Client::state`].

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use irsty_proto::sasl::Mechanism;
use irsty_proto::{Message, ctcp, split};
use tokio::sync::mpsc::{self, error::TrySendError};
use tokio::task::JoinSet;

use crate::config::{Config, NetworkConfig, Paths, SaslMechanism};
use crate::event::{NetCommand, NetEvent, NetworkId};
use crate::log::LogWriter;
use crate::net::connection::{self, EventSender, Server, Settings};
use crate::state::{BufferId, BufferKind, Effect, LineKind, State};

/// Events buffered between connection tasks and the app loop.
const EVENT_QUEUE: usize = 1024;

/// Commands buffered per connection.
const COMMAND_QUEUE: usize = 256;

/// Most events applied before returning, so bursts become one redraw.
const MAX_BATCH: usize = 512;

/// How long [`Client::quit`] waits for connections to close.
const QUIT_TIMEOUT: Duration = Duration::from_secs(5);

/// Why a message could not be handed to a connection.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SendError {
    /// No such network, or it is not running.
    #[error("not connected")]
    NotConnected,
    /// The connection's command queue is full.
    #[error("send queue full")]
    QueueFull,
    /// The network is already connected or connecting.
    #[error("already connected")]
    AlreadyConnected,
    /// No network has that id.
    #[error("no such network")]
    UnknownNetwork,
    /// The line is not a valid IRC message.
    #[error("not a valid IRC message")]
    Invalid,
    /// Messages cannot be sent to a server buffer.
    #[error("this is a server buffer; use /msg or /join")]
    ServerBuffer,
}

/// Lets tests and alternative frontends tweak connection settings.
pub type SettingsHook = Arc<dyn Fn(&mut Settings) + Send + Sync>;

/// The client: state plus running connections.
pub struct Client {
    state: State,
    events: mpsc::Receiver<(NetworkId, NetEvent)>,
    events_tx: EventSender,
    commands: BTreeMap<NetworkId, mpsc::Sender<NetCommand>>,
    tasks: JoinSet<()>,
    logger: Option<LogWriter>,
    configs: BTreeMap<NetworkId, NetworkConfig>,
    hook: Option<SettingsHook>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("networks", &self.commands.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl Client {
    /// Builds state from `config`, starts logging, and connects every network
    /// with `autoconnect` set. Must be called inside a Tokio runtime.
    ///
    /// Secrets are resolved inside each network's task, so a slow `pass` prompt
    /// never blocks the UI. Problems with one network (an unreadable secret, say)
    /// are reported in its server buffer rather than failing the rest.
    pub fn start(config: &Config, paths: Option<&Paths>, hook: Option<SettingsHook>) -> Self {
        let (events_tx, events) = mpsc::channel(EVENT_QUEUE);
        let mut client = Self {
            state: State::new(config.scrollback),
            events,
            events_tx,
            commands: BTreeMap::new(),
            tasks: JoinSet::new(),
            logger: None,
            configs: BTreeMap::new(),
            hook,
        };

        if config.logging.enabled {
            let dir = config
                .logging
                .dir
                .clone()
                .or_else(|| paths.map(Paths::log_dir));
            match dir.map(LogWriter::spawn) {
                Some(Ok(writer)) => client.logger = Some(writer),
                Some(Err(error)) => {
                    tracing::warn!(%error, "logging disabled: cannot create the log directory");
                }
                None => tracing::warn!("logging disabled: no log directory"),
            }
        }

        for (index, net) in config.networks.iter().enumerate() {
            let id = NetworkId(u32::try_from(index).unwrap_or(u32::MAX));
            let nick = net.nicks.first().map_or("irsty", String::as_str);
            client
                .state
                .add_network(id, &net.name, nick, net.autojoin.clone());
            client.state.set_perform(id, net.perform.clone());
            for warning in config
                .plaintext_secrets()
                .iter()
                .filter(|w| w.starts_with(&format!("{}:", net.name)))
            {
                client.note(
                    id,
                    LineKind::Error,
                    &format!("Plaintext secret in config ({warning}); prefer keyring or command"),
                );
            }
            client.configs.insert(id, net.clone());
            if net.autoconnect {
                let _ = client.connect(id);
            }
        }
        client
    }

    /// Connects a network that is not running (at startup, or after
    /// [`Client::disconnect`]).
    pub fn connect(&mut self, id: NetworkId) -> Result<(), SendError> {
        if self.commands.contains_key(&id) {
            return Err(SendError::AlreadyConnected);
        }
        let net = self
            .configs
            .get(&id)
            .cloned()
            .ok_or(SendError::UnknownNetwork)?;
        let (tx, rx) = mpsc::channel(COMMAND_QUEUE);
        self.commands.insert(id, tx);
        let events = self.events_tx.clone();
        let hook = self.hook.clone();
        self.tasks.spawn(async move {
            match settings_for(id, &net).await {
                Ok(mut settings) => {
                    if let Some(hook) = hook {
                        hook(&mut settings);
                    }
                    connection::run(settings, events, rx).await;
                }
                Err(error) => {
                    let _ = events
                        .send((id, NetEvent::Error(format!("Not connecting: {error}"))))
                        .await;
                    let stopped = NetEvent::Disconnected {
                        reason: "a secret could not be read".into(),
                        reconnect_in: None,
                    };
                    let _ = events.send((id, stopped)).await;
                }
            }
        });
        Ok(())
    }

    /// Quits one network; it stays down until [`Client::connect`].
    pub fn disconnect(&self, id: NetworkId, reason: Option<&str>) -> Result<(), SendError> {
        let tx = self.commands.get(&id).ok_or(SendError::NotConnected)?;
        tx.try_send(NetCommand::Quit(reason.map(str::to_owned)))
            .map_err(|_| SendError::QueueFull)
    }

    /// Reports whether a network's connection task is running.
    pub fn is_running(&self, id: NetworkId) -> bool {
        self.commands.contains_key(&id)
    }

    /// Sends text to a channel or nick, split to fit, and records it locally.
    /// With `action`, sends a CTCP ACTION (`/me`). Returns the buffer it was
    /// recorded in.
    pub fn say(
        &mut self,
        id: NetworkId,
        target: &str,
        text: &str,
        action: bool,
    ) -> Result<Option<BufferId>, SendError> {
        let net = self.state.network(id).ok_or(SendError::UnknownNetwork)?;
        if net.status != crate::state::Status::Registered {
            return Err(SendError::NotConnected);
        }
        let nick = net.nick.clone();
        // ACTION adds "\x01ACTION " and "\x01".
        let overhead = if action { 9 } else { 0 };
        let budget = split::text_budget(&nick, None, None, "PRIVMSG", target)
            .saturating_sub(overhead)
            .max(1);
        let mut buffer = None;
        for chunk in text
            .split(['\r', '\n'])
            .flat_map(|line| split::split_text(line, budget))
        {
            if chunk.is_empty() {
                continue;
            }
            let body = if action {
                ctcp::encode("ACTION", Some(chunk))
            } else {
                chunk.to_owned()
            };
            self.send(id, Message::new("PRIVMSG").param(target).param(body))?;
            let kind = if action {
                LineKind::Action
            } else {
                LineKind::Message
            };
            // Never keep a services password in scrollback or logs.
            let shown = crate::redact::services_text(target, chunk);
            let (recorded, effects) = self.state.own_message(id, target, kind, &shown);
            buffer = recorded.or(buffer);
            self.run_effects(effects);
        }
        Ok(buffer)
    }

    /// Sends a raw protocol line, as typed with `/raw`, and notes a redacted copy
    /// in the server buffer.
    pub fn raw(&mut self, id: NetworkId, line: &str) -> Result<(), SendError> {
        let msg = Message::parse(line).map_err(|_| SendError::Invalid)?;
        self.send(id, msg)?;
        self.note(
            id,
            LineKind::Info,
            &format!("-> {}", crate::redact::raw_line(line)),
        );
        Ok(())
    }

    /// Opens (or finds) the query buffer for `nick`.
    pub fn open_query(&mut self, id: NetworkId, nick: &str) -> Option<BufferId> {
        self.state.query_buffer(id, nick)
    }

    /// Adds an informational line to any buffer, for command feedback.
    pub fn note_buffer(&mut self, buffer: BufferId, kind: LineKind, text: &str) {
        let effects = self.state.note_buffer(buffer, kind, text);
        self.run_effects(effects);
    }

    /// Closes a buffer, parting the channel first if we are in it. Server
    /// buffers cannot be closed.
    pub fn close_buffer(
        &mut self,
        buffer: BufferId,
        reason: Option<&str>,
    ) -> Result<(), SendError> {
        let buf = self.state.buffer(buffer).ok_or(SendError::UnknownNetwork)?;
        if buf.kind == BufferKind::Server {
            return Err(SendError::ServerBuffer);
        }
        let network = buf.network;
        let name = buf.name.clone();
        let joined = buf.kind == BufferKind::Channel
            && self
                .state
                .network(network)
                .and_then(|n| n.channel(&name))
                .is_some_and(|c| c.joined);
        if joined {
            let mut part = Message::new("PART").param(name);
            if let Some(reason) = reason {
                part = part.param(reason);
            }
            // Closing must work even when the link is down.
            let _ = self.send(network, part);
        }
        self.state.remove_buffer(buffer);
        Ok(())
    }

    /// Adds an informational line to a network's server buffer.
    pub fn note(&mut self, id: NetworkId, kind: LineKind, text: &str) {
        let effects = self.state.note(id, kind, text);
        self.run_effects(effects);
    }

    /// Read access to state, for rendering.
    pub fn state(&self) -> &State {
        &self.state
    }

    /// Marks a buffer as read.
    pub fn mark_read(&mut self, buffer: BufferId) {
        self.state.mark_read(buffer);
    }

    /// Waits for the next batch of events and applies it. Returns the buffers
    /// that gained lines, or `None` once every connection has stopped.
    ///
    /// Cancel-safe: the only await is the channel receive, so it can be used in
    /// `select!` alongside terminal input.
    pub async fn next(&mut self) -> Option<Vec<BufferId>> {
        if self.commands.is_empty() {
            return None;
        }
        let first = self.events.recv().await?;
        let mut changed = Vec::new();
        let mut event = Some(first);
        let mut applied = 0;
        while let Some((id, ev)) = event.take() {
            if let NetEvent::Disconnected {
                reconnect_in: None, ..
            } = ev
            {
                self.commands.remove(&id);
            }
            let effects = self.state.apply(id, ev);
            changed.extend(self.run_effects(effects));
            applied += 1;
            if applied < MAX_BATCH {
                event = self.events.try_recv().ok();
            }
        }
        changed.sort_unstable();
        changed.dedup();
        Some(changed)
    }

    /// Carries out effects; returns buffers that gained lines.
    fn run_effects(&mut self, effects: Vec<Effect>) -> Vec<BufferId> {
        let mut changed = Vec::new();
        for effect in effects {
            match effect {
                Effect::Send(id, msg) => {
                    if let Err(error) = self.send(id, msg) {
                        tracing::warn!(?id, %error, "cannot send");
                    }
                }
                Effect::Line(buffer) => {
                    self.log(buffer);
                    changed.push(buffer);
                }
            }
        }
        changed
    }

    fn log(&self, buffer: BufferId) {
        let Some(logger) = &self.logger else {
            return;
        };
        let Some(buf) = self.state.buffer(buffer) else {
            return;
        };
        let Some(network) = self.state.network(buf.network) else {
            return;
        };
        if let Some(line) = buf.lines().back() {
            let name = if buf.id == network.server_buffer {
                "-server"
            } else {
                buf.name.as_str()
            };
            logger.log(&network.name, name, line);
        }
    }

    /// Sends a message on a network. Never blocks.
    pub fn send(&self, id: NetworkId, msg: Message) -> Result<(), SendError> {
        let tx = self.commands.get(&id).ok_or(SendError::NotConnected)?;
        tx.try_send(NetCommand::Send(msg)).map_err(|e| match e {
            TrySendError::Full(_) => SendError::QueueFull,
            TrySendError::Closed(_) => SendError::NotConnected,
        })
    }

    /// Asks a network to drop its connection and reconnect now.
    pub fn reconnect(&self, id: NetworkId) -> Result<(), SendError> {
        let tx = self.commands.get(&id).ok_or(SendError::NotConnected)?;
        tx.try_send(NetCommand::Reconnect)
            .map_err(|_| SendError::QueueFull)
    }

    /// Quits every network and waits briefly for them to close, applying their
    /// final events. Logs are flushed.
    pub async fn quit(&mut self, reason: Option<&str>) {
        for tx in self.commands.values() {
            let _ = tx.try_send(NetCommand::Quit(reason.map(str::to_owned)));
        }
        let drain = async {
            // Keep receiving so tasks never block on a full event queue.
            while !self.commands.is_empty() {
                if self.next().await.is_none() {
                    break;
                }
            }
            while self.tasks.join_next().await.is_some() {}
        };
        if tokio::time::timeout(QUIT_TIMEOUT, drain).await.is_err() {
            tracing::warn!("connections did not close in time; aborting them");
            self.tasks.abort_all();
        }
        self.commands.clear();
        if let Some(logger) = self.logger.take() {
            logger.shutdown();
        }
    }
}

/// Resolves secrets and builds connection settings for a network.
async fn settings_for(id: NetworkId, net: &NetworkConfig) -> Result<Settings, String> {
    let mut servers = Vec::with_capacity(net.servers.len());
    for server in &net.servers {
        let password = match &server.password {
            Some(source) => Some(
                source
                    .resolve()
                    .await
                    .map_err(|e| format!("server password: {e}"))?,
            ),
            None => None,
        };
        servers.push(Server {
            config: server.clone(),
            password,
        });
    }
    let mut settings = Settings::new(id, servers, net.nicks.clone());
    if let Some(username) = &net.username {
        settings.username.clone_from(username);
    }
    if let Some(realname) = &net.realname {
        settings.realname.clone_from(realname);
    }
    settings.client_cert.clone_from(&net.client_cert);
    settings.sasl = match &net.sasl {
        None => None,
        Some(sasl) => Some(match sasl.mechanism {
            SaslMechanism::External => Mechanism::External,
            SaslMechanism::Plain | SaslMechanism::ScramSha256 => {
                let password = match &sasl.password {
                    Some(source) => source
                        .resolve()
                        .await
                        .map_err(|e| format!("SASL password: {e}"))?,
                    None => return Err("SASL needs a password".into()),
                };
                let account = sasl.account.clone().unwrap_or_default();
                if sasl.mechanism == SaslMechanism::Plain {
                    Mechanism::Plain { account, password }
                } else {
                    Mechanism::ScramSha256 { account, password }
                }
            }
        }),
    };
    Ok(settings)
}
