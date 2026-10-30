//! Connection management and client state for Irsty.
//!
//! Owns networks, buffers, channels and users, turns network and input events into
//! actions, and applies them in a single update step. It has no UI dependencies, so
//! a headless mode or another frontend can sit on top of it.
#![forbid(unsafe_code)]

pub mod config;
pub mod net;

pub use config::Config;

pub use irsty_proto as proto;
