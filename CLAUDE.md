# Irsty

A modern terminal IRC client in Rust (ratatui). Spiritual successor to irssi and WeeChat. See `PLAN.md` for milestones, `docs/ARCHITECTURE.md` for the design, and `docs/adr/` for decisions.

## Standing rules

- British English in prose, docs and comments. Never use em dashes anywhere (code, docs, commit messages); use commas, colons, semicolons or parentheses.
- Communicate succinctly and with opinions. Push back on bad ideas.
- Use the latest stable Rust and the latest crate releases. Verify versions and APIs live (`cargo info`, docs.rs, Context7); never work from memory. Record changes in `docs/VERSIONS.md`.
- `#![forbid(unsafe_code)]` in every crate. Never panic on external input. Strip terminal escapes from untrusted text before it reaches the UI (ADR 0005).
- Keep core logic UI-free. The render loop never blocks. Every feature ships with tests and docs.
- Run `./scripts/gate.sh` (fmt, clippy with warnings denied, nextest, doctests, deny, audit, MSRV) before declaring a milestone done. Report what passed; don't claim anything works without running it.
- Prefer boring, maintained crates. Don't add a dependency when 20 lines will do. Flag unmaintained crates.
- Use Conventional Commits and small, focused commits, with one milestone per PR. Start every piece of work on a fresh branch from an up-to-date `main`.
- Irsty is licensed under Apache-2.0 only (ADR 0009). New files need no licence header.
- Before raising a PR, update `README.md` and this file wherever the branch's changes make them stale, and commit those updates on the same branch.

## Layout

- `crates/irsty-proto`: sans-IO IRC/IRCv3 parsing, serialisation, CAP and SASL. No async, no I/O.
- `crates/irsty-core`: connections, state, actions and effects, logging, config model. No UI dependencies.
- `crates/irsty-tui`: the terminal UI. `run.rs` is the event loop, `app.rs` maps keys and commands to actions, `command.rs` parses slash commands, `view.rs` draws everything from `&State` and `&mut Ui` (pure, snapshot-tested), `scrollback.rs` is the bottom-up scrollback widget, `terminal.rs` sets up and restores the terminal.
- `crates/irsty`: the binary (CLI, wiring, tracing setup).

The dependency direction is strictly `irsty → irsty-tui → irsty-core → irsty-proto`.

## Tooling

- The Rust toolchain is pinned in `rust-toolchain.toml`. MSRV is `rust-version` in `Cargo.toml` and trails stable by two releases (ADR 0006).
- Gate tools (nextest, deny, audit) are pinned in `mise.toml`.
- Lints are set in `[workspace.lints]`, with pedantic enabled. Exceptions are documented in `CONTRIBUTING.md`. Tests may use `unwrap`, `expect`, `panic` and indexing (`clippy.toml`).
- tracing writes to a file, never to the terminal.
- UI snapshots live in `crates/irsty-tui/tests/snapshots/`. Review changes with `cargo insta review`, or generate with `INSTA_UPDATE=always` and read every changed `.snap` before committing. Snapshot state uses `State::set_clock` so timestamps are fixed.
- Never call `Terminal::clear` in the UI: it queries the cursor position, which fails after two seconds on terminals that don't answer. Use `fresh_terminal()` in `run.rs`.
- Ergo integration tests run when `IRSTY_ERGO_PORT` is set (see `CONTRIBUTING.md`). Local Docker can't pull from ghcr.io on this machine; use the checksum-verified release binary on ports 16667/16697.
- The repository sits in an iCloud-synced folder. Files named like `foo 2.rs` are sync duplicates, not code; compare them with the originals and move them out of the tree (cargo auto-discovers `tests/` and `benches/`).
