//! DCC: file transfers and chats directly between clients (M16).
//!
//! Nothing is accepted automatically. An offer from someone is listed until
//! the user takes it with `/dcc get` or `/dcc chat`, and expires after
//! [`OFFER_TIMEOUT`]. Received files are written in the download directory
//! under a sanitised name, as `name.part` (created new, never over anything),
//! and linked to their final name only when complete and only if that name
//! is free, so nothing is ever overwritten. A transfer that stops short stays
//! as `name.part` for `/dcc resume`. The advertised size is a limit: a peer
//! that sends more is cut off.
//!
//! [`Dcc`] is the bookkeeping: it takes requests from the network and from
//! the user, and returns [`Out`]s for the client to carry out (CTCP replies,
//! lines to show). Each connection runs as a Tokio task that reports back as
//! [`Event`]s.

use std::fmt::Write as _;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use irsty_proto::dcc::{self as proto, Dcc as Request};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncSeekExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

use crate::event::NetworkId;
use crate::state::LineKind;

/// How long an offer waits: someone else's for `/dcc get`, or ours for the
/// other side to connect.
pub const OFFER_TIMEOUT: Duration = Duration::from_mins(5);
/// How long connecting may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a transfer may go without data.
const IDLE_TIMEOUT: Duration = Duration::from_secs(120);
/// How long a sender waits for the last acknowledgement.
const FINAL_ACK_TIMEOUT: Duration = Duration::from_secs(30);
/// Bytes read or written at a time.
const CHUNK: usize = 64 * 1024;
/// Longest line in a DCC chat.
const MAX_CHAT_LINE: usize = 4096;
/// Most transfers (offers included) kept at once.
const MAX_TRANSFERS: usize = 64;
/// Shortest gap between progress reports.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

/// Identifies a transfer or chat.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DccId(pub u64);

impl std::fmt::Display for DccId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// What a transfer is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A file coming to us.
    Get,
    /// A file going from us.
    Send,
    /// A chat.
    Chat,
}

/// Where a transfer is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Someone offered it; waiting for the user.
    Offered,
    /// We asked to resume; waiting for their `ACCEPT`.
    Resuming,
    /// We offered it; waiting for them.
    Waiting,
    /// Connecting.
    Connecting,
    /// Under way.
    Active,
    /// Finished.
    Done,
    /// Stopped, and why.
    Failed(String),
}

/// A transfer or chat.
#[derive(Debug)]
pub struct Transfer {
    /// Its id, for `/dcc close`.
    pub id: DccId,
    /// The network it was arranged over.
    pub network: NetworkId,
    /// Who with.
    pub nick: String,
    /// What it is.
    pub kind: Kind,
    /// The file's (sanitised) name.
    pub name: Option<String>,
    /// The file on disk: being received (`.part`), received, or being sent.
    pub path: Option<PathBuf>,
    /// The file's size, if known.
    pub size: Option<u64>,
    /// Bytes transferred so far, the resumed part included.
    pub bytes: u64,
    /// Where it started (resuming).
    pub offset: u64,
    /// Where it is.
    pub status: Status,
    /// When data started flowing.
    pub started: Option<Instant>,
    /// The name as the other side wrote it, for `RESUME` and `ACCEPT`.
    offered: Option<String>,
    addr: Option<IpAddr>,
    port: u16,
    token: Option<String>,
    passive: bool,
    created: Instant,
    task: Option<tokio::task::AbortHandle>,
    lines: Option<mpsc::UnboundedSender<String>>,
    /// Where a send starts, which a resume can change after the send task
    /// is listening; the task reads it when the connection comes.
    resume_at: Arc<AtomicU64>,
}

impl Transfer {
    /// Whether it is over.
    pub fn finished(&self) -> bool {
        matches!(self.status, Status::Done | Status::Failed(_))
    }

    /// Bytes a second since it started, if it has.
    pub fn rate(&self) -> Option<f64> {
        let secs = self.started?.elapsed().as_secs_f64();
        #[allow(clippy::cast_precision_loss, reason = "a display figure")]
        let moved = (self.bytes - self.offset) as f64;
        (secs > 0.0).then(|| moved / secs)
    }
}

/// What a connection task reports.
#[derive(Debug)]
pub enum Event {
    /// Connected; data is flowing.
    Connected(DccId),
    /// Bytes transferred so far.
    Progress(DccId, u64),
    /// Finished.
    Finished(DccId, u64),
    /// Stopped.
    Failed(DccId, String),
    /// A chat line arrived.
    Line(DccId, String),
    /// The chat ended.
    Closed(DccId),
}

/// Something for the client to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Out {
    /// Send a CTCP `DCC` request (these parameters) to `nick`.
    Ctcp {
        /// Where.
        network: NetworkId,
        /// To whom.
        nick: String,
        /// The parameters after `DCC `.
        params: String,
    },
    /// Show a line in the network's server buffer.
    Note {
        /// Which network.
        network: NetworkId,
        /// What kind of line.
        kind: LineKind,
        /// What it says.
        text: String,
    },
    /// Tell the user: someone offered something.
    Notify {
        /// Which network.
        network: NetworkId,
        /// Who.
        nick: String,
        /// What.
        text: String,
    },
    /// A chat connected: open its `=nick` buffer.
    Open {
        /// Which network.
        network: NetworkId,
        /// With whom.
        nick: String,
    },
    /// A chat line, theirs or ours, for the `=nick` buffer.
    Chat {
        /// Which network.
        network: NetworkId,
        /// With whom.
        nick: String,
        /// The line.
        text: String,
        /// We said it.
        own: bool,
    },
}

/// What the manager needs from the outside for one network.
#[derive(Clone, Copy, Debug)]
pub struct Context<'a> {
    /// Where received files go.
    pub dir: &'a Path,
    /// The ports to listen on, if limited.
    pub ports: Option<(u16, u16)>,
    /// The address our offers advertise, or `None` if DCC can't be offered
    /// on this network (through a proxy, which DCC would bypass).
    pub advertise: Option<IpAddr>,
}

/// Every transfer and chat.
#[derive(Debug)]
pub struct Dcc {
    transfers: Vec<Transfer>,
    next: u64,
    next_token: u32,
    tx: mpsc::UnboundedSender<Event>,
    events: mpsc::UnboundedReceiver<Event>,
    warned: bool,
}

