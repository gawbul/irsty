# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Registration no longer waits on the output rate limit, so logging in (especially with SCRAM, which sends more lines) takes a fraction of a second instead of several.
- The gate and `mise run cov` clear old coverage data first, which could skew the numbers.
- Certificate errors now explain themselves: a name mismatch lists the names the certificate does cover (on macOS and Windows too), and unknown issuers, expiry, not-yet-valid and revocation get plain-English advice. macOS's raw Security framework codes for these are recognised. The explanation appears once per server; retries show a short reminder.
- Servers show as `host:6697 (TLS)` instead of irssi's `host:+6697`.
- On macOS the config file now lives at `~/.config/irsty/config.toml` (or under `$XDG_CONFIG_HOME`), the same as on Linux, instead of `~/Library/Application Support/irsty/`. Logs stay in the platform data directory.
- Irsty is now licensed under Apache-2.0 only, instead of `MIT OR Apache-2.0` (ADR 0009).

### Added

- M5, IRCv3: `account-notify`, `account-tag`, `away-notify`, `batch`, `chghost`, `echo-message`, `extended-join`, `invite-notify`, `labeled-response`, `setname` and `standard-replies`, plus `MONITOR` with a per-network `monitor` list.
  - With `echo-message`, your messages show dimmed until the server echoes them; `labeled-response` ties each echo or error to its line, and lines that fail say why. Only confirmed lines are logged.
  - Netsplits and netjoins collapse into one line per channel.
  - Away users are dimmed in the nick list.
- M4, multiple networks and per-network config:
  - SASL SCRAM-SHA-256 (RFC 7677). The server must prove it knows the password, hostile parameters are rejected, and a mechanism the server doesn't list is never tried or swapped for a weaker one.
  - CertFP login with SASL EXTERNAL, tested against Ergo.
  - `perform`: raw lines sent after registration, with `$nick` substitution.
  - SOCKS5 proxies per network, with optional login. DNS goes through the proxy.
  - IPv6: addresses are raced Happy Eyeballs style, and bracketed literals work in config.
- A test keeps the example in docs/CONFIG.md valid.
- mise tasks for building, running, testing, linting, coverage, fuzzing, benchmarks and snapshot review (`mise tasks` lists them), and `scripts/ergo.sh` (`mise run ergo`) to run a checksum-verified local Ergo server for integration tests.
- Workspace skeleton: `irsty-proto`, `irsty-core`, `irsty-tui` and the `irsty` binary.
- CI: fmt, clippy, tests on Linux and macOS, MSRV check, cargo-deny and cargo-audit.
- Architecture decision records 0001 to 0008.
- `irsty-proto`: zero-copy message parser and validating serialiser; IRCv3 tags; sources; `RPL_ISUPPORT`; casemapping (ascii, rfc1459, rfc1459-strict, Unicode); wildcard masks; mIRC formatting with sanitisation of untrusted text and bidi balancing; CTCP; message splitting on grapheme boundaries; CAP 302 negotiation; SASL PLAIN and EXTERNAL; a redacting `Secret` type; Windows-1252 fallback for invalid UTF-8.
- `irsty-core`: configuration model with keyring, command and (warned) plaintext secrets; TCP and TLS connections with OS certificate verification, explicit insecure mode and client certificates; registration with nick fallback; CAP and SASL; keepalive with lag measurement; rate-limited output; reconnect with jittered exponential backoff and server rotation; network, buffer, channel and member state with automatic rejoin; per-buffer plain-text logs.
- Terminal UI (`irsty`): buffer list, scrollback with word wrap and aligned nicks, nick list, topic and status bars, and an input line with emacs-style editing and history; irssi keybindings; commands `/join /part /msg /query /me /nick /raw /connect /disconnect /close /quit /help`; mouse support behind `[ui] mouse`; suspend with Ctrl+Z; the terminal is restored on exit, panic and suspend.
- Messages are split to fit the line limit and shown locally; passwords sent with `/raw` or to services are redacted on screen and in logs.
- Buffer activity levels (events, messages, highlights) and `Alt+A` to jump to the most important.
- `[ui]` configuration: `mouse`, `nicklist`, `timestamp_format`.
- `irsty --headless`: connect and print every buffer to stdout, quitting cleanly on Ctrl-C or SIGTERM. Diagnostics go to `irsty.log` in the data directory.
- Integration tests against a scripted mock server and against Ergo (in CI as a service container).
- `Message::as_ref` borrows an owned message as a `MessageRef` without serialising.
- Conformance tests against the ircdocs parser-tests corpus, proptest properties, three fuzz targets, criterion benchmarks, snapshot tests of the UI, and a coverage gate in CI.
