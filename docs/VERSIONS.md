# Versions

All versions were checked live on **2026-10-30** using `rustup`, `cargo info` and the crates.io API. Prereleases are ignored. Release dates are for the version listed. When a dependency is added, `cargo add` will choose the exact version, and `Cargo.lock` is committed.

## Toolchain

| Item | Version | Notes |
|---|---|---|
| rustc | 1.99.1 (58cf8f9c3 2026-10-13) | Latest stable. Pinned in `rust-toolchain.toml`. |
| cargo | 1.99.1 (5f94df478 2026-08-27) | |
| Edition | 2024 | Latest edition this toolchain supports. |
| MSRV (`rust-version`) | 1.97 | Policy: trail stable by two releases and bump deliberately. Floor from deps today is 1.89 (rusqlite, notify-rust). |
| rustup | 1.29.2 | |
| cargo-nextest, cargo-deny, cargo-audit | installed (mise) | Need to install: cargo-fuzz (nightly), cargo-insta, cargo-dist. |

## Direct dependencies (planned)

| Crate | Version | Released | Why |
|---|---|---|---|
| ratatui | 0.30.2 | 2026-06-19 | TUI framework. 0.30 split into `ratatui-core`, `-widgets` and `-crossterm`. Use the `ratatui::crossterm` re-export so versions can't drift. MSRV 1.88. |
| crossterm | 0.29.0 | 2025-04-05 | Backend (ratatui's default `crossterm_0_29`). Only listed directly to enable `event-stream` for async input. The repo is active (commits Oct 2026). |
| tokio | 1.54.1 | | Async runtime. |
| tokio-util | 0.7.20 | | `codec::Framed` and `CancellationToken`. |
| futures | 0.3.34 | | Stream combinators (`StreamExt`, `select`). |
| bytes | 1.12.1 | | Buffers for the codec. |
| rustls | 0.23.46 | 2026-10-14 | TLS. Latest stable (0.24 is still in `-dev`). Default `aws_lc_rs` provider. |
| tokio-rustls | 0.26.6 | 2026-09-28 | Async TLS streams. |
| rustls-platform-verifier | 0.7.1 | 2026-09-24 | Uses the OS trust store (Keychain on macOS), so corporate CAs work. Chosen over `webpki-roots`. |
| tokio-socks | 0.5.3 | 2026-05-29 | SOCKS5 client (Phase 2). Small and maintained. |
| serde | 1.0.229 | | Config model. |
| toml | 1.1.8 | | Config format (TOML 1.1). |
| directories | 6.0.0 | 2025-01-12 | XDG and platform paths. Stable, low churn. |
| keyring | 4.2.0 | 2026-08-29 | OS secret store. |
| clap | 4.6.7 | | CLI (derive). |
| clap_complete | 4.6.11 | | Shell completions. |
| clap_mangen | 0.3.3 | | Man page. |
| thiserror | 2.0.21 | | Errors in library crates. |
| color-eyre | 0.6.5 | 2025-05-30 | Errors in the binary. Releases are slow but the project is fine. |
| tracing | 0.1.44 | | Structured logging. |
| tracing-subscriber | 0.3.23 | | Filtering and formatting. |
| tracing-appender | 0.2.5 | 2026-04-17 | Non-blocking file writer. Never writes to the TTY. |
| unicode-width | 0.2.3 | | Column widths. Same major version as ratatui. |
| unicode-segmentation | 1.13.3 | | Grapheme clusters for cursor movement and wrapping. |
| jiff | 0.2.38 | 2026-10-06 | Time zones, parsing `server-time`, timestamps. Better API than `chrono`. |
| regex | 1.13.1 | | Highlight rules. |
| linkify | 0.11.0 | 2026-04-12 | URL detection. |
| signal-hook | 0.4.5 | 2026-10-04 | Handles SIGTSTP and SIGCONT correctly (suspend and resume). Tokio covers INT, TERM and WINCH. |
| sha2 / hmac / pbkdf2 | 0.11.1 / 0.13.0 / 0.13.0 | | Our own SCRAM-SHA-256 (Phase 2). Replaces the stale `scram` crate. |
| base64 | 0.23.1 | | SASL payloads. |
| nucleo-matcher | 0.3.1 | 2024-02-20 | Fuzzy buffer switcher (Phase 2). Used by Helix and stable. Licence is MPL-2.0 (file-level copyleft, fine as a dependency). |
| open | 5.4.5 | 2026-10-22 | Open URLs. More active than `opener`. |
| notify-rust | 4.18.2 | 2026-10-07 | Desktop notifications (Phase 2). OSC 777 and OSC 9 are the primary path. |
| arboard | 3.7.0 | 2026-10-15 | Clipboard fallback behind a feature flag. OSC 52 is the default because it works over SSH. |
| rusqlite | 0.41.1 | 2026-10-28 | SQLite search index (Phase 3), `bundled`. Synchronous calls on a dedicated thread; `sqlx` is unnecessary weight. |
| mlua | 0.12.2 | 2026-10-03 | Scripting (Phase 4). |

### Dev dependencies

| Crate | Version | Why |
|---|---|---|
| insta | 1.49.0 | Snapshot tests for the UI and the parser. |
| proptest | 1.12.0 | Round-trip properties. |
| criterion | 0.8.2 | Benchmarks for the parser and scrollback. |

### Test data

- `ircdocs/parser-tests`: CC0-1.0. Last commit was 2023-05-29, which is fine because it is a stable corpus. Files: `msg-split`, `msg-join`, `userhost-split`, `mask-match` and `validate-hostname` (`.yaml`).
- Ergo v2.19.1 (2026-08-05) for integration tests. It is not installed locally; Docker is available, so use the container image.

## Rejected or flagged

| Crate | Status | Decision |
|---|---|---|
| irc / irc-proto 1.1.0 | Last release 2025-03-24. PRs are occasionally merged, 37 open issues, MPL-2.0. | Not used. We write our own protocol crate (ADR 0001). |
| tui-textarea 0.7.0 | Stale since 2024-10. Replaced by `ratatui-textarea`. | Not used. |
| ratatui-textarea 0.9.3, tui-input 0.15.5 | Both maintained. | Not used either. We write our own input line (history, completion, formatting codes; about 300 lines). |
| tui-scrollview 0.6.8 | Maintained, but it scrolls a pre-rendered buffer. | Not used. We write our own scrollback with a wrap cache. |
| figment 0.10.19 | No release since 2024-05. | Not used. Layering is plain serde: defaults, then file, then CLI. |
| fuzzy-matcher 0.3.7 | Unmaintained (2020). | Use `nucleo-matcher` instead. |
| scram 0.6.0 | Unmaintained (2021). | Write our own on top of RustCrypto. |
| notify 8.2.1 | Fine. | Deferred. Use explicit `/reload` first and add file watching only if it is wanted. |
| sqlx 0.9.0 | MSRV 1.94, async, heavy. | Use `rusqlite` instead. |
| webpki-roots 1.0.11 | CDLA licence, bundled roots. | Use the platform verifier. Keep this only as a fallback feature for exotic platforms. |