impl Default for Dcc {
    fn default() -> Self {
        let (tx, events) = mpsc::unbounded_channel();
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| d.subsec_nanos());
        Self {
            transfers: Vec::new(),
            next: 0,
            next_token: seed % 100_000 + 1,
            tx,
            events,
            warned: false,
        }
    }
}

/// A human-readable size: `512 B`, `1.5 kB`, `2.3 MB`.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    #[allow(clippy::cast_precision_loss, reason = "a display figure")]
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS.get(unit).unwrap_or(&"B"))
    }
}

fn eq_nick(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// Refuses to connect where an offer shouldn't point: system ports, and
/// addresses that aren't one host.
fn check_target(addr: IpAddr, port: u16) -> Result<(), String> {
    let broadcast = matches!(addr, IpAddr::V4(v4) if v4.is_broadcast());
    if addr.is_unspecified() || addr.is_multicast() || broadcast {
        return Err(format!(
            "the offer points at {addr}, which isn't a host; not connecting"
        ));
    }
    if port < 1024 {
        return Err(format!(
            "the offer points at port {port}, where system services listen; not connecting"
        ));
    }
    Ok(())
}

/// Whether an address can only be reached nearby.
fn is_private(addr: IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => {
            // 100.64.0.0/10 is carrier-grade NAT.
            let [a, b, ..] = v4.octets();
            v4.is_private() || v4.is_link_local() || (a == 100 && b & 0xc0 == 64)
        }
        IpAddr::V6(v6) => v6.is_unique_local() || v6.is_unicast_link_local(),
    }
}

/// Listens on a port in `ports` (or any), on every address of the family
/// `like` belongs to. Returns the listener and its port.
fn listen(ports: Option<(u16, u16)>, like: IpAddr) -> io::Result<(TcpListener, u16)> {
    let any: IpAddr = if like.is_ipv6() {
        Ipv6Addr::UNSPECIFIED.into()
    } else {
        Ipv4Addr::UNSPECIFIED.into()
    };
    let (first, last) = ports.unwrap_or((0, 0));
    for port in first..=last {
        match std::net::TcpListener::bind((any, port)) {
            Ok(listener) => {
                listener.set_nonblocking(true)?;
                let port = listener.local_addr()?.port();
                return Ok((TcpListener::from_std(listener)?, port));
            }
            Err(e) if e.kind() == io::ErrorKind::AddrInUse => {}
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AddrInUse,
        "every port in dcc.ports is in use",
    ))
}

/// `name`, or with a number before its extension (`photo.1.jpg`), the
/// first that `taken` says is free.
fn candidates(name: &str) -> impl Iterator<Item = String> + '_ {
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (name, String::new()),
    };
    std::iter::once(name.to_owned()).chain((1..10_000).map(move |n| format!("{stem}.{n}{ext}")))
}

/// Creates a new `.part` file for `name` in `dir`, with a name no file has
/// yet (neither finished nor partial). Returns the file and its path.
fn create_part(dir: &Path, name: &str) -> io::Result<(std::fs::File, PathBuf)> {
    std::fs::create_dir_all(dir)?;
    for candidate in candidates(name) {
        if dir.join(&candidate).exists() {
            continue;
        }
        let part = dir.join(format!("{candidate}.part"));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&part)
        {
            Ok(file) => return Ok((file, part)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no free file name",
    ))
}

/// Gives a finished `.part` file its name: the first free one, linked so an
/// existing file can never be replaced.
fn finish_part(part: &Path) -> io::Result<PathBuf> {
    let dir = part.parent().unwrap_or(Path::new("."));
    let name = part
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_suffix(".part"))
        .unwrap_or("download")
        .to_owned();
    for candidate in candidates(&name) {
        let target = dir.join(&candidate);
        match std::fs::hard_link(part, &target) {
            Ok(()) => {
                std::fs::remove_file(part)?;
                return Ok(target);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            // A file system without hard links: rename, after looking.
            Err(_) if !target.exists() => {
                std::fs::rename(part, &target)?;
                return Ok(target);
            }
            Err(e) if target.exists() => {
                let _ = e;
            }
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no free file name",
    ))
}

impl Dcc {
    /// Every transfer and chat, oldest first.
    pub fn list(&self) -> &[Transfer] {
        &self.transfers
    }

    /// Whether anything is under way, so events may still come.
    pub fn busy(&self) -> bool {
        self.transfers
            .iter()
            .any(|t| t.task.is_some() && !t.finished())
    }

    /// The next event from a connection task.
    pub async fn next_event(&mut self) -> Option<Event> {
        self.events.recv().await
    }

    fn new_id(&mut self) -> DccId {
        self.next += 1;
        DccId(self.next)
    }

    fn new_token(&mut self) -> String {
        self.next_token = self.next_token.wrapping_add(1).max(1);
        self.next_token.to_string()
    }

    fn get_mut(&mut self, id: DccId) -> Option<&mut Transfer> {
        self.transfers.iter_mut().find(|t| t.id == id)
    }

    /// Drops offers nobody took, and finished transfers beyond the limit.
    fn tidy(&mut self) {
        let now = Instant::now();
        self.transfers.retain(|t| {
            !(matches!(t.status, Status::Offered | Status::Resuming)
                && now.duration_since(t.created) > OFFER_TIMEOUT)
        });
        while self.transfers.len() > MAX_TRANSFERS {
            match self.transfers.iter().position(Transfer::finished) {
                Some(i) => {
                    self.transfers.remove(i);
                }
                None => break,
            }
        }
    }

    /// The once-per-run note that offers reveal your address.
    fn reveal_note(&mut self, network: NetworkId, addr: IpAddr) -> Option<Out> {
        if std::mem::replace(&mut self.warned, true) {
            return None;
        }
        let mut text = format!("DCC offers tell the other person your IP address ({addr})");
        if is_private(addr) {
            text.push_str(
                "; it is a private address, so someone outside your network can't connect: \
                 set dcc.ip and forward dcc.ports, or use -passive",
            );
        }
        Some(Out::Note {
            network,
            kind: LineKind::Info,
            text,
        })
    }

    fn note(network: NetworkId, kind: LineKind, text: String) -> Out {
        Out::Note {
            network,
            kind,
            text,
        }
    }

