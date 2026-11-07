# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `ROADMAP.md`: what could come next, starting with the IRCv3 extensions Irsty doesn't support yet.
- Scoop: `scoop bucket add gawbul https://github.com/gawbul/scoop-bucket`, then `scoop install gawbul/irsty`. The `Publish Scoop manifest` workflow updates the bucket after each release, and can be run by hand for an existing one.
- A Nix flake: `nix run github:gawbul/irsty` or `nix profile install github:gawbul/irsty`, on Linux and macOS.
- The AUR package in `packaging/aur` is ready to publish, with its checksum and `.SRCINFO`; `packaging/nix/package.nix` is ready for nixpkgs, with real hashes.
- The `Packaging` workflow builds and installs the AUR package, the flake and the Scoop manifest in CI; `mise run packaging:aur` and `packaging:nix` run the first two in Docker.

### Changed

- `Publish crates` and `Publish Scoop manifest` share their release checks, in `scripts/check-release-tag.sh`.
- The project site has a front page of its own: a tour of Irsty's windows, the move from irssi and WeeChat, scripting, accessibility and every way to install. Each terminal screen on it is a real render, kept current by a test. The docs moved to `/irsty/docs/` and now share the site's fonts and colours.

### Fixed

- The light theme's status bar showed the `[Act: ...]` list in blue and magenta on the blue bar, so buffer numbers could vanish. The bar now has its own theme elements, `act_messages` and `act_highlight` (bold white, and bold black on yellow, in the light theme; the dark theme looks as before).

## [0.1.0] - 2026-11-07

The first release.

### Release

- Published to crates.io (`cargo install irsty`), with release archives, installers and checksums on GitHub, and a project site at https://gawbul.github.io/irsty/ built from the docs. The `Publish crates` workflow publishes the crates to crates.io after each tagged release, with trusted publishing.
- `brew install gawbul/tap/irsty`: dist pushes a formula for the prebuilt binaries to the tap with each release, replacing the source-built template.
- `irsty-core` and `irsty-tui` say they are Irsty's internals with no API stability promise; every crate carries the licence.

### Fixed

- `/set` showed `typing.send` and `typing.show` without their values; a test now checks every setting has one.
- `422` (no MOTD) no longer shows as an error.
- A batch inside another (such as a multi-line message in chat history) is now handled with its parent, in order, instead of as soon as it ends.
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

- M16, DCC: `/dcc list|get|resume|send|chat|close`, as in irssi. File transfers with resume (`RESUME`/`ACCEPT`), chats in `=nick` buffers, and passive (reverse) DCC for users behind NAT, both ways. Nothing is accepted automatically; offers are listed and notified, and lapse after five minutes. Received files get a sanitised name in `dcc.dir`, are written as `.part` and never replace an existing file, and the offered size is enforced. `[dcc]` has `dir`, `ports` and `ip`; the first offer warns that it reveals your IP address, and DCC is off through a proxy unless `dcc.ip` is set. Works with irssi.
- M15, releases and packaging:
  - `irsty --man` prints a man page (with files, environment and exit status as well as the options), and `irsty --completions <shell>` prints completions for bash, zsh, fish, PowerShell and Elvish. Copies live in `assets/`; `mise run assets` regenerates them.
  - Release archives for macOS (arm64 and x86_64), Linux (x86_64 and arm64) and Windows, with the man page and completions inside, checksums, and shell and PowerShell installers, built by dist from a version tag.
  - Templates for an AUR `PKGBUILD` and a Nix derivation in `packaging/`, and `docs/PACKAGING.md`.
  - Every crate is `publish = false`.
- M14, accessibility:
  - `irsty --plain`: a line-by-line interface for screen readers and braille displays. Nothing is drawn; new lines in the current buffer are printed as they arrive, mentions and private messages from other buffers are announced with where they came from, switching buffers reads out what you hadn't seen, and commands are typed on an ordinary line.
  - `[ui] accessible` (`/set ui.accessible on`): no side panels, ASCII instead of box drawing and symbols, and words beside anything shown only by colour (`!nick` for mentions, `(sending)`, `>` on the focused pane, `!` in the activity list).
  - The `high-contrast` theme preset: no colours of its own, only bold, italic, underline and reverse video in the terminal's own colours. `accessible` picks it unless another preset is set; `[theme] preset` is now optional.
  - A `pending` theme element for your messages until the server confirms them.
