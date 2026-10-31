//! Terminal user interface for Irsty, built on ratatui.
//!
//! Renders from `irsty-core` state and turns key, mouse and paste events into
//! actions. It never touches sockets.
#![forbid(unsafe_code)]

pub mod app;
pub mod command;
pub mod input;
pub mod run;
pub mod scrollback;
pub mod terminal;
pub mod theme;
pub mod ui;
pub mod view;

pub use irsty_core as core;
pub use run::run;