    /// Handles a DCC request from `nick`: an offer, or an answer to ours.
    pub fn on_request(
        &mut self,
        network: NetworkId,
        nick: &str,
        request: Request,
        ctx: Context<'_>,
    ) -> Vec<Out> {
        self.tidy();
        match request {
            Request::Send {
                file,
                addr,
                port,
                size,
                token,
            } => {
                if let Some(token) = &token
                    && port != 0
                {
                    return self.passive_answer(
                        network,
                        nick,
                        Kind::Send,
                        (addr, port, token),
                        ctx,
                    );
                }
                self.offer(
                    network,
                    nick,
                    Kind::Get,
                    Some(file),
                    addr,
                    port,
                    size,
                    token,
                )
            }
            Request::Chat { addr, port, token } => {
                if let Some(token) = &token
                    && port != 0
                {
                    return self.passive_answer(
                        network,
                        nick,
                        Kind::Chat,
                        (addr, port, token),
                        ctx,
                    );
                }
                self.offer(network, nick, Kind::Chat, None, addr, port, None, token)
            }
            Request::Resume {
                file,
                port,
                position,
                token,
            } => self.on_resume(network, nick, &file, port, position, token),
            Request::Accept {
                file,
                port,
                position,
                token,
            } => self.on_accept(network, nick, &file, port, position, token.as_ref(), ctx),
        }
    }

    /// Someone offers a file or a chat: list it, and say how to take it.
    #[allow(clippy::too_many_arguments, reason = "the fields of the offer")]
    fn offer(
        &mut self,
        network: NetworkId,
        nick: &str,
        kind: Kind,
        offered: Option<String>,
        addr: IpAddr,
        port: u16,
        size: Option<u64>,
        token: Option<String>,
    ) -> Vec<Out> {
        let pending = self
            .transfers
            .iter()
            .filter(|t| t.status == Status::Offered)
            .count();
        if pending >= MAX_TRANSFERS / 2 {
            return vec![Dcc::note(
                network,
                LineKind::Info,
                format!(
                    "Ignored a DCC offer from {nick}: too many offers waiting (/dcc close some)"
                ),
            )];
        }
        if port == 0 && token.is_none() {
            return vec![Dcc::note(
                network,
                LineKind::Info,
                format!("Ignored a DCC offer from {nick} with no port and no token"),
            )];
        }
        let id = self.new_id();
        let name = offered.as_deref().map(proto::safe_file_name);
        let what = match (&name, size) {
            (Some(name), Some(size)) => format!("the file {name} ({})", human_size(size)),
            (Some(name), None) => format!("the file {name}"),
            (None, _) => "a chat".to_owned(),
        };
        let mut text = format!("{nick} offers {what}");
        if let (Some(name), Some(sent)) = (&name, &offered)
            && name != sent
        {
            text.push_str(" (its name made safe)");
        }
        let take = match kind {
            Kind::Chat => format!("/dcc chat {nick}"),
            _ => format!("/dcc get {nick}"),
        };
        let _ = write!(text, ": {take} to accept, /dcc close {id} to decline");
        self.transfers.push(Transfer {
            id,
            network,
            nick: nick.to_owned(),
            kind,
            name,
            path: None,
            size,
            bytes: 0,
            offset: 0,
            status: Status::Offered,
            started: None,
            offered,
            addr: Some(addr),
            port,
            token,
            passive: port == 0,
            created: Instant::now(),
            task: None,
            lines: None,
            resume_at: Arc::new(AtomicU64::new(0)),
        });
        vec![
            Dcc::note(network, LineKind::Info, text.clone()),
            Out::Notify {
                network,
                nick: nick.to_owned(),
                text,
            },
        ]
    }

    /// The other side answered our passive offer: connect to them.
    fn passive_answer(
        &mut self,
        network: NetworkId,
        nick: &str,
        kind: Kind,
        (addr, port, token): (IpAddr, u16, &str),
        ctx: Context<'_>,
    ) -> Vec<Out> {
        if ctx.advertise.is_none() {
            return vec![Dcc::note(network, LineKind::Error, no_advertise())];
        }
        let Some(index) = self.transfers.iter().position(|t| {
            t.network == network
                && t.kind == kind
                && t.status == Status::Waiting
                && t.passive
                && t.token.as_deref() == Some(token)
                && eq_nick(&t.nick, nick)
        }) else {
            return vec![Dcc::note(
                network,
                LineKind::Info,
                format!("Ignored a DCC answer from {nick} that matches nothing we offered"),
            )];
        };
        if let Err(error) = check_target(addr, port) {
            return vec![Dcc::note(
                network,
                LineKind::Error,
                format!("DCC with {nick}: {error}"),
            )];
        }
        let link = Link::Connect(SocketAddr::new(addr, port));
        let id = self.transfers.get(index).map_or(DccId(0), |t| t.id);
        self.start(id, link);
        Vec::new()
    }

    /// They want to resume a file we offered from `position`.
    fn on_resume(
        &mut self,
        network: NetworkId,
        nick: &str,
        file: &str,
        port: u16,
        position: u64,
        token: Option<String>,
    ) -> Vec<Out> {
        let found = self.transfers.iter_mut().find(|t| {
            t.network == network
                && t.kind == Kind::Send
                && t.status == Status::Waiting
                && eq_nick(&t.nick, nick)
                && t.offered.as_deref() == Some(file)
                && (t.port == port || (port == 0 && t.token == token))
        });
        let Some(transfer) = found else {
            return vec![Dcc::note(
                network,
                LineKind::Info,
                format!(
                    "Ignored a DCC resume request from {nick} for {file}, which we didn't offer them"
                ),
            )];
        };
        if transfer.size.is_none_or(|size| position >= size) {
            return vec![Dcc::note(
                network,
                LineKind::Info,
                format!("{nick} asked to resume {file} past its end; ignored"),
            )];
        }
        transfer.offset = position;
        transfer.bytes = position;
        transfer.resume_at.store(position, Ordering::SeqCst);
        vec![
            Out::Ctcp {
                network,
                nick: nick.to_owned(),
                params: Request::Accept {
                    file: file.to_owned(),
                    port,
                    position,
                    token,
                }
                .encode(),
            },
            Dcc::note(
                network,
                LineKind::Info,
                format!("{nick} resumes {file} from {}", human_size(position)),
            ),
        ]
    }

