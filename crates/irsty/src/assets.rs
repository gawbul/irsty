//! The man page and shell completions, generated from the command-line
//! definition so they can't disagree with it. `irsty --man` and
//! `irsty --completions <shell>` print them; copies live in `assets/` for
//! packagers, and a test keeps those up to date.

use clap::{Command, CommandFactory};
use clap_complete::Shell;
use clap_mangen::Man;
use clap_mangen::roff::{Roff, bold, italic, roman};

use crate::Cli;

/// Every shell we ship completions for, and the file name each conventionally
/// uses (the names in `assets/completions`).
#[cfg(test)]
const SHELLS: &[(Shell, &str)] = &[
    (Shell::Bash, "irsty.bash"),
    (Shell::Zsh, "_irsty"),
    (Shell::Fish, "irsty.fish"),
    (Shell::PowerShell, "_irsty.ps1"),
    (Shell::Elvish, "irsty.elv"),
];

/// The man page's date. A fixed date, not today's, so the page in `assets/`
/// only changes when something in it does; set it when releasing
/// (`docs/PACKAGING.md`, "Releasing").
const MAN_DATE: &str = "2026-11-07";

fn command() -> Command {
    let mut cmd = Cli::command();
    cmd.build();
    cmd
}

/// Completions for `shell`.
pub(crate) fn completions(shell: Shell) -> Vec<u8> {
    let mut out = Vec::new();
    clap_complete::generate(shell, &mut command(), "irsty", &mut out);
    out
}

/// The man page, in roff: the options from clap, and the files, environment
/// and further reading a man page should have.
pub(crate) fn man() -> std::io::Result<Vec<u8>> {
    let man = Man::new(command()).manual("Irsty Manual").date(MAN_DATE);
    let mut out = Vec::new();
    man.render_title(&mut out)?;
    man.render_name_section(&mut out)?;
    man.render_synopsis_section(&mut out)?;
    man.render_description_section(&mut out)?;
    man.render_options_section(&mut out)?;
    extra_sections().to_writer(&mut out)?;
    man.render_version_section(&mut out)?;
    man.render_authors_section(&mut out)?;
    Ok(out)
}

/// A paragraph: a tag, then indented text, as man pages list things.
fn item(roff: &mut Roff, tag: &str, text: &str) {
    roff.control("TP", []);
    roff.text([bold(tag)]);
    roff.text([roman(text)]);
}

fn extra_sections() -> Roff {
    let mut roff = Roff::new();
    roff.control("SH", ["GETTING STARTED"]);
    roff.text([roman(
        "Irsty starts with a buffer list, the scrollback and an input line. Type messages, \
         or commands starting with /: /server add to add a network, /join, /msg, /window to \
         switch buffers, /help for everything. /quit leaves. Without a config file Irsty \
         starts empty and writes one when you add a network.",
    )]);

    roff.control("SH", ["FILES"]);
    item(
        &mut roff,
        "$XDG_CONFIG_HOME/irsty/config.toml",
        "The configuration (usually ~/.config/irsty/config.toml on Linux and macOS). \
         irsty --print-default-config prints every setting at its default.",
    );
    item(
        &mut roff,
        "~/.local/share/irsty/",
        "Data on Linux (~/Library/Application Support/irsty/ on macOS): logs/ with a \
         plain-text log per buffer, search.sqlite for /search, and irsty.log with \
         Irsty's own diagnostics.",
    );

    roff.control("SH", ["ENVIRONMENT"]);
    item(
        &mut roff,
        "XDG_CONFIG_HOME",
        "Where the config directory is; ignored unless absolute.",
    );
    item(
        &mut roff,
        "IRSTY_LOG",
        "Filter for irsty.log, such as irsty_core=debug. Passwords are never logged.",
    );
    item(
        &mut roff,
        "NO_COLOR",
        "If set and not empty, no colours (unless [theme] colours says otherwise).",
    );
    item(
        &mut roff,
        "COLORTERM, TERM",
        "How many colours the terminal shows, with [theme] colours = \"auto\".",
    );
    item(
        &mut roff,
        "TMUX",
        "Inside tmux, notifications are wrapped so tmux passes them on.",
    );

    roff.control("SH", ["EXIT STATUS"]);
    roff.text([roman(
        "0 on success. 1 if --check-config finds a problem. Other failures (an unreadable \
         config, a terminal that can't be set up) exit with a non-zero status and a message.",
    )]);

    roff.control("SH", ["SEE ALSO"]);
    roff.text([
        italic("irssi"),
        roman("(1), "),
        italic("weechat"),
        roman(
            "(1). The full documentation, configuration, commands, key bindings, \
               scripting and accessibility, is at https://github.com/gawbul/irsty/tree/main/docs.",
        ),
    ]);
    roff
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;

    fn assets() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets")
    }

    /// The copies in `assets/` match what `irsty` generates. Set
    /// `IRSTY_UPDATE_ASSETS=1` (or run `mise run assets`) to rewrite them.
    #[test]
    fn assets_are_up_to_date() {
        let mut files = vec![(assets().join("man/irsty.1"), man().unwrap())];
        for (shell, name) in SHELLS {
            files.push((assets().join("completions").join(name), completions(*shell)));
        }
        let update = std::env::var_os("IRSTY_UPDATE_ASSETS").is_some();
        for (path, want) in files {
            if update {
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, &want).unwrap();
                continue;
            }
            let have = std::fs::read(&path).unwrap_or_default();
            assert!(
                have == want,
                "{} is out of date; run mise run assets",
                path.display()
            );
        }
    }

    #[test]
    fn the_man_page_has_its_sections() {
        let page = String::from_utf8(man().unwrap()).unwrap();
        for section in [
            "NAME",
            "SYNOPSIS",
            "OPTIONS",
            "FILES",
            "ENVIRONMENT",
            "SEE ALSO",
        ] {
            let plain = format!(".SH {section}\n");
            let quoted = format!(".SH \"{section}\"\n");
            assert!(page.contains(&plain) || page.contains(&quoted), "{section}");
        }
        assert!(page.contains("\\-\\-check\\-config"), "options from clap");
    }

    #[test]
    fn completions_know_the_flags() {
        for (shell, _) in SHELLS {
            let text = String::from_utf8(completions(*shell)).unwrap();
            assert!(text.contains("print-default-config"), "{shell}");
        }
    }
}
