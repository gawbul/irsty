//! The connection task: one per network. It owns the socket, registers, runs CAP
//! and SASL, answers pings, rate-limits output, and reconnects with backoff.
//!
//! It reports to the app loop over a bounded channel and blocks when that channel
//! is full, so a slow UI slows reads rather than buffering without bound
//! (ADR 0002).

use std::collections::VecDeque;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use irsty_proto::cap::{CapEvent, CapNegotiator};
use irsty_proto::sasl::{Authenticator, Mechanism, Outcome};
use irsty_proto::{Message, Secret};
use tokio::sync::mpsc;
use tokio::time::{Instant, sleep_until};
use tokio_util::codec::Framed;

use super::backoff::Backoff;
use super::codec::IrcCodec;
use super::ratelimit::TokenBucket;
use super::transport::{self, Stream};
use crate::config::{ClientCertConfig, ServerConfig};
use crate::event::{NetCommand, NetEvent, NetworkId};

/// Capabilities requested by default, beyond `sasl`.
pub const DEFAULT_CAPS: &[&str] = &[
    "cap-notify",
    "message-tags",
    "multi-prefix",
    "server-time",
    "userhost-in-names",
];

/// Most messages held for sending before new ones are refused.
const MAX_QUEUE: usize = 1024;

/// How long to wait for the server to close after `QUIT`.
const QUIT_GRACE: Duration = Duration::from_secs(2);

/// Timing knobs. Defaults suit real networks; tests shorten them.
#[derive(Clone, Debug)]
pub struct Timeouts {
    /// TCP plus TLS handshake.
    pub connect: Duration,
    /// From connecting to `RPL_WELCOME`.
    pub registration: Duration,
    /// Silence before we send a `PING`.
    pub ping_interval: Duration,
    /// Wait for the `PONG` before giving up.
    pub ping_timeout: Duration,
    /// A connection registered this long resets the backoff.
    pub stable_after: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(30),
            registration: Duration::from_secs(90),
            ping_interval: Duration::from_secs(90),
            ping_timeout: Duration::from_secs(60),
            stable_after: Duration::from_secs(60),
        }
    }
}

/// Outgoing rate limit: a burst, then one message per interval.
#[derive(Clone, Copy, Debug)]
pub struct RateLimit {
    /// Messages allowed back to back.
    pub burst: u32,
    /// Interval between messages after the burst.
    pub interval: Duration,
}

impl Default for RateLimit {
    fn default() -> Self {
        Self {
            burst: 5,
            interval: Duration::from_millis(2000),
        }
    }
}

/// A server with its password already resolved.
#[derive(Clone, Debug)]
pub struct Server {
    /// From the config.
    pub config: ServerConfig,
    /// Resolved `PASS`.
    pub password: Option<Secret>,
}

/// Everything a connection task needs. Secrets are resolved before it starts.
#[derive(Clone, Debug)]
pub struct Settings {
    /// Identifies the network in events.
    pub id: NetworkId,
    /// Servers, rotated on failure.
    pub servers: Vec<Server>,
    /// Nicks to try in order.
    pub nicks: Vec<String>,
    /// Username (ident).
    pub username: String,
    /// Real name.
    pub realname: String,
    /// SASL, if wanted.
    pub sasl: Option<Mechanism>,
    /// TLS client certificate.
    pub client_cert: Option<ClientCertConfig>,
    /// Capabilities to request when offered.
    pub caps: Vec<String>,
    /// Timing.
    pub timeouts: Timeouts,
    /// Output rate limit.
    pub rate: RateLimit,
    /// First reconnect delay.
    pub backoff_base: Duration,
    /// Longest reconnect delay.
    pub backoff_cap: Duration,
}

