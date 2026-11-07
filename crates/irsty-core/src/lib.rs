//! Connection management and client state for Irsty.
//!
//! Owns networks, buffers, channels and users, runs a connection task per
//! network, and applies network events in a single update step
//! ([`state::State::apply`]). It also logs, and reads and edits the config.
//! It has no UI dependencies, so a headless mode or another frontend can sit on
//! top of it.
//!
//! **This crate is Irsty's own internals**, published so that `cargo install
//! irsty` works. It makes no API stability promise: anything may change in
//! any release, and its version moves with the app's.
#![forbid(unsafe_code)]

pub mod batch;
pub mod client;
pub mod config;
pub mod config_file;
pub mod dcc;
pub mod event;
pub mod log;
pub mod net;
pub mod redact;
pub mod replies;
pub mod search;
pub mod state;

pub use client::Client;
pub use config::Config;

pub use irsty_proto as proto;
