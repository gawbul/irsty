//! Networking: framing, transport, rate limiting, backoff and the connection task.

pub mod backoff;
pub mod codec;
pub mod connection;
pub mod dial;
pub mod ratelimit;
pub mod socks;
pub mod transport;