impl Settings {
    /// Settings with defaults for everything but identity.
    pub fn new(id: NetworkId, servers: Vec<Server>, nicks: Vec<String>) -> Self {
        let first = nicks.first().cloned().unwrap_or_else(|| "irsty".into());
        Self {
            id,
            servers,
            username: first.clone(),
            realname: first,
            nicks,
            sasl: None,
            client_cert: None,
            caps: DEFAULT_CAPS.iter().map(|&c| c.to_owned()).collect(),
            timeouts: Timeouts::default(),
            rate: RateLimit::default(),
            backoff_base: Duration::from_secs(2),
            backoff_cap: Duration::from_secs(300),
        }
    }
}

/// The sending half of the app's event channel.
pub type EventSender = mpsc::Sender<(NetworkId, NetEvent)>;

/// How a session ended.
enum End {
    /// We quit on purpose.
    Quit,
    /// The app is gone.
    Closed,
    /// The connection failed or dropped.
    Lost { reason: String, stable: bool },
    /// The app asked to reconnect now.
    Reconnect,
}

/// Runs a network's connection until it quits or the app goes away.
pub async fn run(
    settings: Settings,
    events: EventSender,
    mut commands: mpsc::Receiver<NetCommand>,
) {
    let id = settings.id;
    let mut backoff = Backoff::new(settings.backoff_base, settings.backoff_cap);
    let mut index = 0usize;
    let emit = |event| events.send((id, event));

    loop {
        let Some(server) = settings.servers.get(index % settings.servers.len().max(1)) else {
            let _ = emit(NetEvent::Disconnected {
                reason: "no servers configured".into(),
                reconnect_in: None,
            })
            .await;
            return;
        };
        if emit(NetEvent::Connecting {
            server: server.config.to_string(),
        })
        .await
        .is_err()
        {
            return;
        }

        let end = match open(&settings, server).await {
            Ok(stream) => {
                let connected = NetEvent::Connected {
                    server: server.config.to_string(),
                    insecure: server.config.tls && !server.config.tls_verify,
                };
                if emit(connected).await.is_err() {
                    return;
                }
                Session::new(&settings, server, &events)
                    .run(stream, &mut commands)
                    .await
            }
            Err(error) => End::Lost {
                reason: format!("cannot connect to {}: {error}", server.config),
                stable: false,
            },
        };

        let reason = match end {
            End::Quit => {
                let _ = emit(NetEvent::Disconnected {
                    reason: "quit".into(),
                    reconnect_in: None,
                })
                .await;
                return;
            }
            End::Closed => return,
            End::Reconnect => {
                let _ = emit(NetEvent::Disconnected {
                    reason: "reconnecting".into(),
                    reconnect_in: Some(Duration::ZERO),
                })
                .await;
                continue;
            }
            End::Lost { reason, stable } => {
                if stable {
                    backoff.reset();
                }
                reason
            }
        };

        index = index.wrapping_add(1);
        let delay = backoff.next_delay();
        if emit(NetEvent::Disconnected {
            reason,
            reconnect_in: Some(delay),
        })
        .await
        .is_err()
        {
            return;
        }
        if !wait_to_reconnect(delay, &mut commands, &events, id).await {
            return;
        }
    }
}

/// Waits out the backoff while still obeying commands. Returns false to stop.
async fn wait_to_reconnect(
    delay: Duration,
    commands: &mut mpsc::Receiver<NetCommand>,
    events: &EventSender,
    id: NetworkId,
) -> bool {
    let deadline = Instant::now() + delay;
    loop {
        tokio::select! {
            () = sleep_until(deadline) => return true,
            command = commands.recv() => match command {
                None | Some(NetCommand::Quit(_)) => {
                    let _ = events.send((id, NetEvent::Disconnected { reason: "quit".into(), reconnect_in: None })).await;
                    return false;
                }
                Some(NetCommand::Reconnect) => return true,
                Some(NetCommand::Send(_)) => {
                    if events.send((id, NetEvent::Error("not connected; message dropped".into()))).await.is_err() {
                        return false;
                    }
                }
            },
        }
    }
}

async fn open(settings: &Settings, server: &Server) -> Result<Stream, transport::ConnectError> {
    let tls = if server.config.tls {
        Some(transport::tls_config(
            server.config.tls_verify,
            settings.client_cert.as_ref(),
        )?)
    } else {
        None
    };
    transport::connect(&server.config, tls, settings.timeouts.connect).await
}

