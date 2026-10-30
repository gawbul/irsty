//! The app loop's engine: owns [`State`], spawns a connection task per network,
//! applies events, and carries out effects. Frontends (the TUI, headless mode)
//! drive it with [`Client::next`] and read [`Client::state`].

use std::collections::BTreeMap;
use std::time::Duration;

use irsty_proto::Message;
use irsty_proto::sasl::Mechanism;
use tokio::sync::mpsc::{self, error::TrySendError};
use tokio::task::JoinSet;

use crate::config::{Config, NetworkConfig, Paths, SaslMechanism};
use crate::event::{NetCommand, NetEvent, NetworkId};
use crate::log::LogWriter;
use crate::net::connection::{self, EventSender, Server, Settings};
use crate::state::{BufferId, Effect, LineKind, State};

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
}

/// Lets tests and alternative frontends tweak connection settings.
pub type SettingsHook = Box<dyn Fn(&mut Settings) + Send>;

/// The client: state plus running connections.
pub struct Client {
    state: State,
    events: mpsc::Receiver<(NetworkId, NetEvent)>,
    events_tx: EventSender,
    commands: BTreeMap<NetworkId, mpsc::Sender<NetCommand>>,
    tasks: JoinSet<()>,
    logger: Option<LogWriter>,
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
    /// with `autoconnect` set. Problems with one network (an unreadable secret,
    /// say) are reported in its server buffer rather than failing the rest.
    pub async fn start(config: &Config, paths: Option<&Paths>, hook: Option<SettingsHook>) -> Self {
        let (events_tx, events) = mpsc::channel(EVENT_QUEUE);
        let mut client = Self {
            state: State::new(config.scrollback),
            events,
            events_tx,
            commands: BTreeMap::new(),
            tasks: JoinSet::new(),
            logger: None,
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
            if !net.autoconnect {
                continue;
            }
            match settings_for(id, net).await {
                Ok(mut settings) => {
                    if let Some(hook) = &hook {
                        hook(&mut settings);
                    }
                    client.spawn(settings);
                }
                Err(error) => client.note(id, LineKind::Error, &format!("Not connecting: {error}")),
            }
        }
        client
    }

    fn spawn(&mut self, settings: Settings) {
        let (tx, rx) = mpsc::channel(COMMAND_QUEUE);
        self.commands.insert(settings.id, tx);
        self.tasks
            .spawn(connection::run(settings, self.events_tx.clone(), rx));
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
            SaslMechanism::Plain => {
                let password = match &sasl.password {
                    Some(source) => source
                        .resolve()
                        .await
                        .map_err(|e| format!("SASL password: {e}"))?,
                    None => return Err("SASL plain needs a password".into()),
                };
                Mechanism::Plain {
                    account: sasl.account.clone().unwrap_or_default(),
                    password,
                }
            }
        }),
    };
    Ok(settings)
}
