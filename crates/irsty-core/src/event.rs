//! Messages between connection tasks and the app loop.

use std::time::Duration;

use irsty_proto::Message;

/// Identifies a network for the lifetime of the process.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NetworkId(pub u32);

/// What a connection task reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetEvent {
    /// Starting a connection attempt.
    Connecting {
        /// `host:port`, with `+` before the port for TLS.
        server: String,
    },
    /// The transport is up and registration has started.
    Connected {
        /// As in `Connecting`.
        server: String,
        /// Certificate verification is off for this server.
        insecure: bool,
    },
    /// The server accepted registration (`RPL_WELCOME`).
    Registered {
        /// The nick we ended up with.
        nick: String,
    },
    /// A message from the server. `PING` and `PONG` are handled internally and
    /// not reported.
    Message(Message),
    /// The set of enabled capabilities changed.
    Caps {
        /// Every enabled capability.
        enabled: Vec<String>,
        /// The values the server gave enabled capabilities, where not empty
        /// (such as `draft/multiline`'s limits).
        values: Vec<(String, String)>,
    },
    /// SASL finished.
    Sasl(Result<Option<String>, String>),
    /// Latest round-trip time to the server.
    Lag(Duration),
    /// Something went wrong that did not end the connection.
    Error(String),
    /// The connection ended.
    Disconnected {
        /// Why.
        reason: String,
        /// When the next attempt starts, or `None` if not reconnecting.
        reconnect_in: Option<Duration>,
    },
}

/// What the app asks a connection task to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetCommand {
    /// Send a message, subject to rate limiting.
    Send(Message),
    /// Send `QUIT` and stop for good.
    Quit(Option<String>),
    /// Drop the connection (if any) and connect again now.
    Reconnect,
}