/// One connected session, from registration until the socket closes.
struct Session<'a> {
    settings: &'a Settings,
    server: &'a Server,
    events: &'a EventSender,
    queue: VecDeque<String>,
    bucket: TokenBucket,
    cap: CapNegotiator,
    sasl: Option<Authenticator>,
    /// Account from 900, which arrives before the 903 that ends the exchange.
    account: Option<String>,
    nick: String,
    nick_attempt: usize,
    started: Instant,
    registered_at: Option<Instant>,
    last_rx: Instant,
    ping: Option<(Instant, String)>,
}

impl<'a> Session<'a> {
    fn new(settings: &'a Settings, server: &'a Server, events: &'a EventSender) -> Self {
        let now = Instant::now();
        Self {
            settings,
            server,
            events,
            queue: VecDeque::new(),
            bucket: TokenBucket::new(settings.rate.burst, settings.rate.interval, now),
            cap: CapNegotiator::new(settings.caps.iter().cloned(), settings.sasl.is_some()),
            sasl: None,
            account: None,
            nick: settings
                .nicks
                .first()
                .cloned()
                .unwrap_or_else(|| "irsty".into()),
            nick_attempt: 0,
            started: now,
            registered_at: None,
            last_rx: now,
            ping: None,
        }
    }

    async fn emit(&self, event: NetEvent) -> Result<(), End> {
        self.events
            .send((self.settings.id, event))
            .await
            .map_err(|_| End::Closed)
    }

    fn lost(&self, reason: impl Into<String>) -> End {
        let stable = self
            .registered_at
            .is_some_and(|at| at.elapsed() >= self.settings.timeouts.stable_after);
        End::Lost {
            reason: reason.into(),
            stable,
        }
    }

    /// Queues a message; it is sent when the rate limit allows.
    fn enqueue(&mut self, msg: &Message) -> Result<(), String> {
        let line = msg
            .to_wire()
            .map_err(|e| format!("cannot send {}: {e}", msg.command))?;
        if self.queue.len() >= MAX_QUEUE {
            return Err("send queue full; message dropped".into());
        }
        self.queue.push_back(line);
        Ok(())
    }

    fn enqueue_all(&mut self, msgs: impl IntoIterator<Item = Message>) {
        for msg in msgs {
            if let Err(error) = self.enqueue(&msg) {
                tracing::warn!(%error, "dropping protocol message");
            }
        }
    }

    /// The next time a timer needs attention.
    fn deadline(&self) -> Instant {
        let t = &self.settings.timeouts;
        let keepalive = match &self.ping {
            Some((sent, _)) => *sent + t.ping_timeout,
            None => self.last_rx + t.ping_interval,
        };
        if self.registered_at.is_none() {
            keepalive.min(self.started + t.registration)
        } else {
            keepalive
        }
    }

