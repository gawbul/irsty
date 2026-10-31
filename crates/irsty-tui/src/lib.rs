//! Terminal user interface for Irsty, built on ratatui.
//!
//! Renders from `irsty-core` state and turns key, mouse, paste and focus events
//! into calls on the client, through a configurable keymap. It never touches
//! sockets.
#![forbid(unsafe_code)]

pub mod app;
pub mod command;
pub mod complete;
pub mod input;
pub mod keymap;
pub mod notify;
pub mod run;
pub mod scrollback;
pub mod terminal;
pub mod theme;
pub mod ui;
pub mod urls;
pub mod view;

pub use irsty_core as core;
pub use run::run;
