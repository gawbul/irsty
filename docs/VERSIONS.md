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
| Developer tools | installed (mise) | See the table below. Still to add: cargo-dist (Phase 4). |

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
| tokio-socks | 0.5.3 | 2026-05-29 | Not used: M4 needed only SOCKS5 CONNECT, which is about 100 lines in `net/socks.rs` with exact error messages and guaranteed remote DNS. |
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
| sha2 / hmac / pbkdf2 | 0.11.1 / 0.13.0 / 0.13.0 | 2026-10-12 / 2026-03-29 / 2026-04-21 | Our own SCRAM-SHA-256 (added in M4). Replaces the stale `scram` crate. |
| base64 | 0.23.1 | | SASL payloads. |
| nucleo-matcher | 0.3.1 | 2024-02-20 | Fuzzy buffer switcher (Phase 2). Used by Helix and stable. Licence is MPL-2.0 (file-level copyleft, fine as a dependency). |
| open | 5.4.5 | 2026-10-22 | Open URLs. More active than `opener`. |
| notify-rust | 4.18.2 | 2026-10-07 | Desktop notifications (Phase 2). OSC 777 and OSC 9 are the primary path. |
| arboard | 3.7.0 | 2026-10-15 | Clipboard fallback behind a feature flag. OSC 52 is the default because it works over SSH. |
| rusqlite | 0.41.1 | 2026-10-28 | SQLite search index (Phase 3), `bundled`. Synchronous calls on a dedicated thread; `sqlx` is unnecessary weight. |
| mlua | 0.12.2 | 2026-10-03 | Scripting (Phase 4). |

### Added in M2 (checked 2026-10-30)

| Crate | Version | Why |
|---|---|---|
| tokio | 1.54.1 | Runtime. `irsty-core` enables only `rt`, `net`, `time`, `sync`, `macros`, `io-util` and `process`; the binary adds `rt-multi-thread` and `signal`. |
| tokio-util | 0.7.20 | `Framed` codec and `Either` for plain-or-TLS streams. |
| tokio-rustls | 0.26.6 | TLS. rustls is used through its re-export so the versions can't drift. |
| rustls-platform-verifier | 0.7.1 | OS trust store via `BuilderVerifierExt::with_platform_verifier`. |
| rustls-webpki | 0.103.16 | Added 2026-10-31. Reads the names a certificate covers when the macOS or Windows verifier reports a bare name mismatch. Already in the tree through rustls; `alloc` feature only. |
| rustls-pki-types | 1.15.1 | PEM loading for client certificates (`PemObject`). Replaces `rustls-pemfile`, which has had no release since 2024-09 and whose parser moved here. |
| keyring | 4.2.0 | Secrets. 4.x is a restructure around `keyring-core`; the default `v1` feature picks Keychain, Credential Manager or Secret Service automatically, and `Entry::new(service, user)?.get_password()` is all we need. On Linux it pulls in zbus 5; see the deny exceptions below. |
| toml | 1.1.8 | Config (TOML 1.1). |
| directories | 6.0.0 | Platform paths. |
| jiff | 0.2.38 | Timestamps and `server-time` parsing. |
| futures | 0.3.34 | `SinkExt` and `StreamExt` on the framed stream. |
| bytes | 1.12.1 | Codec buffers. |
| tracing-subscriber | 0.3.23 | `env-filter` and `fmt`, writing to a file. |
| tempfile | 3.27.0 | Test-only. |

Ergo v2.19.1 (2026-08-05) for integration tests: the container `ghcr.io/ergochat/ergo:v2.19.1` in CI, or the checksum-verified release binary locally.

### Added in M4 (checked 2026-10-31)

| Crate | Version | Why |
|---|---|---|
| sha2, hmac, pbkdf2 | 0.11.1, 0.13.0, 0.13.0 | SCRAM-SHA-256 in `irsty-proto`. RustCrypto, current; `hmac` and `sha2` were already in the tree. `pbkdf2` has only its `hmac` feature. |
| stringprep | 0.1.5 (2024-05-25) | SASLprep for SCRAM names and passwords. No release since 2024, but it implements a frozen spec and is used by sqlx and tokio-postgres (about 74 million recent downloads). |

### Dev dependencies

| Crate | Version | Why |
|---|---|---|
| insta | 1.49.0 | Snapshot tests for the UI (arrives in M3). |
| rcgen | 0.14.10 | Added 2026-10-31. Generates fresh certificates in TLS tests, so no fixture can expire. Maintained by the rustls project. `aws_lc_rs` and `pem` features only. Test-only. |
| proptest | 1.12.0 | Round-trip and no-panic properties. |
| criterion | 0.8.2 | Benchmarks for the parser and formatter. Uses `std::hint::black_box`. |
| serde-saphyr | 2.0.0 (2026-10-13) | Loads the parser-tests YAML. `serde_yaml` is deprecated, and `serde_yaml_ng` and `serde_norway` haven't released since 2024. This one is designed to be panic-free. Test-only. |
| libfuzzer-sys | 0.4.13 | Fuzz targets in `fuzz/`, which sit outside the workspace because they need nightly. |

### Developer tools (pinned in `mise.toml`)

| Tool | Version | Notes |
|---|---|---|
| cargo-nextest | 0.9.150 | |
| cargo-deny | 0.20.2 | |
| cargo-audit | 0.22.2 | |
| cargo-llvm-cov | 0.9.1 | Needs the `llvm-tools` rustup component. |
| cargo-insta | 1.49.0 | Matches the `insta` crate version. Used by `mise run snap`. |
| bacon | 3.26.0 | `mise run watch`. AGPL-3.0, which is fine for a tool we run but never link. Chosen over cargo-watch, which is on life support and recommends bacon. |
| cargo-fuzz | 0.13.2 | Needs nightly (1.101.0-nightly 2026-10-29 when checked). On networks where nightly cargo stalls fetching the index, set `CARGO_HTTP_MULTIPLEXING=false`. |

### Test data

- `ircdocs/parser-tests`: CC0-1.0. Last commit was 2023-05-29, which is fine because it is a stable corpus. Files: `msg-split`, `msg-join`, `userhost-split`, `mask-match` and `validate-hostname` (`.yaml`).
- Ergo v2.19.1 (2026-08-05) for integration tests. `mise run ergo` downloads and verifies the release binary; CI uses the container image.

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
| webpki-roots 1.0.11 | CDLA licence, bundled roots. | Use the platform verifier. It brings `webpki-root-certs` (CDLA-Permissive-2.0, Mozilla's root data) as its Linux fallback; `deny.toml` allows that licence for that crate only. |
| rustls-pemfile 2.2.0 | No release since 2024-09; its PEM parser now lives in `rustls-pki-types`. | Not used. |
| zbus 5 internals (`zvariant`, `zvariant_derive`, `zbus_names`) | Flagged unmaintained (RUSTSEC-2026-0370, -0371, -0372): folded into zbus 6. Reached only through keyring's Linux Secret Service store, and `secret-service` 5.2.0 still requires zbus 5. | Ignored in `deny.toml` and `.cargo/audit.toml` with reasons. Revisit when `secret-service` moves to zbus 6. The alternative (the libdbus-based store) would add a C dependency. |
