//! Irsty: a modern, keyboard-driven terminal IRC client.
#![forbid(unsafe_code)]

mod headless;

use std::path::PathBuf;

use clap::Parser;
use color_eyre::eyre::{Result, WrapErr, eyre};
use irsty_core::config::Paths;
use irsty_core::config_file::ConfigFile;
use tracing_subscriber::EnvFilter;

/// A modern, keyboard-driven terminal IRC client.
#[derive(Debug, Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    /// Configuration file [default: config.toml in the platform config directory]
    #[arg(short, long, value_name = "PATH")]
    config: Option<PathBuf>,

    /// Run without the terminal UI, printing every buffer's new lines to stdout
    #[arg(long)]
    headless: bool,
}

fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();
    let paths = Paths::platform();

    let config_path = cli
        .config
        .or_else(|| paths.as_ref().map(Paths::config_file))
        .ok_or_else(|| eyre!("cannot determine the configuration directory; pass --config"))?;
    // A missing file is fine: the UI starts empty and /server add creates it.
    let (file, config) = ConfigFile::open(&config_path)
        .wrap_err("see docs/CONFIG.md for the configuration format")?;

    let log_dir = paths.as_ref().map(|p| p.data_dir.clone());
    let _guard = init_tracing(log_dir)?;
    tracing::info!(config = %config_path.display(), "starting");

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    if cli.headless {
        if config.networks.is_empty() {
            return Err(eyre!(
                "no networks in {}; add one with /server add in the UI, or see docs/CONFIG.md",
                config_path.display()
            ));
        }
        return runtime.block_on(headless::run(&config, paths.as_ref()));
    }
    runtime.block_on(async {
        let client = irsty_core::Client::start(&config, paths.as_ref(), None);
        irsty_tui::run(client, config, Some(file)).await
    })?;
    Ok(())
}

/// Sends tracing output to `<data dir>/irsty.log`, never to the terminal.
/// Filter with `IRSTY_LOG` (for example `IRSTY_LOG=irsty_core=debug`).
fn init_tracing(
    dir: Option<PathBuf>,
) -> Result<Option<tracing_appender::non_blocking::WorkerGuard>> {
    let Some(dir) = dir else {
        return Ok(None);
    };
    std::fs::create_dir_all(&dir).wrap_err_with(|| format!("cannot create {}", dir.display()))?;
    let (writer, guard) =
        tracing_appender::non_blocking(tracing_appender::rolling::never(&dir, "irsty.log"));
    let filter = EnvFilter::try_from_env("IRSTY_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_ansi(false)
        .init();
    Ok(Some(guard))
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::{CommandFactory, Parser};

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_flags() {
        let cli = Cli::parse_from(["irsty", "--headless", "-c", "/tmp/x.toml"]);
        assert!(cli.headless);
        assert_eq!(
            cli.config.as_deref(),
            Some(std::path::Path::new("/tmp/x.toml"))
        );
    }
}