    /// They agreed to our resume: carry on from `position`.
    #[allow(clippy::too_many_arguments, reason = "the fields of the request")]
    fn on_accept(
        &mut self,
        network: NetworkId,
        nick: &str,
        file: &str,
        port: u16,
        position: u64,
        token: Option<&String>,
        ctx: Context<'_>,
    ) -> Vec<Out> {
        let Some(index) = self.transfers.iter().position(|t| {
            t.network == network
                && t.kind == Kind::Get
                && t.status == Status::Resuming
                && eq_nick(&t.nick, nick)
                && t.offered.as_deref() == Some(file)
                && (t.port == port || (port == 0 && t.token.as_ref() == token))
        }) else {
            return vec![Dcc::note(
                network,
                LineKind::Info,
                format!(
                    "Ignored a DCC accept from {nick} for {file}, which we didn't ask to resume"
                ),
            )];
        };
        let Some(transfer) = self.transfers.get(index) else {
            return Vec::new();
        };
        if position != transfer.offset {
            return vec![Dcc::note(
                network,
                LineKind::Error,
                format!("{nick} would resume {file} from a different place; not resuming"),
            )];
        }
        let id = transfer.id;
        self.connect_get(id, ctx)
    }

    /// Starts receiving an offered (or resumed) file.
    fn connect_get(&mut self, id: DccId, ctx: Context<'_>) -> Vec<Out> {
        let Some(transfer) = self.get_mut(id) else {
            return Vec::new();
        };
        let network = transfer.network;
        let nick = transfer.nick.clone();
        // Connecting out is as revealing as listening: not through a proxy.
        if ctx.advertise.is_none() {
            transfer.status = Status::Failed(no_advertise());
            return vec![Dcc::note(network, LineKind::Error, no_advertise())];
        }
        if !transfer.passive {
            let (Some(addr), port) = (transfer.addr, transfer.port) else {
                return Vec::new();
            };
            if let Err(error) = check_target(addr, port) {
                transfer.status = Status::Failed(error.clone());
                return vec![Dcc::note(
                    network,
                    LineKind::Error,
                    format!("DCC from {nick}: {error}"),
                )];
            }
            self.start(id, Link::Connect(SocketAddr::new(addr, port)));
            return Vec::new();
        }
        // Passive: we listen, and tell them where.
        let Some(advertise) = ctx.advertise else {
            return vec![Dcc::note(network, LineKind::Error, no_advertise())];
        };
        match listen(ctx.ports, advertise) {
            Ok((listener, port)) => {
                let Some(transfer) = self.get_mut(id) else {
                    return Vec::new();
                };
                let reply = Request::Send {
                    file: transfer.offered.clone().unwrap_or_default(),
                    addr: advertise,
                    port,
                    size: transfer.size,
                    token: transfer.token.clone(),
                };
                self.start(id, Link::Listen(listener));
                let mut out = vec![Out::Ctcp {
                    network,
                    nick,
                    params: reply.encode(),
                }];
                out.extend(self.reveal_note(network, advertise));
                out
            }
            Err(error) => vec![Dcc::note(
                network,
                LineKind::Error,
                format!("DCC: cannot listen for {nick}'s file: {error}"),
            )],
        }
    }

    /// The newest offer of `kind` matching `nick` and (for files) `name`.
    fn find_offer(
        &self,
        network: NetworkId,
        kind: Kind,
        nick: Option<&str>,
        name: Option<&str>,
        statuses: &[Status],
    ) -> Option<DccId> {
        self.transfers
            .iter()
            .rev()
            .find(|t| {
                t.network == network
                    && t.kind == kind
                    && statuses.contains(&t.status)
                    && nick.is_none_or(|n| eq_nick(&t.nick, n))
                    && name.is_none_or(|n| {
                        t.name.as_deref() == Some(n) || t.offered.as_deref() == Some(n)
                    })
            })
            .map(|t| t.id)
    }

    /// `/dcc get [nick [file]]`: accept an offered file.
    pub fn get(
        &mut self,
        network: NetworkId,
        nick: Option<&str>,
        name: Option<&str>,
        ctx: Context<'_>,
    ) -> Result<Vec<Out>, String> {
        self.tidy();
        // Any DCC connection, in or out, would bypass a proxy.
        ctx.advertise.ok_or_else(no_advertise)?;
        let id = self
            .find_offer(network, Kind::Get, nick, name, &[Status::Offered])
            .ok_or("No file is offered like that; /dcc list shows what is")?;
        let transfer = self.get_mut(id).ok_or("gone")?;
        let name = transfer.name.clone().unwrap_or_else(|| "download".into());
        let (file, part) = create_part(ctx.dir, &name)
            .map_err(|e| format!("Cannot create a file in {}: {e}", ctx.dir.display()))?;
        drop(file);
        transfer.path = Some(part.clone());
        let nick = transfer.nick.clone();
        let mut out = vec![Dcc::note(
            network,
            LineKind::Info,
            format!("Receiving {name} from {nick} into {}", part.display()),
        )];
        out.extend(self.connect_get(id, ctx));
        Ok(out)
    }

    /// `/dcc resume [nick [file]]`: carry on with a file received in part,
    /// from where its `.part` file ends.
    pub fn resume(
        &mut self,
        network: NetworkId,
        nick: Option<&str>,
        name: Option<&str>,
        ctx: Context<'_>,
    ) -> Result<Vec<Out>, String> {
        self.tidy();
        ctx.advertise.ok_or_else(no_advertise)?;
        let id = self
            .find_offer(network, Kind::Get, nick, name, &[Status::Offered])
            .ok_or("No file is offered like that; ask them to offer it again, then /dcc resume")?;
        let transfer = self.get_mut(id).ok_or("gone")?;
        let name = transfer.name.clone().unwrap_or_else(|| "download".into());
        let part = ctx.dir.join(format!("{name}.part"));
        let have = std::fs::metadata(&part)
            .map_err(|_| {
                format!(
                    "There is no {} to resume; /dcc get starts afresh",
                    part.display()
                )
            })?
            .len();
        if transfer.size.is_some_and(|size| have >= size) {
            return Err(format!("{} is already complete", part.display()));
        }
        transfer.path = Some(part);
        transfer.offset = have;
        transfer.bytes = have;
        transfer.status = Status::Resuming;
        let request = Request::Resume {
            file: transfer.offered.clone().unwrap_or_default(),
            port: transfer.port,
            position: have,
            token: transfer.token.clone(),
        };
        Ok(vec![
            Out::Ctcp {
                network,
                nick: transfer.nick.clone(),
                params: request.encode(),
            },
            Dcc::note(
                network,
                LineKind::Info,
                format!(
                    "Asking {} to resume {name} from {}",
                    transfer.nick,
                    human_size(have)
                ),
            ),
        ])
    }