    async fn run(mut self, stream: Stream, commands: &mut mpsc::Receiver<NetCommand>) -> End {
        let mut framed = Framed::new(stream, IrcCodec::default());

        let mut hello = Vec::new();
        if let Some(password) = &self.server.password {
            hello.push(Message::new("PASS").param(password.expose()));
        }
        hello.push(self.cap.start());
        hello.push(Message::new("NICK").param(self.nick.clone()));
        hello.push(
            Message::new("USER")
                .param(self.settings.username.clone())
                .param("0")
                .param("*")
                .param(self.settings.realname.clone()),
        );
        self.enqueue_all(hello);

        loop {
            let send_at = self.bucket.ready_at();
            let has_output = !self.queue.is_empty();
            tokio::select! {
                incoming = framed.next() => {
                    let msg = match incoming {
                        Some(Ok(msg)) => msg,
                        Some(Err(error)) => return self.lost(format!("read error: {error}")),
                        None => return self.lost("connection closed by server"),
                    };
                    self.last_rx = Instant::now();
                    if let Some(pong) = ping_reply(&msg) {
                        // Answer immediately, ahead of the queue and the rate limit.
                        if let Err(error) = framed.send(pong).await {
                            return self.lost(format!("write error: {error}"));
                        }
                        continue;
                    }
                    if let Err(end) = self.handle(msg).await {
                        return end;
                    }
                }
                command = commands.recv() => match command {
                    Some(NetCommand::Send(msg)) => {
                        if let Err(error) = self.enqueue(&msg)
                            && let Err(end) = self.emit(NetEvent::Error(error)).await
                        {
                            return end;
                        }
                    }
                    Some(NetCommand::Quit(reason)) => {
                        let mut quit = Message::new("QUIT");
                        if let Some(reason) = reason {
                            quit = quit.param(reason);
                        }
                        // Flush what the user already sent before saying goodbye;
                        // the rate limit no longer matters once we are leaving.
                        for line in self.queue.drain(..) {
                            if framed.feed(line).await.is_err() {
                                break;
                            }
                        }
                        if let Ok(line) = quit.to_wire() {
                            let _ = framed.send(line).await;
                        }
                        // Give the server a moment to close cleanly.
                        let _ = tokio::time::timeout(QUIT_GRACE, async {
                            while let Some(Ok(_)) = framed.next().await {}
                        })
                        .await;
                        return End::Quit;
                    }
                    Some(NetCommand::Reconnect) => return End::Reconnect,
                    None => return End::Closed,
                },
                () = sleep_until(send_at), if has_output => {
                    if self.bucket.try_take(Instant::now())
                        && let Some(line) = self.queue.pop_front()
                        && let Err(error) = framed.send(line).await
                    {
                        return self.lost(format!("write error: {error}"));
                    }
                }
                () = sleep_until(self.deadline()) => {
                    if let Some(end) = self.on_timer(&mut framed).await {
                        return end;
                    }
                }
            }
        }
    }

    async fn on_timer(&mut self, framed: &mut Framed<Stream, IrcCodec>) -> Option<End> {
        let now = Instant::now();
        let t = &self.settings.timeouts;
        if self.registered_at.is_none() && now >= self.started + t.registration {
            return Some(self.lost("registration timed out"));
        }
        match &self.ping {
            Some((sent, _)) if now >= *sent + t.ping_timeout => {
                Some(self.lost(format!("ping timeout ({}s)", t.ping_timeout.as_secs())))
            }
            None if now >= self.last_rx + t.ping_interval => {
                let token = format!("irsty-{}", now.duration_since(self.started).as_millis());
                self.ping = Some((now, token.clone()));
                let line = Message::new("PING")
                    .param(token)
                    .to_wire()
                    .unwrap_or_default();
                framed
                    .send(line)
                    .await
                    .err()
                    .map(|e| self.lost(format!("write error: {e}")))
            }
            // A ping is outstanding but not yet late, or nothing is due.
            _ => None,
        }
    }

    async fn handle(&mut self, msg: Message) -> Result<(), End> {
        let command = msg.command.to_ascii_uppercase();
        match command.as_str() {
            "PONG" => {
                if let Some((sent, token)) = &self.ping
                    && msg.params.last() == Some(token)
                {
                    let lag = sent.elapsed();
                    self.ping = None;
                    self.emit(NetEvent::Lag(lag)).await?;
                }
                return Ok(());
            }
            "CAP" => self.on_cap(&msg).await?,
            "AUTHENTICATE" | "900" | "901" | "902" | "903" | "904" | "905" | "906" | "907"
            | "908" => {
                self.on_sasl(&msg).await?;
                if command == "AUTHENTICATE" {
                    return Ok(());
                }
            }
            "001" => {
                if let Some(nick) = msg.params.first() {
                    self.nick.clone_from(nick);
                }
                self.registered_at = Some(Instant::now());
                let step = self.cap.registered();
                self.on_cap_events(step.events).await?;
                self.emit(NetEvent::Registered {
                    nick: self.nick.clone(),
                })
                .await?;
            }
            "421"
                if msg
                    .params
                    .get(1)
                    .is_some_and(|c| c.eq_ignore_ascii_case("CAP")) =>
            {
                let step = self.cap.registered();
                self.on_cap_events(step.events).await?;
            }
            "432" | "433" | "436" | "437" if self.registered_at.is_none() => {
                self.nick = self.next_nick();
                self.enqueue_all([Message::new("NICK").param(self.nick.clone())]);
            }
            "ERROR" => {
                let reason = msg.params.last().cloned().unwrap_or_default();
                self.emit(NetEvent::Message(msg)).await?;
                return Err(self.lost(format!("server closed the link: {reason}")));
            }
            _ => {}
        }
        self.emit(NetEvent::Message(msg)).await
    }

