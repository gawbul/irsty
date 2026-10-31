# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Irsty is now licensed under Apache-2.0 only, instead of `MIT OR Apache-2.0` (ADR 0009).

### Added

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