    /// `/dcc send [-passive] nick path`: offer a file.
    pub fn send(
        &mut self,
        network: NetworkId,
        nick: &str,
        path: &Path,
        passive: bool,
        ctx: Context<'_>,
    ) -> Result<Vec<Out>, String> {
        self.tidy();
        let advertise = ctx.advertise.ok_or_else(no_advertise)?;
        let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if !meta.is_file() {
            return Err(format!("{} is not a file", path.display()));
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .map_or_else(|| "file".into(), proto::safe_file_name);
        let size = meta.len();
        let id = self.new_id();
        let (port, token, link) = if passive {
            (0, Some(self.new_token()), None)
        } else {
            let (listener, port) =
                listen(ctx.ports, advertise).map_err(|e| format!("Cannot listen: {e}"))?;
            (port, None, Some(Link::Listen(listener)))
        };
        let request = Request::Send {
            file: name.clone(),
            addr: advertise,
            port,
            size: Some(size),
            token: token.clone(),
        };
        self.transfers.push(Transfer {
            id,
            network,
            nick: nick.to_owned(),
            kind: Kind::Send,
            name: Some(name.clone()),
            path: Some(path.to_owned()),
            size: Some(size),
            bytes: 0,
            offset: 0,
            status: Status::Waiting,
            started: None,
            offered: Some(name.clone()),
            addr: None,
            port,
            token,
            passive,
            created: Instant::now(),
            task: None,
            lines: None,
            resume_at: Arc::new(AtomicU64::new(0)),
        });
        if let Some(link) = link {
            self.start(id, link);
            if let Some(t) = self.get_mut(id) {
                t.status = Status::Waiting;
            }
        }
        let mut out = vec![
            Out::Ctcp {
                network,
                nick: nick.to_owned(),
                params: request.encode(),
            },
            Dcc::note(
                network,
                LineKind::Info,
                format!(
                    "Offering {name} ({}) to {nick}{}",
                    human_size(size),
                    if passive { " (passive)" } else { "" }
                ),
            ),
        ];
        if !passive {
            out.extend(self.reveal_note(network, advertise));
        }
        Ok(out)
    }

    /// `/dcc chat [-passive] nick`: accept their chat offer, or offer one.
    pub fn chat(
        &mut self,
        network: NetworkId,
        nick: &str,
        passive: bool,
        ctx: Context<'_>,
    ) -> Result<Vec<Out>, String> {
        self.tidy();
        // Accepting connects out, offering listens: either bypasses a proxy.
        ctx.advertise.ok_or_else(no_advertise)?;
        if let Some(id) = self.find_offer(network, Kind::Chat, Some(nick), None, &[Status::Offered])
        {
            let transfer = self.get_mut(id).ok_or("gone")?;
            if !transfer.passive {
                let (addr, port) = (transfer.addr.ok_or("no address")?, transfer.port);
                if let Err(error) = check_target(addr, port) {
                    transfer.status = Status::Failed(error.clone());
                    return Err(format!("DCC chat with {nick}: {error}"));
                }
                self.start(id, Link::Connect(SocketAddr::new(addr, port)));
                return Ok(Vec::new());
            }
            let advertise = ctx.advertise.ok_or_else(no_advertise)?;
            let (listener, port) =
                listen(ctx.ports, advertise).map_err(|e| format!("Cannot listen: {e}"))?;
            let token = self.get_mut(id).and_then(|t| t.token.clone());
            self.start(id, Link::Listen(listener));
            let mut out = vec![Out::Ctcp {
                network,
                nick: nick.to_owned(),
                params: Request::Chat {
                    addr: advertise,
                    port,
                    token,
                }
                .encode(),
            }];
            out.extend(self.reveal_note(network, advertise));
            return Ok(out);
        }
        let advertise = ctx.advertise.ok_or_else(no_advertise)?;
        let id = self.new_id();
        let (port, token, link) = if passive {
            (0, Some(self.new_token()), None)
        } else {
            let (listener, port) =
                listen(ctx.ports, advertise).map_err(|e| format!("Cannot listen: {e}"))?;
            (port, None, Some(Link::Listen(listener)))
        };
        self.transfers.push(Transfer {
            id,
            network,
            nick: nick.to_owned(),
            kind: Kind::Chat,
            name: None,
            path: None,
            size: None,
            bytes: 0,
            offset: 0,
            status: Status::Waiting,
            started: None,
            offered: None,
            addr: None,
            port,
            token: token.clone(),
            passive,
            created: Instant::now(),
            task: None,
            lines: None,
            resume_at: Arc::new(AtomicU64::new(0)),
        });
        if let Some(link) = link {
            self.start(id, link);
            if let Some(t) = self.get_mut(id) {
                t.status = Status::Waiting;
            }
        }
        let mut out = vec![
            Out::Ctcp {
                network,
                nick: nick.to_owned(),
                params: Request::Chat {
                    addr: advertise,
                    port,
                    token,
                }
                .encode(),
            },
            Dcc::note(
                network,
                LineKind::Info,
                format!("Offering a chat to {nick}; it opens as ={nick} when they accept"),
            ),
        ];
        if !passive {
            out.extend(self.reveal_note(network, advertise));
        }
        Ok(out)
    }

    /// A line typed in a DCC chat with `nick`.
    pub fn say(&mut self, network: NetworkId, nick: &str, text: &str) -> Result<Vec<Out>, String> {
        let transfer = self
            .transfers
            .iter()
            .find(|t| {
                t.network == network
                    && t.kind == Kind::Chat
                    && t.status == Status::Active
                    && eq_nick(&t.nick, nick)
            })
            .ok_or_else(|| {
                format!("No DCC chat with {nick} is open; /dcc chat {nick} offers one")
            })?;
        let lines = transfer.lines.as_ref().ok_or("the chat isn't connected")?;
        let mut out = Vec::new();
        for line in text.split(['\r', '\n']).filter(|l| !l.is_empty()) {
            lines
                .send(line.to_owned())
                .map_err(|_| "the chat has closed")?;
            out.push(Out::Chat {
                network,
                nick: nick.to_owned(),
                text: line.to_owned(),
                own: true,
            });
        }
        Ok(out)
    }

    /// `/dcc close <id|nick>`: stop transfers or chats, or decline offers.
    pub fn close(&mut self, selector: &str) -> Result<Vec<Out>, String> {
        let by_id = selector.parse::<u64>().ok().map(DccId);
        let mut out = Vec::new();
        self.transfers.retain(|t| {
            let hit = Some(t.id) == by_id || eq_nick(&t.nick, selector);
            if hit {
                if let Some(task) = &t.task {
                    task.abort();
                }
                out.push(Dcc::note(
                    t.network,
                    LineKind::Info,
                    format!(
                        "Closed DCC {} {} with {}",
                        t.id,
                        describe_kind(t.kind),
                        t.nick
                    ),
                ));
            }
            !hit
        });
        if out.is_empty() {
            return Err(format!("No DCC {selector}; /dcc list shows them"));
        }
        Ok(out)
    }

    fn start(&mut self, id: DccId, link: Link) {
        let tx = self.tx.clone();
        let Some(transfer) = self.get_mut(id) else {
            return;
        };
        transfer.status = Status::Connecting;
        let handle = match transfer.kind {
            Kind::Get => {
                let Some(part) = transfer.path.clone() else {
                    return;
                };
                let job = Receive {
                    id,
                    part,
                    offset: transfer.offset,
                    size: transfer.size,
                };
                tokio::spawn(job.run(link, tx))
            }
            Kind::Send => {
                let Some(path) = transfer.path.clone() else {
                    return;
                };
                let job = SendFile {
                    id,
                    path,
                    size: transfer.size.unwrap_or(0),
                    offset: Arc::clone(&transfer.resume_at),
                };
                tokio::spawn(job.run(link, tx))
            }
            Kind::Chat => {
                let (lines, rx) = mpsc::unbounded_channel();
                transfer.lines = Some(lines);
                tokio::spawn(chat(id, link, tx, rx))
            }
        };
        transfer.task = Some(handle.abort_handle());
    }

    /// Takes in what a connection task reported.
    pub fn on_event(&mut self, event: Event) -> Vec<Out> {
        let id = match &event {
            Event::Connected(id)
            | Event::Progress(id, _)
            | Event::Finished(id, _)
            | Event::Failed(id, _)
            | Event::Line(id, _)
            | Event::Closed(id) => *id,
        };
        let Some(transfer) = self.get_mut(id) else {
            return Vec::new();
        };
        let (network, nick) = (transfer.network, transfer.nick.clone());
        let name = transfer.name.clone().unwrap_or_default();
        match event {
            Event::Connected(_) => {
                transfer.status = Status::Active;
                transfer.started = Some(Instant::now());
                let text = match transfer.kind {
                    Kind::Chat => format!("DCC chat with {nick} open in ={nick}"),
                    Kind::Get => format!("Receiving {name} from {nick}"),
                    Kind::Send => format!("Sending {name} to {nick}"),
                };
                let open = transfer.kind == Kind::Chat;
                let mut out = vec![Dcc::note(network, LineKind::Info, text)];
                if open {
                    out.push(Out::Open { network, nick });
                }
                out
            }
            Event::Progress(_, bytes) => {
                transfer.bytes = bytes;
                Vec::new()
            }
            Event::Finished(_, bytes) => finished(transfer, bytes, network, &nick, &name),
            Event::Failed(_, reason) => {
                transfer.status = Status::Failed(reason.clone());
                transfer.task = None;
                let what = describe_kind(transfer.kind);
                vec![Dcc::note(
                    network,
                    LineKind::Error,
                    format!("DCC {what} with {nick} failed: {reason}"),
                )]
            }
            Event::Line(_, text) => vec![Out::Chat {
                network,
                nick,
                text,
                own: false,
            }],
            Event::Closed(_) => {
                transfer.status = Status::Done;
                transfer.task = None;
                transfer.lines = None;
                vec![Dcc::note(
                    network,
                    LineKind::Info,
                    format!("DCC chat with {nick} closed"),
                )]
            }
        }
    }
}

/// A transfer finished: name a received file, and say so.
fn finished(
    transfer: &mut Transfer,
    bytes: u64,
    network: NetworkId,
    nick: &str,
    name: &str,
) -> Vec<Out> {
    transfer.bytes = bytes;
    transfer.status = Status::Done;
    transfer.task = None;
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a display figure, never negative"
    )]
    let rate = transfer
        .rate()
        .map(|r| format!(" at {}/s", human_size(r as u64)))
        .unwrap_or_default();
    if transfer.kind == Kind::Get
        && let Some(part) = transfer.path.clone()
    {
        match finish_part(&part) {
            Ok(path) => {
                let text = format!(
                    "Received {name} ({}) from {nick}{rate}: {}",
                    human_size(bytes),
                    path.display()
                );
                transfer.path = Some(path);
                return vec![Dcc::note(network, LineKind::Info, text)];
            }
            Err(error) => {
                let text = format!(
                    "Received {name}, but cannot name it: {error}; it is {}",
                    part.display()
                );
                transfer.status = Status::Failed(error.to_string());
                return vec![Dcc::note(network, LineKind::Error, text)];
            }
        }
    }
    vec![Dcc::note(
        network,
        LineKind::Info,
        format!("Sent {name} ({}) to {nick}{rate}", human_size(bytes)),
    )]
}

