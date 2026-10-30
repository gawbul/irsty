# Irsty

[![CI](https://github.com/gawbul/irsty/actions/workflows/ci.yml/badge.svg)](https://github.com/gawbul/irsty/actions/workflows/ci.yml)

A modern, keyboard-driven terminal IRC client written in Rust with [ratatui](https://ratatui.rs). It is a spiritual successor to irssi and WeeChat: fast, readable out of the box, scriptable, with first-class IRCv3 support and good support for bouncers (soju, ZNC).

> **Status:** early development. The protocol and connection layers work (M1 and M2) and `irsty --headless` connects, joins and logs; the terminal UI arrives in M3. See [`PLAN.md`](PLAN.md) for the roadmap.

## Goals

- irssi and WeeChat muscle memory: irssi keybindings by default, with a WeeChat preset.
- IRCv3 throughout: `server-time`, `echo-message`, `labeled-response`, `chathistory`, `soju.im/bouncer-networks` and more.
- Safe by default: TLS verified against the OS trust store, secrets kept out of config files, and every bit of remote text stripped of terminal escape sequences.
- Smooth with large scrollback and busy channels.

## Building

Requires the Rust toolchain pinned in `rust-toolchain.toml` (rustup installs it automatically).

```sh
cargo run -- --help
```

## Documentation

- [Architecture](docs/ARCHITECTURE.md)
- [Configuration](docs/CONFIG.md)
- [Keybindings](docs/KEYBINDINGS.md)
- [Commands](docs/COMMANDS.md)
- [Decision records](docs/adr/)
- [Dependency versions](docs/VERSIONS.md)

## Licence

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT licence](LICENSE-MIT), at your option.

Unless you explicitly state otherwise, any contribution you intentionally submit for inclusion in this work, as defined in the Apache-2.0 licence, shall be dual licensed as above, without any additional terms or conditions.
