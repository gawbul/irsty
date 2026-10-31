//! Terminal user interface for Irsty, built on ratatui.
//!
//! Renders from `irsty-core` state and turns key, mouse and paste events into
//! actions. It never touches sockets.
#![forbid(unsafe_code)]

pub mod command;
pub mod input;
pub mod theme;

pub use irsty_core as core;