fn describe_kind(kind: Kind) -> &'static str {
    match kind {
        Kind::Get => "file",
        Kind::Send => "send",
        Kind::Chat => "chat",
    }
}

fn no_advertise() -> String {
    "DCC is off for this network: it goes through a proxy, which a direct connection would \
     bypass, revealing your address (set dcc.ip to allow it anyway)"
        .into()
}

/// How a connection is made: we connect, or we wait for them.
#[derive(Debug)]
enum Link {
    Connect(SocketAddr),
    Listen(TcpListener),
}

async fn establish(link: Link) -> Result<TcpStream, String> {
    let stream = match link {
        Link::Connect(addr) => tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr))
            .await
            .map_err(|_| "connecting timed out".to_owned())?
            .map_err(|e| format!("cannot connect: {e}"))?,
        Link::Listen(listener) => {
            tokio::time::timeout(OFFER_TIMEOUT, listener.accept())
                .await
                .map_err(|_| "nobody connected in time".to_owned())?
                .map_err(|e| format!("cannot accept: {e}"))?
                .0
        }
    };
    let _ = stream.set_nodelay(true);
    Ok(stream)
}

/// Receiving a file into its `.part` file.
struct Receive {
    id: DccId,
    part: PathBuf,
    offset: u64,
    size: Option<u64>,
}