    fn next_nick(&mut self) -> String {
        self.nick_attempt += 1;
        let nicks = &self.settings.nicks;
        if let Some(nick) = nicks.get(self.nick_attempt) {
            return nick.clone();
        }
        let base = nicks.first().map_or("irsty", String::as_str);
        format!("{base}{}", self.nick_attempt + 1 - nicks.len())
    }

    async fn on_cap(&mut self, msg: &Message) -> Result<(), End> {
        let step = self.cap.handle(&msg.as_ref());
        self.enqueue_all(step.send);
        self.on_cap_events(step.events).await
    }

    async fn on_cap_events(&mut self, events: Vec<CapEvent>) -> Result<(), End> {
        let mut caps_changed = false;
        for event in events {
            match event {
                CapEvent::Enabled(_) | CapEvent::Disabled(_) => caps_changed = true,
                CapEvent::SaslReady => {
                    if let Some(mechanism) = self.settings.sasl.clone() {
                        let auth = Authenticator::new(mechanism);
                        self.enqueue_all([auth.start()]);
                        self.sasl = Some(auth);
                    }
                }
                CapEvent::SaslUnavailable => {
                    self.emit(NetEvent::Sasl(Err("the server does not offer SASL".into())))
                        .await?;
                }
                CapEvent::Rejected(caps) => tracing::debug!(?caps, "capabilities rejected"),
                CapEvent::Finished => {}
            }
        }
        if caps_changed {
            let enabled = self.cap.enabled().map(str::to_owned).collect();
            self.emit(NetEvent::Caps { enabled }).await?;
        }
        Ok(())
    }

    async fn on_sasl(&mut self, msg: &Message) -> Result<(), End> {
        let Some(auth) = self.sasl.as_mut() else {
            return Ok(());
        };
        let step = auth.handle(&msg.as_ref());
        self.enqueue_all(step.send);
        if step.logged_in_as.is_some() {
            self.account = step.logged_in_as;
        }
        if let Some(outcome) = step.outcome {
            let result = match outcome {
                Outcome::Success | Outcome::AlreadyAuthenticated => Ok(self.account.take()),
                Outcome::Failure(reason) => Err(reason),
            };
            self.sasl = None;
            self.emit(NetEvent::Sasl(result)).await?;
            let finish = self.cap.finish();
            self.enqueue_all(finish.send);
        }
        Ok(())
    }
}

/// The reply to a server `PING`, if `msg` is one.
fn ping_reply(msg: &Message) -> Option<String> {
    if !msg.command.eq_ignore_ascii_case("PING") {
        return None;
    }
    let token = msg.params.last().cloned().unwrap_or_default();
    Message::new("PONG").param(token).to_wire().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_pings() {
        assert_eq!(
            ping_reply(&Message::parse("PING :abc def").unwrap()).as_deref(),
            Some("PONG :abc def")
        );
        assert_eq!(
            ping_reply(&Message::parse("PING").unwrap()).as_deref(),
            Some("PONG :")
        );
        assert_eq!(ping_reply(&Message::parse("PONG x").unwrap()), None);
    }

    #[test]
    fn settings_defaults() {
        let s = Settings::new(NetworkId(1), Vec::new(), vec!["me".into()]);
        assert_eq!(s.username, "me");
        assert!(s.caps.iter().any(|c| c == "server-time"));
    }
}
