# Irsty

[![CI](https://github.com/gawbul/irsty/actions/workflows/ci.yml/badge.svg)](https://github.com/gawbul/irsty/actions/workflows/ci.yml)

A modern, keyboard-driven terminal IRC client written in Rust with [ratatui](https://ratatui.rs). It is a spiritual successor to irssi and WeeChat: fast, readable out of the box, scriptable, with first-class IRCv3 support and good support for bouncers (soju, ZNC).

> **Status:** early development, but usable day to day. The foundation and daily-driver phases are done (milestones M0 to M8), and chat history and bouncer support (M9) have landed: several networks at once, the core IRCv3 extensions, the full command set, tab completion, configurable keys, nick colours, notifications and history. See [`PLAN.md`](PLAN.md) for the roadmap.

## Goals

- irssi and WeeChat muscle memory: irssi keybindings by default, with a WeeChat preset.
- IRCv3 throughout: `server-time`, `echo-message`, `labeled-response`, `chathistory`, `soju.im/bouncer-networks` and more.
- Safe by default: TLS verified against the OS trust store, secrets kept out of config files, and every bit of remote text stripped of terminal escape sequences.
- Smooth with large scrollback and busy channels.

## What works today

- **Protocol** (`irsty-proto`): a zero-copy message parser and validating serialiser, IRCv3 message tags, `RPL_ISUPPORT`, casemapping, mIRC formatting, CTCP, and message splitting. It is checked against the ircdocs parser-tests corpus, property-tested and fuzzed.
- **Untrusted text:** escape sequences, control characters and bidi tricks from the network are neutralised before anything is displayed or logged.
- **Connections** (`irsty-core`): TLS with OS certificate verification and plain-English certificate errors, client certificates, IPv6 with Happy Eyeballs, SOCKS5 proxies (Tor-friendly: DNS goes through the proxy), `CAP LS 302`, SASL SCRAM-SHA-256, PLAIN and EXTERNAL (CertFP) with no silent downgrade, perform lines, nick fallback, keepalive with lag measurement, rate-limited output, and reconnection with backoff, server rotation and automatic rejoin.
- **State:** networks, channels, members and their modes, topics, queries and per-buffer scrollback, plus each user's account, host, real name and away status.
- **History and bouncers:** chat history (`draft/chathistory`: what you missed on join and reconnect, private conversations you missed, and older messages as you scroll back), read markers shared between devices (`draft/read-marker`), soju's networks opened as networks of their own (`soju.im/bouncer-networks`), and ZNC playback of only what you missed (`znc.in/playback`).
- **IRCv3:** `account-notify`, `account-tag`, `away-notify`, `batch` (netsplits and netjoins collapse to one line), `cap-notify`, `chghost`, `echo-message`, `extended-join`, `invite-notify`, `labeled-response`, `message-tags`, `multi-prefix`, `sasl`, `server-time`, `setname`, `standard-replies`, `userhost-in-names`, and `MONITOR`. With `echo-message`, your messages show dimmed until the server confirms them, and say why if they fail.
- **Secrets:** read from the OS keyring or a command such as `pass`. Plaintext works but triggers a warning at every start.
- **Logs:** one plain-text file per buffer, safe to `cat`, optionally rotated daily or monthly.
- **Terminal UI:** a buffer list grouped by network, scrollback with aligned nicks and word wrapping that handles CJK and emoji, a nick list, topic and status bars (activity, lag, and a loud warning for unverified TLS), and an input line with emacs-style editing, history and tab completion (nicks, recent speakers first; channels; commands and their arguments). Ctrl+G opens a fuzzy buffer switcher, and pasting several lines asks before sending. Keys follow irssi, or WeeChat with `keymap = "weechat"`, and any key can be rebound with `/bind` ([docs/KEYBINDINGS.md](docs/KEYBINDINGS.md)).
- **Commands** ([docs/COMMANDS.md](docs/COMMANDS.md)): talking (`/msg /query /me /notice /ctcp /away`), finding out (`/whois /whowas /who /names /topic /list`, with replies gathered where you typed them), running channels (`/mode /op /voice /kick /ban /invite`), buffers (`/window /close /clear`), and setting things up from inside Irsty: `/server add|del|update|list`, `/set`, `/alias`, `/ignore` and `/highlight`, all saved to your config with its comments intact. Passwords sent with `/raw` or to NickServ are hidden on screen and in logs.
- **First run:** no config file needed; add a network with `/server add` and Irsty writes one.
- **Ignores and highlights:** hide people by mask, per network or channel; highlight words and regexes besides your nick. Highlighted lines stand out, and highlights and private messages raise desktop notifications through the terminal (OSC 777, OSC 9 or the bell, tmux-aware).
- **Looks:** nick colours (with overrides), underlined links with `/url` to open them, and mIRC formatting typed with irssi's or WeeChat's keys.
- **CTCP:** answers `VERSION`, `PING`, `TIME`, `CLIENTINFO` and `SOURCE`, rate-limited.
- **Headless mode:** `--headless` prints every buffer's new lines to stdout.

Not yet: typing indicators, replies and reactions (M10), search and split windows (M11), themes (M12), scripting (M13) and DCC (M16). See [`PLAN.md`](PLAN.md).

## Quick start

Build it (rustup installs the toolchain pinned in `rust-toolchain.toml` automatically):

```sh
cargo build --release
```

Then run `./target/release/irsty` and add a network from inside it:

```
/server add libera irc.libera.chat -nick yournick,yournick_ -autojoin #irsty
```

That connects, joins `#irsty`, and saves `~/.config/irsty/config.toml` (or under `$XDG_CONFIG_HOME`) on Linux and macOS. `/help server` lists every option. You can also write the file yourself:

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

Type `/help` for commands and `/quit` to exit. Alt+1, Alt+2 and so on switch buffers. Use `--headless` to print everything to stdout instead of running the UI (Ctrl-C quits). Every option is described in [docs/CONFIG.md](docs/CONFIG.md). Diagnostics go to `irsty.log` in the platform data directory, never to the terminal; set `IRSTY_LOG=irsty_core=debug` for more detail.

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
```

The tasks wrap plain `cargo` commands and `scripts/`, so everything works without mise too. See [CONTRIBUTING.md](CONTRIBUTING.md) for details.

## Documentation

- [Architecture](docs/ARCHITECTURE.md)
- [Configuration](docs/CONFIG.md)
- [Keybindings](docs/KEYBINDINGS.md)
- [Commands](docs/COMMANDS.md)
- [Decision records](docs/adr/)
- [Dependency versions](docs/VERSIONS.md)
- [Contributing](CONTRIBUTING.md)
- [Changelog](CHANGELOG.md)

## Licence

Licensed under the [Apache License, Version 2.0](LICENSE) ([ADR 0009](docs/adr/0009-apache-2-licence.md)).

Unless you explicitly state otherwise, any contribution you intentionally submit for inclusion in this work, as defined in the Apache-2.0 licence, is licensed as above, without any additional terms or conditions.