impl Receive {
    async fn run(self, link: Link, tx: mpsc::UnboundedSender<Event>) {
        let id = self.id;
        let result = self.receive(link, &tx).await;
        let _ = tx.send(match result {
            Ok(bytes) => Event::Finished(id, bytes),
            Err(reason) => Event::Failed(id, reason),
        });
    }

    async fn receive(self, link: Link, tx: &mpsc::UnboundedSender<Event>) -> Result<u64, String> {
        // Resuming appends to the part file; otherwise it was just created.
        let mut file = tokio::fs::OpenOptions::new()
            .append(true)
            .open(&self.part)
            .await
            .map_err(|e| format!("cannot open {}: {e}", self.part.display()))?;
        let mut stream = establish(link).await?;
        let _ = tx.send(Event::Connected(self.id));
        let mut got = self.offset;
        let mut buf = vec![0u8; CHUNK];
        let mut reported = Instant::now();
        loop {
            if self.size.is_some_and(|size| got >= size) {
                break;
            }
            let n = tokio::time::timeout(IDLE_TIMEOUT, stream.read(&mut buf))
                .await
                .map_err(|_| "no data for two minutes".to_owned())?
                .map_err(|e| format!("the connection broke: {e}"))?;
            if n == 0 {
                break;
            }
            let room = self.size.map_or(n, |size| {
                usize::try_from(size - got).unwrap_or(usize::MAX).min(n)
            });
            let chunk = buf.get(..room).unwrap_or_default();
            file.write_all(chunk)
                .await
                .map_err(|e| format!("cannot write {}: {e}", self.part.display()))?;
            got += room as u64;
            if room < n {
                file.flush().await.ok();
                return Err(format!(
                    "they sent more than the {} offered; stopped there",
                    human_size(self.size.unwrap_or(0))
                ));
            }
            // Acknowledge, as older senders wait for it.
            if stream.write_all(&proto::ack(got)).await.is_err() {
                break;
            }
            if reported.elapsed() >= PROGRESS_EVERY {
                reported = Instant::now();
                let _ = tx.send(Event::Progress(self.id, got));
            }
        }
        file.flush()
            .await
            .map_err(|e| format!("cannot write {}: {e}", self.part.display()))?;
        if let Some(size) = self.size
            && got < size
        {
            return Err(format!(
                "the connection closed after {} of {}; /dcc resume can carry on when they offer it again",
                human_size(got),
                human_size(size)
            ));
        }
        Ok(got)
    }
}

/// Sending a file.
struct SendFile {
    id: DccId,
    path: PathBuf,
    size: u64,
    /// Where to start: 0, or where a resume agreed.
    offset: Arc<AtomicU64>,
}

impl SendFile {
    async fn run(self, link: Link, tx: mpsc::UnboundedSender<Event>) {
        let id = self.id;
        let result = self.send(link, &tx).await;
        let _ = tx.send(match result {
            Ok(bytes) => Event::Finished(id, bytes),
            Err(reason) => Event::Failed(id, reason),
        });
    }

    async fn send(self, link: Link, tx: &mpsc::UnboundedSender<Event>) -> Result<u64, String> {
        let stream = establish(link).await?;
        let _ = tx.send(Event::Connected(self.id));
        // A resume, if any, has been agreed by the time they connect.
        let offset = self.offset.load(Ordering::SeqCst);
        let mut file = tokio::fs::File::open(&self.path)
            .await
            .map_err(|e| format!("cannot open {}: {e}", self.path.display()))?;
        file.seek(io::SeekFrom::Start(offset))
            .await
            .map_err(|e| format!("cannot seek {}: {e}", self.path.display()))?;
        let (mut reader, mut writer) = stream.into_split();
        let size = self.size;
        // Read acknowledgements as they come, so the other side never
        // blocks writing them while we write the file.
        let acks = tokio::spawn(async move {
            let mut ack = [0u8; 4];
            loop {
                match reader.read_exact(&mut ack).await {
                    Ok(_) if ack == proto::ack(size) => return true,
                    Ok(_) => {}
                    Err(_) => return false,
                }
            }
        });
        let mut sent = offset;
        let mut buf = vec![0u8; CHUNK];
        let mut reported = Instant::now();
        while sent < size {
            let n = file
                .read(&mut buf)
                .await
                .map_err(|e| format!("cannot read {}: {e}", self.path.display()))?;
            if n == 0 {
                return Err(format!("{} got shorter while sending", self.path.display()));
            }
            let chunk = buf.get(..n).unwrap_or_default();
            tokio::time::timeout(IDLE_TIMEOUT, writer.write_all(chunk))
                .await
                .map_err(|_| "they stopped reading for two minutes".to_owned())?
                .map_err(|e| format!("the connection broke: {e}"))?;
            sent += n as u64;
            if reported.elapsed() >= PROGRESS_EVERY {
                reported = Instant::now();
                let _ = tx.send(Event::Progress(self.id, sent));
            }
        }
        // Wait for the last acknowledgement, or for them to close.
        let _ = tokio::time::timeout(FINAL_ACK_TIMEOUT, acks).await;
        Ok(sent)
    }
}

