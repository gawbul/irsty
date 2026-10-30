# Irsty

[![CI](https://github.com/gawbul/irsty/actions/workflows/ci.yml/badge.svg)](https://github.com/gawbul/irsty/actions/workflows/ci.yml)

A modern, keyboard-driven terminal IRC client written in Rust with [ratatui](https://ratatui.rs). It is a spiritual successor to irssi and WeeChat: fast, readable out of the box, scriptable, with first-class IRCv3 support and good support for bouncers (soju, ZNC).

> **Status:** early development. The protocol and connection layers are done (milestones M1 and M2), and `irsty --headless` connects, registers, joins channels and logs. The terminal UI arrives in M3. See [`PLAN.md`](PLAN.md) for the roadmap.

## Goals

- irssi and WeeChat muscle memory: irssi keybindings by default, with a WeeChat preset.
- IRCv3 throughout: `server-time`, `echo-message`, `labeled-response`, `chathistory`, `soju.im/bouncer-networks` and more.
- Safe by default: TLS verified against the OS trust store, secrets kept out of config files, and every bit of remote text stripped of terminal escape sequences.
- Smooth with large scrollback and busy channels.

## What works today

- **Protocol** (`irsty-proto`): a zero-copy message parser and validating serialiser, IRCv3 message tags, `RPL_ISUPPORT`, casemapping, mIRC formatting, CTCP, and message splitting. It is checked against the ircdocs parser-tests corpus, property-tested and fuzzed.
- **Untrusted text:** escape sequences, control characters and bidi tricks from the network are neutralised before anything is displayed or logged.
- **Connections** (`irsty-core`): TLS with OS certificate verification, client certificates, `CAP LS 302`, SASL PLAIN and EXTERNAL, nick fallback, keepalive with lag measurement, rate-limited output, and reconnection with backoff, server rotation and automatic rejoin.
- **State:** networks, channels, members and their modes, topics, queries and per-buffer scrollback.
- **Secrets:** read from the OS keyring or a command such as `pass`. Plaintext works but triggers a warning at every start.
- **Logs:** one plain-text file per buffer, safe to `cat`.
- **Headless mode:** prints every buffer's new lines to stdout.

Not yet: the terminal UI, user commands, and most IRCv3 extensions beyond `sasl`, `server-time`, `message-tags`, `multi-prefix`, `cap-notify` and `userhost-in-names`. These arrive in M3 to M5.

## Quick start

Build it (rustup installs the toolchain pinned in `rust-toolchain.toml` automatically):

```sh
cargo build --release
```

Create `~/.config/irsty/config.toml` on Linux, or `~/Library/Application Support/irsty/config.toml` on macOS:

```toml
[[network]]
name = "libera"
nicks = ["yournick", "yournick_"]
autojoin = ["#irsty"]

[[network.servers]]
host = "irc.libera.chat"

[network.sasl]
mechanism = "plain"
account = "yournick"
password = { keyring = "libera" }
```

Store the password in your OS keyring under service `irsty`:

```sh
# macOS
security add-generic-password -s irsty -a libera -w
# Linux (GNOME Keyring or KWallet via the Secret Service)
secret-tool store --label "irsty libera" service irsty username libera
```

Then run it:

```sh
./target/release/irsty --headless
```

Press Ctrl-C to quit cleanly. Every option is described in [docs/CONFIG.md](docs/CONFIG.md). Diagnostics go to `irsty.log` in the platform data directory, never to the terminal; set `IRSTY_LOG=irsty_core=debug` for more detail.

## Development

```sh
./scripts/gate.sh   # fmt, clippy (pedantic, warnings denied), tests, coverage, deny, audit, MSRV
./scripts/fuzz.sh   # all fuzz targets for 10 minutes (needs nightly)
```

Gate tools are pinned in `mise.toml`. The integration tests against a real Ergo server are described in [CONTRIBUTING.md](CONTRIBUTING.md).

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

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT licence](LICENSE-MIT), at your option.

Unless you explicitly state otherwise, any contribution you intentionally submit for inclusion in this work, as defined in the Apache-2.0 licence, shall be dual licensed as above, without any additional terms or conditions.
