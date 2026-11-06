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
pub mod panes;
pub mod run;
pub mod script;
pub mod scrollback;
pub mod terminal;
pub mod theme;
pub mod ui;
pub mod urls;
pub mod view;

pub use irsty_core as core;
pub use run::run;

/// What is wrong with the parts of a config the UI reads, which parsing
/// alone doesn't check: key bindings, the theme and the saved layout.
pub fn check_config(config: &irsty_core::Config) -> Vec<String> {
    let mut problems = keymap::Keymap::new(&config.ui.keymap, &config.keys).1;
    problems.extend(theme::Theme::from_config(&config.theme, config.ui.accessible).1);
    if let Some(layout) = &config.ui.layout
        && let Err(error) = panes::Panes::from_spec(layout)
    {
        problems.push(format!("ui.layout: {error}"));
    }
    problems
}