async fn chat(
    id: DccId,
    link: Link,
    tx: mpsc::UnboundedSender<Event>,
    mut lines: mpsc::UnboundedReceiver<String>,
) {
    let stream = match establish(link).await {
        Ok(stream) => stream,
        Err(reason) => {
            let _ = tx.send(Event::Failed(id, reason));
            return;
        }
    };
    let _ = tx.send(Event::Connected(id));
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut buf = Vec::new();
    loop {
        tokio::select! {
            read = async {
                buf.clear();
                let mut limited = (&mut reader).take(MAX_CHAT_LINE as u64);
                limited.read_until(b'\n', &mut buf).await
            } => match read {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let text = String::from_utf8_lossy(&buf);
                    let text = text.trim_end_matches(['\r', '\n']);
                    if !text.is_empty() {
                        let _ = tx.send(Event::Line(id, text.to_owned()));
                    }
                }
            },
            outgoing = lines.recv() => match outgoing {
                Some(text) => {
                    if writer.write_all(format!("{text}\n").as_bytes()).await.is_err() {
                        break;
                    }
                }
                None => break,
            },
        }
    }
    let _ = tx.send(Event::Closed(id));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1500), "1.5 kB");
        assert_eq!(human_size(2_300_000), "2.3 MB");
        assert_eq!(human_size(u64::MAX), "18446744.1 TB");
    }

    #[test]
    fn files_are_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"old").unwrap();
        let (_, first) = create_part(dir.path(), "a.txt").unwrap();
        assert_eq!(first, dir.path().join("a.1.txt.part"));
        let (_, second) = create_part(dir.path(), "a.txt").unwrap();
        assert_eq!(
            second,
            dir.path().join("a.2.txt.part"),
            "a .part counts as taken"
        );
        // Finishing never replaces a file that appeared meanwhile.
        std::fs::write(dir.path().join("a.1.txt"), b"appeared").unwrap();
        std::fs::write(&first, b"new").unwrap();
        let done = finish_part(&first).unwrap();
        assert_eq!(done, dir.path().join("a.1.1.txt"));
        assert_eq!(
            std::fs::read(dir.path().join("a.1.txt")).unwrap(),
            b"appeared"
        );
        assert_eq!(std::fs::read(done).unwrap(), b"new");
        assert!(!first.exists());
        let names: Vec<String> = candidates("noext").take(3).collect();
        assert_eq!(names, ["noext", "noext.1", "noext.2"]);
    }

    #[test]
    fn where_offers_may_point() {
        assert!(check_target("127.0.0.1".parse().unwrap(), 5000).is_ok());
        assert!(check_target("10.0.0.1".parse().unwrap(), 1024).is_ok());
        for (addr, port) in [
            ("127.0.0.1", 22),
            ("0.0.0.0", 5000),
            ("255.255.255.255", 5000),
            ("224.0.0.1", 5000),
            ("::", 5000),
            ("ff02::1", 5000),
        ] {
            assert!(
                check_target(addr.parse().unwrap(), port).is_err(),
                "{addr}:{port}"
            );
        }
        for private in [
            "192.168.1.5",
            "10.1.2.3",
            "172.16.0.1",
            "100.64.1.1",
            "169.254.1.1",
            "fd00::1",
            "fe80::1",
        ] {
            assert!(is_private(private.parse().unwrap()), "{private}");
        }
        for public in ["8.8.8.8", "100.128.0.1", "2001:db8::1"] {
            assert!(!is_private(public.parse().unwrap()), "{public}");
        }
    }

    #[tokio::test]
    async fn no_offers_without_an_address_to_advertise() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f");
        std::fs::write(&file, b"x").unwrap();
        let mut dcc = Dcc::default();
        let ctx = Context {
            dir: dir.path(),
            ports: None,
            advertise: None,
        };
        let error = dcc
            .send(NetworkId(1), "bob", &file, false, ctx)
            .unwrap_err();
        assert!(error.contains("proxy"), "{error}");
        assert!(
            dcc.chat(NetworkId(1), "bob", false, ctx)
                .unwrap_err()
                .contains("proxy")
        );
        assert!(dcc.list().is_empty());

        // A port range is honoured.
        let ctx = Context {
            dir: dir.path(),
            ports: Some((47_100, 47_110)),
            advertise: Some(IpAddr::V4(Ipv4Addr::LOCALHOST)),
        };
        let out = dcc.send(NetworkId(1), "bob", &file, false, ctx).unwrap();
        let Some(Out::Ctcp { params, .. }) = out.first() else {
            panic!("{out:?}")
        };
        let Some(Request::Send { port, .. }) = Request::parse(params) else {
            panic!("{params}")
        };
        assert!((47_100..=47_110).contains(&port));
        assert!(
            out.iter()
                .any(|o| matches!(o, Out::Note { text, .. } if text.contains("your IP address"))),
            "the first offer warns"
        );
        let again = dcc.send(NetworkId(1), "bob", &file, true, ctx).unwrap();
        assert!(
            !again
                .iter()
                .any(|o| matches!(o, Out::Note { text, .. } if text.contains("your IP address"))),
            "once"
        );
        assert!(
            dcc.send(NetworkId(1), "bob", dir.path(), false, ctx)
                .is_err(),
            "not a file"
        );
    }

    /// Accepting someone's offer connects out directly, which would bypass
    /// a proxy just as much as offering: refused, before any file is made.
    #[tokio::test]
    async fn no_connections_out_without_an_address_to_advertise() {
        let dir = tempfile::tempdir().unwrap();
        let mut dcc = Dcc::default();
        let ctx = Context {
            dir: dir.path(),
            ports: None,
            advertise: None,
        };
        let offered = |dcc: &mut Dcc, request| {
            dcc.on_request(NetworkId(1), "bob", request, ctx);
        };
        offered(
            &mut dcc,
            Request::Send {
                file: "f.txt".into(),
                addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
                port: 5000,
                size: Some(1),
                token: None,
            },
        );
        offered(
            &mut dcc,
            Request::Chat {
                addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
                port: 5001,
                token: None,
            },
        );
        assert_eq!(dcc.list().len(), 2, "offers are still listed");
        for error in [
            dcc.get(NetworkId(1), Some("bob"), None, ctx).unwrap_err(),
            dcc.resume(NetworkId(1), Some("bob"), None, ctx)
                .unwrap_err(),
            dcc.chat(NetworkId(1), "bob", false, ctx).unwrap_err(),
        ] {
            assert!(error.contains("proxy"), "{error}");
        }
        assert!(
            dcc.list()
                .iter()
                .all(|t| t.status == Status::Offered && t.task.is_none())
        );
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            0,
            "no .part made"
        );
        // A forged answer to a passive offer connects nowhere either.
        let out = dcc.on_request(
            NetworkId(1),
            "bob",
            Request::Send {
                file: "f.txt".into(),
                addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
                port: 5002,
                size: Some(1),
                token: Some("7".into()),
            },
            ctx,
        );
        assert!(
            matches!(&out[..], [Out::Note { text, .. }] if text.contains("proxy")),
            "{out:?}"
        );
    }
}