- M13, scripting: Luau scripts (`[[script]]`, `/script load|unload|reload|list`) react to events (`irsty.on`), add commands (`irsty.command`), print, set timers and read buffers and nicks, each in a sandboxed state of its own with no file or network access and time, memory and error limits. Sending messages and running commands needs `allow = ["send"]` and is rate-limited. `docs/SCRIPTING.md` documents the API, and `examples/scripts` has working examples.
- Join, part, quit, kick, nick-change and topic-change lines now record who did it.
- M12, looks and config tools:
  - Themes: `[theme] preset` (`dark`, the existing look, or `light`) and a style for any element (`bar = "white on blue"`), including the nick palette. Style strings take modifiers, names, `#rrggbb`, `colour0` to `colour255` and `on` a background.
  - Colour fallback: `[theme] colours` (`auto`, `truecolor`, `256`, `16` or `none`); every colour on screen is fitted to what the terminal shows. `auto` honours `NO_COLOR`, `COLORTERM` and `TERM`; with no colour, bars and highlights show in reverse video.
  - `/reload` reads the config file again and applies it, networks included (added, changed ones reconnect, removed ones close).
  - `irsty --check-config` checks a config file (status 1 on errors, including key bindings, theme and layout), and `irsty --print-default-config` prints one with every setting at its default.
  - `[nick_colours]` takes `#rrggbb` and `colour0` to `colour255` as well as names.
- M11, finding your way around:
  - `/search` (`/grep`): a full-text index of the messages Irsty logs (SQLite with FTS5, `search.sqlite` in the data directory, `[search] enabled`), searched in the current buffer, the network or everywhere (`-all`), optionally from one nick, with matches in bold.
  - Split windows: `/window split`, `vsplit`, `merge [all]` and `pane`, with F7 and F8 to move focus and a click to focus a pane; `/layout save|apply|reset` keeps them in `ui.layout`, restored at startup.
  - An `unread` rule above the first line you hadn't seen when you came back to a buffer, and Alt+U (WeeChat's keys) or Shift+Alt+U (irssi's) or `/window unread` to jump to it.
- M10, modern messaging:
  - Typing indicators (`+typing`): who is typing shows in the status bar (`typing.show`, on by default). Telling others when you type is opt-in (`typing.send`, off by default), at most once every three seconds per channel, and never for commands.
  - Replies (`+reply`): `/reply <nick> <text>` answers their newest message; replies show what they answer (`↪ alice: …`). The older `+draft/reply` is understood too.
  - Reactions (`+draft/react` and `+draft/unreact`): `/react` and `/unreact`; reactions show after their message with a count, capped per message.
  - Multi-line messages (`draft/multiline`): a pasted block goes as one message where the server supports it, split into batches within its limits, and incoming ones read as one block under one nick, with one notification.
  - `TAGMSG` is understood, so tag-only messages no longer show as empty lines in the server buffer.
- M9, history and bouncers:
  - Chat history (`draft/chathistory`): recent messages on joining, what you missed after a reconnect, private conversations you missed, and older messages when you scroll back past the top. History never notifies or answers CTCP, isn't shown twice, and appears in time order.
  - Read markers shared with your other clients (`draft/read-marker`).
  - soju's networks open as networks of their own (`soju.im/bouncer-networks`), bound with `BOUNCER BIND`.
  - ZNC playback of only what you missed (`znc.in/playback`).
  - `[history] lines` (default 100; 0 turns it off) and a per-network `bouncer_networks` switch.
  - `mise run soju` runs a local soju bouncer in front of Ergo for testing, and `tests/soju.rs` checks Irsty against it.
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
  - Pasting several lines asks first: send them, join them into one line, or cancel. Pasted lines are never run as commands. (Since M10, Enter sends them as one multi-line message where the server supports it.)
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

[Unreleased]: https://github.com/gawbul/irsty/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/gawbul/irsty/releases/tag/v0.1.0
