//! Irsty: a modern, keyboard-driven terminal IRC client.
#![forbid(unsafe_code)]

mod assets;
mod headless;

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::PathBuf;

use clap::Parser;
use color_eyre::eyre::{Result, WrapErr, eyre};
use irsty_core::config::Paths;
use irsty_core::config_file::ConfigFile;
use tracing_subscriber::EnvFilter;

/// A modern, keyboard-driven terminal IRC client.
#[derive(Debug, Parser)]
#[command(
    version,
    about,
    long_about = "A modern, keyboard-driven terminal IRC client, a spiritual successor to irssi \
                  and WeeChat: several networks at once, IRCv3 throughout (history, typing, \
                  replies, reactions), bouncer support (soju, ZNC), search, split windows, \
                  themes, Luau scripting, and an accessible plain mode for screen readers. \
                  TLS is verified against the system's certificates, secrets stay in the \
                  keyring, and text from the network can never send escape sequences to your \
                  terminal."
)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "command-line flags; clap keeps the modes apart"
)]
struct Cli {
    /// Configuration file [default: config.toml in the platform config directory]
    #[arg(short, long, value_name = "PATH")]
    config: Option<PathBuf>,

    /// Run without the terminal UI, printing every buffer's new lines to stdout
    #[arg(long)]
    headless: bool,

    /// A line-by-line interface for screen readers and braille displays:
    /// nothing is drawn, lines are printed as they arrive, and commands are
    /// read a line at a time
    #[arg(long, conflicts_with = "headless")]
    plain: bool,

    /// Check the configuration file, report any problems, and exit (status 1
    /// if there are any)
    #[arg(long, conflicts_with_all = ["headless", "plain", "print_default_config"])]
    check_config: bool,

    /// Print a configuration file with every setting at its default, and exit
    #[arg(long, conflicts_with_all = ["headless", "plain"])]
    print_default_config: bool,

    /// Print the man page (roff), and exit
    #[arg(long, conflicts_with_all = ["headless", "plain", "check_config", "print_default_config"])]
    man: bool,

    /// Print completions for SHELL, and exit
    #[arg(long, value_name = "SHELL", conflicts_with_all = ["headless", "plain", "check_config", "print_default_config", "man"])]
    completions: Option<clap_complete::Shell>,
}

fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();
    if cli.man {
        std::io::stdout().lock().write_all(&assets::man()?)?;
        return Ok(());
    }
    if let Some(shell) = cli.completions {
        std::io::stdout()
            .lock()
            .write_all(&assets::completions(shell))?;
        return Ok(());
    }
    if cli.print_default_config {
        std::io::stdout()
            .lock()
            .write_all(irsty_core::config::DEFAULT_CONFIG.as_bytes())?;
        return Ok(());
    }
    let paths = Paths::platform();

    let config_path = cli
        .config
        .or_else(|| paths.as_ref().map(Paths::config_file))
        .ok_or_else(|| eyre!("cannot determine the configuration directory; pass --config"))?;
    // A missing file is fine: the UI starts empty and /server add creates it.
    if cli.check_config {
        let (report, ok) = check(&config_path);
        std::io::stdout().lock().write_all(report.as_bytes())?;
        std::process::exit(i32::from(!ok));
    }
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
        if cli.plain {
            irsty_tui::plain::run(client, config, Some(file)).await
        } else {
            irsty_tui::run(client, config, Some(file)).await
        }
    })?;
    Ok(())
}

/// `--check-config`: a report on the file, and whether it is fine.
/// Plaintext secrets are warnings; anything Irsty would refuse or skip is
/// an error.
fn check(path: &std::path::Path) -> (String, bool) {
    if !path.exists() {
        return (
            format!(
                "{} doesn't exist; Irsty would start with no networks (see --print-default-config)\n",
                path.display()
            ),
            true,
        );
    }
    let config = match ConfigFile::open(path) {
        Ok((_, config)) => config,
        Err(error) => return (format!("error: {error}\n"), false),
    };
    let mut report = String::new();
    let problems = irsty_tui::check_config(&config);
    for problem in &problems {
        let _ = writeln!(report, "error: {problem}");
    }
    for warning in config.plaintext_secrets() {
        let _ = writeln!(
            report,
            "warning: plaintext secret ({warning}); prefer keyring or command"
        );
    }
    if problems.is_empty() {
        let networks = config.networks.len();
        let _ = writeln!(
            report,
            "{} is fine: {networks} network{}",
            path.display(),
            if networks == 1 { "" } else { "s" }
        );
    }
    (report, problems.is_empty())
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
    fn checks_config_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let (report, ok) = super::check(&path);
        assert!(ok && report.contains("doesn't exist"), "{report}");

        std::fs::write(&path, irsty_core::config::DEFAULT_CONFIG).unwrap();
        let (report, ok) = super::check(&path);
        assert!(ok && report.contains("is fine: 0 networks"), "{report}");

        std::fs::write(
            &path,
            "[keys]\n\"ctrl-q\" = \"fly\"\n[theme]\nbar = \"puce\"\n[ui]\nlayout = \"v(\"\n",
        )
        .unwrap();
        let (report, ok) = super::check(&path);
        assert!(!ok);
        assert_eq!(report.matches("error:").count(), 3, "{report}");

        std::fs::write(&path, "scrollback = \"lots\"\n").unwrap();
        let (report, ok) = super::check(&path);
        assert!(!ok && report.starts_with("error:"), "{report}");

        std::fs::write(
            &path,
            "[[network]]\nname = \"n\"\nnicks = [\"me\"]\n[[network.servers]]\nhost = \"h\"\npassword = { plain = \"x\" }\n",
        )
        .unwrap();
        let (report, ok) = super::check(&path);
        assert!(
            ok && report.contains("warning: plaintext secret"),
            "{report}"
        );
    }

    #[test]
    fn parses_flags() {
        let cli = Cli::parse_from(["irsty", "--headless", "-c", "/tmp/x.toml"]);
        assert!(cli.headless);
        assert!(Cli::parse_from(["irsty", "--plain"]).plain);
        assert!(Cli::try_parse_from(["irsty", "--plain", "--headless"]).is_err());
        assert_eq!(
            cli.config.as_deref(),
            Some(std::path::Path::new("/tmp/x.toml"))
        );
    }
}

#[cfg(test)]
mod licence {
    /// Each crate carries a copy of the licence for crates.io; they must all
    /// be the repository's.
    #[test]
    fn every_crate_has_the_licence() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let licence = std::fs::read(root.join("LICENSE")).unwrap();
        for krate in ["irsty-proto", "irsty-core", "irsty-tui", "irsty"] {
            let copy = std::fs::read(root.join("crates").join(krate).join("LICENSE")).unwrap();
            assert!(
                copy == licence,
                "crates/{krate}/LICENSE differs from LICENSE"
            );
        }
    }
}
