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
- Use Conventional Commits and small, focused commits, with one milestone per PR.

## Layout

- `crates/irsty-proto`: sans-IO IRC/IRCv3 parsing, serialisation, CAP and SASL. No async, no I/O.
- `crates/irsty-core`: connections, state, actions and effects, logging, config model. No UI dependencies.
- `crates/irsty-tui`: ratatui views, input, keymaps, themes.
- `crates/irsty`: the binary (CLI, wiring, tracing setup).

The dependency direction is strictly `irsty → irsty-tui → irsty-core → irsty-proto`.

## Tooling

- The Rust toolchain is pinned in `rust-toolchain.toml`. MSRV is `rust-version` in `Cargo.toml` and trails stable by two releases (ADR 0006).
- Gate tools (nextest, deny, audit) are pinned in `mise.toml`.
- Lints are set in `[workspace.lints]`, with pedantic enabled. Exceptions are documented in `CONTRIBUTING.md`. Tests may use `unwrap`, `expect`, `panic` and indexing (`clippy.toml`).
- tracing writes to a file, never to the terminal.
