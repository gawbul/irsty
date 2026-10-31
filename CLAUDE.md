# Irsty

A modern terminal IRC client in Rust (ratatui). Spiritual successor to irssi and WeeChat. See `PLAN.md` for milestones, `docs/ARCHITECTURE.md` for the design, and `docs/adr/` for decisions.

## Standing rules

- British English in prose, docs and comments. Never use em dashes anywhere (code, docs, commit messages); use commas, colons, semicolons or parentheses.
- Communicate succinctly and with opinions. Push back on bad ideas.
- Use the latest stable Rust and the latest crate releases. Verify versions and APIs live (`cargo info`, docs.rs, Context7); never work from memory. Record changes in `docs/VERSIONS.md`.
- `#![forbid(unsafe_code)]` in every crate. Never panic on external input. Strip terminal escapes from untrusted text before it reaches the UI (ADR 0005).
- Keep core logic UI-free. The render loop never blocks. Every feature ships with tests and docs.
- Run `./scripts/gate.sh` (no tracked iCloud duplicates, fmt, clippy with warnings denied, nextest, doctests, `irsty-proto` coverage of at least 90%, deny, audit, MSRV) before declaring a milestone done. Report what passed; don't claim anything works without running it.
- Prefer boring, maintained crates. Don't add a dependency when 20 lines will do. Flag unmaintained crates.
- Use Conventional Commits and small, focused commits, with one milestone per PR. Start every piece of work on a fresh branch from an up-to-date `main`. Once a PR is merged, switch to an updated `main` and delete the merged local branch with `git branch -d` (never `-D`; if it refuses, ask).
- Irsty is licensed under Apache-2.0 only (ADR 0009). New files need no licence header.
- After making any change, and always before opening a PR, update `README.md`, this file and every other document the change affects (`docs/` such as ARCHITECTURE, CONFIG, COMMANDS, KEYBINDINGS and VERSIONS, plus `CONTRIBUTING.md`, `PLAN.md`, `CHANGELOG.md`, and doc comments), checked against the code rather than against older docs. Commit those updates on the same branch.

## Layout

- `crates/irsty-proto`: sans-IO IRC/IRCv3 parsing, serialisation, CAP and SASL. No async, no I/O.
- `crates/irsty-core`: `state` (`State::apply` turns network events into state changes and effects), `client` (owns the state and the connection tasks), `net/` (`connection`, `transport` with TLS and certificate explanations, `dial` for Happy Eyeballs, `socks`, `codec`, `ratelimit`, `backoff`), `batch` (IRCv3 batches), `replies` (formatting numeric replies), `log`, `redact`, the config model and `config_file` (editing `config.toml` in place). No UI dependencies.
- `crates/irsty-tui`: the terminal UI. `run.rs` is the event loop, `app.rs` maps keys and commands to actions, `command.rs` parses slash commands (its `COMMANDS` table drives parsing, `/help` and a test against `docs/COMMANDS.md`, so add every new command there), `view.rs` draws everything from `&State` and `&mut Ui` (pure, snapshot-tested), `scrollback.rs` is the bottom-up scrollback widget, `terminal.rs` sets up and restores the terminal, `keymap.rs` maps keys to actions (presets and `/bind`), `complete.rs` does Tab completion and fuzzy matching, `urls.rs` finds and opens links, and `notify.rs` builds terminal notifications. UI integration tests in `crates/irsty-tui/tests/` drive real key presses against a mock server (`tests/support`).
- `crates/irsty`: the binary (CLI, wiring, tracing setup).

The dependency direction is strictly `irsty → irsty-tui → irsty-core → irsty-proto`.

## Tooling

- The Rust toolchain is pinned in `rust-toolchain.toml`. MSRV is `rust-version` in `Cargo.toml` and trails stable by two releases (ADR 0006).
- Developer tools (nextest, deny, audit, llvm-cov, fuzz, insta, bacon) and tasks live in `mise.toml`; `mise tasks` lists them. Tasks are thin wrappers over `cargo` and `scripts/`, so keep the scripts the source of truth and add a task when you add a script.
- Lints are set in `[workspace.lints]`, with pedantic enabled. When `doc_markdown` flags a protocol term (IPv6, SASLprep), add it to `doc-valid-idents` in `clippy.toml` rather than wrapping it in backticks. Exceptions are documented in `CONTRIBUTING.md`. Tests may use `unwrap`, `expect`, `panic` and indexing (`clippy.toml`).
- tracing writes to a file, never to the terminal.
- UI snapshots live in `crates/irsty-tui/tests/snapshots/`. Review changes with `cargo insta review`, or generate with `INSTA_UPDATE=always` and read every changed `.snap` before committing. Snapshot state uses `State::set_clock` so timestamps are fixed.
- Never call `Terminal::clear` in the UI: it queries the cursor position, which fails after two seconds on terminals that don't answer. Use `fresh_terminal()` in `run.rs`.
- Mock-server integration tests share `crates/irsty-core/tests/support/mod.rs`. A test that waits on the mock for something the client sends only after processing a server message must keep driving the client (`drive`), or it deadlocks.
- Ergo integration tests run when `IRSTY_ERGO_PORT` is set (see `CONTRIBUTING.md`). Local Docker can't pull from ghcr.io on this machine; use `mise run ergo` (`scripts/ergo.sh`), which runs the checksum-verified release binary on ports 16667/16697. Ergo's NickServ answers by notice, which lands in a `NickServ` query buffer, not the server buffer.
- The repository sits in an iCloud-synced folder. Files named like `foo 2.rs` are sync duplicates, not code; compare them with the originals and move them out of the tree (cargo auto-discovers `tests/` and `benches/`). `.gitignore` ignores such names, and the gate and CI fail if one is tracked (two were committed by accident in M5).
