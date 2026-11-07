# Irsty

[![CI](https://github.com/gawbul/irsty/actions/workflows/ci.yml/badge.svg)](https://github.com/gawbul/irsty/actions/workflows/ci.yml)

A modern, keyboard-driven terminal IRC client written in Rust with [ratatui](https://ratatui.rs). It is a spiritual successor to irssi and WeeChat: fast, readable out of the box, scriptable, with first-class IRCv3 support and good support for bouncers (soju, ZNC).

> **Status:** early development, but usable day to day. All the planned milestones, M0 to M16, are done: several networks at once, the core and modern IRCv3 extensions (history, typing, replies, reactions, multi-line messages), bouncer support, the full command set, search, split windows, themes, Luau scripting, accessibility (a plain mode for screen readers, an accessible mode and a high-contrast theme), the release tooling (a man page, shell completions, release archives built by dist, Homebrew, Scoop and a Nix flake, and an AUR package), and DCC file transfers and chats. The first release is v0.1.0. See [`PLAN.md`](PLAN.md) for the roadmap.

## Goals

- irssi and WeeChat muscle memory: irssi keybindings by default, with a WeeChat preset.
- IRCv3 throughout: `server-time`, `echo-message`, `labeled-response`, `chathistory`, `soju.im/bouncer-networks`, typing, replies, reactions, multi-line messages and more.
- Safe by default: TLS verified against the OS trust store, secrets kept out of config files, and every bit of remote text stripped of terminal escape sequences.
- Smooth with large scrollback and busy channels.

## What works today

- **Protocol** (`irsty-proto`): a zero-copy message parser and validating serialiser, IRCv3 message tags, `RPL_ISUPPORT`, casemapping, mIRC formatting, CTCP, DCC requests, and message splitting. It is checked against the ircdocs parser-tests corpus, property-tested and fuzzed.
- **Untrusted text:** escape sequences, control characters and bidi tricks from the network are neutralised before anything is displayed or logged.
- **Connections** (`irsty-core`): TLS with OS certificate verification and plain-English certificate errors, client certificates, IPv6 with Happy Eyeballs, SOCKS5 proxies (Tor-friendly: DNS goes through the proxy), `CAP LS 302`, SASL SCRAM-SHA-256, PLAIN and EXTERNAL (CertFP) with no silent downgrade, perform lines, nick fallback, keepalive with lag measurement, rate-limited output, and reconnection with backoff, server rotation and automatic rejoin.
- **State:** networks, channels, members and their modes, topics, queries and per-buffer scrollback, plus each user's account, host, real name and away status.
- **History and bouncers:** chat history (`draft/chathistory`: what you missed on join and reconnect, private conversations you missed, and older messages as you scroll back), read markers shared between devices (`draft/read-marker`), soju's networks opened as networks of their own (`soju.im/bouncer-networks`), and ZNC playback of only what you missed (`znc.in/playback`).
- **IRCv3:** `account-notify`, `account-tag`, `away-notify`, `batch` (netsplits and netjoins collapse to one line), `cap-notify`, `chghost`, `echo-message`, `extended-join`, `invite-notify`, `labeled-response`, `message-tags`, `multi-prefix`, `sasl`, `server-time`, `setname`, `standard-replies`, `userhost-in-names`, and `MONITOR`. With `echo-message`, your messages show dimmed until the server confirms them, and say why if they fail.
- **Modern messaging:** who is typing shows in the status bar (and, if you turn on `typing.send`, others see when you type); `/reply` answers someone's message and replies show what they answer; `/react` adds reactions, shown after the message; and a pasted block goes as one multi-line message (`draft/multiline`) where the server supports it.
- **Secrets:** read from the OS keyring or a command such as `pass`. Plaintext works but triggers a warning at every start.
- **Logs and search:** one plain-text file per buffer, safe to `cat`, optionally rotated daily or monthly, and a full-text index of the same messages (SQLite with FTS5, on its own thread) for `/search`: this buffer, the network or everywhere, by word or the start of one, ignoring case and accents.
- **Split windows:** `/window split` and `vsplit` divide the screen into panes, each showing its own buffer; F7 and F8 (or a click) move focus, and `/layout save` keeps the arrangement for the next start. Coming back to a buffer marks where the unread lines start, and Alt+U (Shift+Alt+U with irssi's keys) jumps there.
- **Terminal UI:** a buffer list grouped by network, scrollback with aligned nicks and word wrapping that handles CJK and emoji, a nick list, topic and status bars (activity, lag, and a loud warning for unverified TLS), and an input line with emacs-style editing, history and tab completion (nicks, recent speakers first; channels; commands and their arguments). Ctrl+G opens a fuzzy buffer switcher, and pasting several lines asks before sending. Keys follow irssi, or WeeChat with `keymap = "weechat"`, and any key can be rebound with `/bind` ([docs/KEYBINDINGS.md](docs/KEYBINDINGS.md)).
- **Commands** ([docs/COMMANDS.md](docs/COMMANDS.md)): talking (`/msg /query /me /reply /react /notice /ctcp /away`), finding out (`/whois /whowas /who /names /topic /list`, with replies gathered where you typed them), running channels (`/mode /op /voice /kick /ban /invite`), buffers and panes (`/window /close /clear /layout /search /url`), files and direct chats (`/dcc`), and setting things up from inside Irsty: `/server add|del|update|list`, `/set`, `/alias`, `/ignore`, `/highlight` and `/bind`, all saved to your config with its comments intact, `/reload` after you edit it by hand, and `/script` to load and manage scripts. Passwords sent with `/raw` or to NickServ are hidden on screen and in logs.
- **First run:** no config file needed; add a network with `/server add` and Irsty writes one.
- **Ignores and highlights:** hide people by mask, per network or channel; highlight words and regexes besides your nick. Highlighted lines stand out, and highlights and private messages raise desktop notifications through the terminal (OSC 777, OSC 9 or the bell, tmux-aware).
- **Looks:** themes (`dark`, `light` and `high-contrast` presets, and every element restylable in `[theme]`), colours fitted to what the terminal shows (truecolor, 256, 16, or none, honouring `NO_COLOR`), nick colours (with overrides), underlined links with `/url` to open them, and mIRC formatting typed with irssi's or WeeChat's keys.
- **Scripting:** Luau scripts react to events, add commands, print and set timers, each in a sandbox of its own (no files, network or programs, and time and memory limits); acting for you on the network needs `allow = ["send"]`. See [docs/SCRIPTING.md](docs/SCRIPTING.md) and [examples/scripts](examples/scripts).
- **Accessibility:** `irsty --plain` is a line-by-line interface for screen readers and braille displays: nothing is drawn, new lines are printed as they arrive (mentions and private messages from other buffers too), and you type commands on an ordinary line. `[ui] accessible` makes the full-screen UI one column of ASCII text with words beside anything shown only by colour, and the `high-contrast` theme needs no colours at all. See [docs/ACCESSIBILITY.md](docs/ACCESSIBILITY.md).
- **Config tools:** `/reload` reads the file again after you edit it (networks included), `irsty --check-config` checks a file without starting, and `irsty --print-default-config` prints one with every setting at its default.
- **CTCP:** answers `VERSION`, `PING`, `TIME`, `CLIENTINFO` and `SOURCE`, rate-limited.
- **Headless mode:** `--headless` prints every buffer's new lines to stdout.
- **DCC:** send and receive files and chat directly with someone, as in irssi (and with irssi): `/dcc get`, `send`, `chat`, `resume` and `close`, passive DCC for users behind NAT, and resuming a file that stopped part-way. Nothing is accepted without you; received files get a safe name and never replace an existing file. See [docs/COMMANDS.md](docs/COMMANDS.md#files-and-direct-chats-dcc).
- **Man page and completions:** `irsty --man` and `irsty --completions <bash|zsh|fish|powershell|elvish>`, with copies in `assets/` for packagers ([docs/PACKAGING.md](docs/PACKAGING.md)).

Not yet: the AUR package (ready in `packaging/aur`, waiting for AUR registration to reopen) and nixpkgs (`packaging/nix/package.nix` is ready to submit). See [`PLAN.md`](PLAN.md).

## Install

On macOS and Linux:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/gawbul/irsty/releases/latest/download/irsty-installer.sh | sh
```

With Homebrew, on macOS and Linux:

```sh
brew install gawbul/tap/irsty
```

On Windows, in PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/gawbul/irsty/releases/latest/download/irsty-installer.ps1 | iex"
```

With Scoop, on Windows:

```powershell
scoop bucket add gawbul https://github.com/gawbul/scoop-bucket
scoop install gawbul/irsty
```

With Nix, on Linux and macOS (it builds from source, so it takes a few minutes):

```sh
nix profile install github:gawbul/irsty
```

With Rust, `cargo install irsty` builds it. The [releases](https://github.com/gawbul/irsty/releases) have archives for each platform, with the man page and shell completions inside. The [project site](https://gawbul.github.io/irsty/) has all the documentation.

### Building from source

rustup installs the toolchain pinned in `rust-toolchain.toml` automatically; you also need a C and C++ compiler, for the bundled SQLite and Luau (Xcode's command line tools on macOS, `build-essential` or similar on Linux):

```sh
cargo build --release
```

The binary is `./target/release/irsty`, and the man page and shell completions are in `assets/` ([docs/PACKAGING.md](docs/PACKAGING.md#man-page-and-completions) says where each goes).

## Quick start

Run `irsty` and add a network from inside it:

```
/server add libera irc.libera.chat -nick yournick,yournick_ -autojoin #irsty
```

That connects, joins `#irsty`, and saves `~/.config/irsty/config.toml` (or under `$XDG_CONFIG_HOME`) on Linux and macOS. `/help server` lists every option. You can also write the file yourself (`irsty --print-default-config` prints every setting to start from, and `irsty --check-config` checks your file):

```toml
[[network]]
name = "libera"
nicks = ["yournick", "yournick_"]
autojoin = ["#irsty"]

[[network.servers]]
host = "irc.libera.chat"

[network.sasl]
mechanism = "scram-sha-256"
account = "yournick"
password = { keyring = "libera" }
```

Store the password in your OS keyring under service `irsty` (with `/server add`, pass `-sasl scram-sha-256 -account yournick -keyring libera`):

```sh
# macOS
security add-generic-password -s irsty -a libera -w
# Linux (GNOME Keyring or KWallet via the Secret Service)
secret-tool store --label "irsty libera" service irsty username libera
```

Type `/help` for commands and `/quit` to exit. Alt+1, Alt+2 and so on switch buffers. Use `--plain` for a line-by-line interface that works with screen readers ([docs/ACCESSIBILITY.md](docs/ACCESSIBILITY.md)), or `--headless` to print everything to stdout without any input (Ctrl-C quits). Every option is described in [docs/CONFIG.md](docs/CONFIG.md). Diagnostics go to `irsty.log` in the platform data directory, never to the terminal; set `IRSTY_LOG=irsty_core=debug` for more detail.

## Development

Day-to-day work goes through [mise](https://mise.jdx.dev) tasks, which also pin the tools:

```sh
mise run setup      # install the pinned tools, the MSRV toolchain and nightly
mise tasks          # list every task
mise run test       # nextest; extra arguments are filters (mise run test scrollback)
mise run lint       # clippy with warnings denied
mise run watch      # bacon: recheck on every save
mise run gate       # the full gate: fmt, clippy, tests, coverage, deny, audit, MSRV, no sync duplicates
mise run ergo       # a local Ergo server on 16667/16697 (in another terminal)
mise run test:ergo  # integration tests against it
mise run run:ergo   # Irsty itself, connected to it
mise run soju       # a local soju bouncer in front of Ergo (build it once with mise run soju:install)
mise run test:soju  # integration tests against it
mise run fuzz       # all fuzz targets for 10 minutes (needs nightly)
mise run assets     # regenerate the man page and completions after changing a flag
```

The tasks wrap plain `cargo` commands and `scripts/`, so everything works without mise too. See [CONTRIBUTING.md](CONTRIBUTING.md) for details.

## Documentation

Everything below is also in the [docs on the project site](https://gawbul.github.io/irsty/docs/).

- [Architecture](docs/ARCHITECTURE.md)
- [Configuration](docs/CONFIG.md)
- [Keybindings](docs/KEYBINDINGS.md)
- [Commands](docs/COMMANDS.md)
- [Scripting](docs/SCRIPTING.md)
- [Accessibility](docs/ACCESSIBILITY.md)
- [Packaging and releases](docs/PACKAGING.md)
- [Decision records](docs/adr/)
- [Dependency versions](docs/VERSIONS.md)
- [Contributing](CONTRIBUTING.md)
- [Changelog](CHANGELOG.md)

## Licence

Licensed under the [Apache License, Version 2.0](LICENSE) ([ADR 0009](docs/adr/0009-apache-2-licence.md)).

Unless you explicitly state otherwise, any contribution you intentionally submit for inclusion in this work, as defined in the Apache-2.0 licence, is licensed as above, without any additional terms or conditions.
