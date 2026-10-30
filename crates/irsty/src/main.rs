//! Irsty: a modern, keyboard-driven terminal IRC client.
#![forbid(unsafe_code)]

use clap::Parser;
use color_eyre::eyre::Result;

/// A modern, keyboard-driven terminal IRC client.
#[derive(Debug, Parser)]
#[command(version, about, long_about = None)]
struct Cli {}

fn main() -> Result<()> {
    color_eyre::install()?;
    let _cli = Cli::parse();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
