# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- Two iCloud sync duplicates (`dial 2.rs`, `socks 2.rs`) had been committed; they are gone, `.gitignore` ignores such names, and the gate and CI fail if one is tracked.
- When one event added two lines to the same buffer (such as the user count and the names list after `/names`), the log got the second line twice and never the first. Effects now name the line they added.

### Changed

- The docs describe what was built rather than the original plan: PLAN.md's layout, task model and state, ARCHITECTURE.md's event flow and back-pressure, and VERSIONS.md's planned crates that went unused or aren't needed yet.
- In the irssi preset, Ctrl+B now types bold, as in irssi, instead of moving the cursor left; the WeeChat preset keeps Ctrl+B for moving left.
- `/server <network>` still connects, but `/server` now also manages networks; `/bind` moved from M6 to M7 with the configurable keymap.
- Registration no longer waits on the output rate limit, so logging in (especially with SCRAM, which sends more lines) takes a fraction of a second instead of several.
- The gate and `mise run cov` clear old coverage data first, which could skew the numbers.
- Certificate errors now explain themselves: a name mismatch lists the names the certificate does cover (on macOS and Windows too), and unknown issuers, expiry, not-yet-valid and revocation get plain-English advice. macOS's raw Security framework codes for these are recognised. The explanation appears once per server; retries show a short reminder.
- Servers show as `host:6697 (TLS)` instead of irssi's `host:+6697`.
- On macOS the config file now lives at `~/.config/irsty/config.toml` (or under `$XDG_CONFIG_HOME`), the same as on Linux, instead of `~/Library/Application Support/irsty/`. Logs stay in the platform data directory.
- Irsty is now licensed under Apache-2.0 only, instead of `MIT OR Apache-2.0` (ADR 0009).

### Added

- M8:
  - Formatting keys: irssi's Ctrl+B, Ctrl+C, Ctrl+], Ctrl+_, Ctrl+^, Ctrl+V and Ctrl+O, or WeeChat's Ctrl+C then a letter. Codes show as reversed letters in the input.
  - Nick colours can be turned off or overridden per nick; highlighted lines show the nick dark on yellow.
  - Links are underlined, and `/url` lists and opens them (web links only, without a shell).
  - Desktop notifications for highlights and private messages, through OSC 777, OSC 9 or the bell, when the terminal isn't focused, rate-limited and stripped of control sequences.
  - CTCP replies (`VERSION`, `PING`, `TIME`, `CLIENTINFO`, `SOURCE`), three per ten seconds per network at most.
  - Daily or monthly log rotation.
- M7, input polish:
  - Tab completion of nicks (recent speakers first, `: ` after a nick at the start of a line), channels, commands, aliases and command arguments; Shift+Tab cycles back.
  - Ctrl+G opens a fuzzy buffer switcher; Alt+< and Alt+> go back and forward through the buffers you have shown.
  - Pasting several lines asks first: send them as separate messages, join them into one line, or cancel. Pasted lines are never run as commands.
  - Key bindings are configurable: `keymap = "weechat"` for WeeChat's buffer keys (Alt+J and two digits, F5 and F6, Alt+Up and Alt+Down), `[keys]` overrides, and `/bind` and `/unbind`.
  - Ctrl+T swaps characters.
- M6, the full command set: `/whois /whowas /who /names /topic /mode /op /deop /voice /devoice /kick /ban /unban /invite /away /list /notice /ctcp /clear /window`, with replies gathered in the buffer you typed in.
  - `/server add|del|update|list`, `/set`, `/alias`, `/ignore` and `/highlight` change `config.toml` at once, keeping its comments, and apply live where they can.
  - Irsty starts without a config file and `/server add` creates one.
  - Config gains `[aliases]`, `[[ignore]]` and `[highlight]` (words and regexes).
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
