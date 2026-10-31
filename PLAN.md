# Irsty plan

Status: draft for review, 2026-10-30. Versions and the evidence for them are in [`docs/VERSIONS.md`](docs/VERSIONS.md).

## 1. Stack, in brief

| Concern | Choice | Reason |
|---|---|---|
| Toolchain | Rust 1.99.1, edition 2024, MSRV 1.97 (N-2) | Latest stable. `clippy::incompatible_msrv` keeps us honest. |
| TUI | ratatui 0.30 + crossterm 0.29 (`event-stream`) | ratatui's 0.30 split is settled. Use `ratatui::crossterm` everywhere to avoid having two crossterm versions. |
| Scrollback, input line | **Bespoke** | Off-the-shelf widgets don't do wrap caches over 10k+ lines, IRC formatting spans, nick-aligned hanging indents, or completion and history. Each is a few hundred lines that we fully control. |
| Async | tokio, tokio-util codecs, futures | Standard. |
| TLS | rustls 0.23 (aws-lc-rs) + tokio-rustls + rustls-platform-verifier | No OpenSSL. Uses the OS trust store. Client certs for CertFP. |
| Protocol | **Our own `irsty-proto`**, sans-IO | See ADR 0001 below. |
| Config | serde + toml 1.x, no figment | Layering is three steps (defaults, then file, then CLI) and doesn't justify a stale dependency. |
| Secrets | `password = { keyring = "…" }` or `{ command = ["pass", "irc/libera"] }` | Plaintext is allowed only with an explicit `{ plain = "…" }` and a warning at startup. |
| Persistence | Plain-text logs per buffer; rusqlite index in Phase 3 | Logs are greppable. SQLite runs on its own thread. |
| Fuzzy matching | nucleo-matcher | fuzzy-matcher is unmaintained. |
| Time | jiff | Correct time zones and simple `server-time` parsing. |
| Signals | tokio::signal (INT, TERM, WINCH) + signal-hook (TSTP, CONT) | Suspend must restore the terminal, re-raise SIGTSTP with the default handler, then re-init on CONT. |
| Scripting (Phase 4) | mlua | Lua 5.4 or Luau is an open question (§6). |

## 2. Workspace layout

```
irsty/
├── Cargo.toml                 # [workspace], shared [workspace.package], [workspace.lints], [workspace.dependencies]
├── rust-toolchain.toml        # 1.99.1, components: rustfmt, clippy
├── deny.toml, clippy.toml, rustfmt.toml, .config/nextest.toml
├── crates/
│   ├── irsty-proto/           # no I/O, no async, no tokio. Depends on: thiserror, base64, sha2/hmac/pbkdf2 (SCRAM), jiff (optional)
│   │   ├── message.rs         #   zero-copy MessageRef<'a> + owned Message, serialiser
│   │   ├── tags.rs            #   tag escaping and unescaping, client-only (+) tags, size limits
│   │   ├── isupport.rs        #   005 parsing: PREFIX, CHANTYPES, CHANMODES, CASEMAPPING, MONITOR, LINELEN, …
│   │   ├── casemap.rs         #   ascii, rfc1459, rfc1459-strict; owned normalised key types
│   │   ├── cap.rs             #   CAP LS 302 / REQ / ACK / NAK / NEW / DEL state machine
│   │   ├── sasl.rs            #   PLAIN, EXTERNAL, SCRAM-SHA-256 mechanisms (sans-IO)
│   │   ├── format.rs          #   mIRC control codes to styled spans; sanitiser (see §3.4)
│   │   ├── ctcp.rs, split.rs  #   CTCP framing; message splitting at UTF-8 and grapheme boundaries within byte budgets
│   │   └── numerics.rs
│   ├── irsty-core/            # tokio, no UI deps. Connections, state, actions, effects, logging, config model
│   ├── irsty-tui/             # ratatui: views, input line, keymaps, themes, terminal guard
│   └── irsty/                 # binary: clap CLI, tracing setup, wiring, color-eyre
├── tests/fixtures/            # parser-tests YAML (CC0, vendored with attribution), mock-server scripts
├── fuzz/                      # cargo-fuzz targets (nightly only, excluded from the workspace)
├── benches/ (per crate)
└── docs/ (ARCHITECTURE, CONFIG, KEYBINDINGS, COMMANDS, VERSIONS, adr/)
```

The dependency direction is strictly `irsty → irsty-tui → irsty-core → irsty-proto`. `irsty-tui` reads core state through `&State` and emits `Action`s; it never touches sockets.

## 3. Event and state model

### 3.1 Tasks and channels

```
 crossterm EventStream ──► [input task] ──(bounded 64)──┐
 signals (INT/TERM/TSTP/CONT/WINCH) ──► [signal task] ──┤
 timers (reconnect, typing expiry, ping timeout) ───────┤
                                                       ▼
 [conn task: net A] ──(bounded 1024, shared)──► ┌──────────────┐ ──► render (if dirty, ≤ 60 fps)
 [conn task: net B] ───────────────────────────►│  app loop    │
          ▲                                     │ update(state,│──► Effects ─► conn cmd tx (bounded 256, per network)
          └──────────── NetCommand ◄────────────│   action)    │           ├► log writer thread (bounded 4096)
                                                └──────────────┘           ├► notifier
                                                                            └► sqlite thread (Phase 3)
```

- **Connection task (one per network):** owns the socket, the `Framed` codec (`LinesCodec`-style with a hard 8191+512 byte cap and invalid-UTF-8 fallback), the token-bucket send queue, keepalive and lag measurement, and the CAP/SASL machines from `irsty-proto`. It emits `NetEvent::{Connecting, Connected, Message(Message), Disconnected(reason), Lag(Duration)}`. Reconnection uses exponential backoff with full jitter (1 s base, 5 min cap) and resets after a stable period.
- **App loop (single task, owns `State`):** `select!` over all receivers. Each event becomes an `Action`. `update(&mut State, Action) -> Effects` is pure apart from mutating state, so it is unit-testable without tokio. After updating, it drains up to N more pending events before rendering, which coalesces bursts (a netsplit or ZNC playback becomes one frame instead of thousands).
- **Render:** happens only when `state.dirty` is set, limited to 60 fps, and never awaits. Drawing reads state; it does not mutate it.

### 3.2 Back-pressure

- Net to app is bounded. If the UI falls behind, connection tasks block on `send`, which slows reads, and TCP pushes back on the server. Data is never dropped silently.
- App to net uses `try_send`. A full queue becomes a visible error ("send queue full") in that buffer. The app loop never blocks on a network.
- App to log writer uses `try_send` with a bounded overflow buffer. If that also fills, we count dropped lines and write a single marker line when space returns.

### 3.3 State

`State { networks: SlotMap<NetId, Network>, buffers: SlotMap<BufId, Buffer>, ui: UiState, config: Arc<Config> }`. Channels and users are keyed by casemapped key types (`ChanKey`, `NickKey`) so that mixing raw strings is a type error. When CASEMAPPING arrives in 005, the maps are re-keyed. Each buffer's scrollback is a `VecDeque<Line>` capped by config (default 10,000). Each `Line` stores pre-parsed `Vec<Span>` plus metadata (time, kind, msgid, highlight flag). The view keeps a wrap cache keyed by `(BufId, width)` and lays out only the visible window.

### 3.4 Untrusted text

All remote text goes through `irsty_proto::format::parse` exactly once, at ingest. Its output is `StyledText`, which is the only text type the TUI will render. The parser recognises the mIRC codes (`\x02 \x03 \x04 \x0f \x11 \x16 \x1d \x1e \x1f`). It drops or replaces with U+FFFD every other C0/C1 control, DEL, ESC sequences, and also bidi overrides (U+202A–U+202E, U+2066–U+2069) from nicks and other identifiers. Tests cover CSI, OSC, DCS, 8-bit C1 (U+009B), and attempts to inject OSC 52, OSC 8 and title-setting sequences.

### 3.5 Key ADRs (to be written in M0)

- **0001 Own protocol crate.** irc-proto has had no release since March 2025, has 37 open issues and is MPL-2.0. Its tag and IRCv3 support is incomplete, and it allocates per message. A sans-IO parser and serialiser is roughly 1,500 lines including tests, gives us zero-copy `MessageRef<'a>`, and lets CAP and SASL logic be tested without sockets.
- **0002 Unidirectional state and sans-IO state machines.**
- **0003 Bespoke scrollback and input widgets.**
- **0004 Secrets: keyring or command, never plaintext by default.**
- **0005 Untrusted text sanitisation at ingest.**
- **0006 MSRV policy (N-2).**
- **0007 OSC 52 and OSC 777 before native clipboard and notifications** (they work over SSH and inside tmux with passthrough).

## 4. Milestones and acceptance criteria

Every milestone ends with the full gate passing locally and in CI: `fmt --check`, `clippy --all-targets --all-features -D warnings` (with selected pedantic lints), `nextest`, `deny`, `audit`, and an MSRV build. Each milestone is a small series of conventional commits and goes in as one PR.

### Phase 1: foundation

**M0. Skeleton.** `git init`, workspace, toolchain file, lints, `deny.toml`, GitHub Actions (gate matrix on Linux and macOS, MSRV job, audit on a schedule), `CLAUDE.md`, ADRs 0001 to 0007, stub docs, `forbid(unsafe_code)` in every crate.
*Accept:* CI is green on an empty build, and `cargo run -- --version` works.

**M1. irsty-proto.** *(Done 2026-10-30.)* Message parse and serialise (zero-copy and owned), tags, ISUPPORT, casemapping, mIRC formatting and sanitiser, CTCP framing, message splitting, CAP LS 302 machine, SASL PLAIN.
*Accept:* every case in the parser-tests `msg-split`, `msg-join`, `userhost-split` and `mask-match` files passes. A proptest round-trip (`serialise(parse(x)) ≡ x` for canonical messages) passes. A fuzz target runs for 10 minutes with no panics. Escape-injection tests pass. There is a criterion baseline for parsing. Coverage of the crate is ≥ 90%.

**M2. irsty-core connection.** *(Done 2026-10-30. The Ergo suite checks reconnection with a client-requested reconnect against the real server; server-side drops, ping timeouts and refused connections are covered against the mock server, rather than by restarting the container mid-test.)* TCP and TLS (platform verifier, optional client cert), registration with nick fallbacks, PING/PONG and lag, CAP negotiation, SASL PLAIN, reconnect with backoff, rate-limited send queue, state model (networks, buffers, channels, users, modes from NAMES, JOIN, PART, QUIT, NICK, KICK, MODE, TOPIC), a minimal config with secrets, and log files per buffer.
*Accept:* a scripted mock server covers registration, CAP NAK, SASL failure, ping timeout and abrupt disconnect. Ergo in Docker covers registration, SASL, join/part/message between two clients, and reconnect after the container restarts. `irsty --headless` connects and logs.

**M3. Basic TUI.** *(Done 2026-10-31. Snapshots, the 0.3 ms frame benchmark and a pseudo-terminal run against Ergo pass; the day on Libera is yours to do.)* Terminal guard (panic hook, signals, suspend and resume, bracketed paste, focus, resize, optional mouse), layout (buffer list, topic bar, scrollback, nicklist, status bar, input line), our own input line with history, and commands `/join /part /msg /query /me /quit /raw /connect /disconnect /close /help`, with `/raw` output redacted.
*Accept:* insta snapshots for each view at 80×24 and 200×50, plus wide-character and emoji cases. A 100k-line buffer scrolls at under 2 ms per frame (criterion). Manual check: I use it on Libera for a day.

### Phase 2: daily driver

- **M4. Multiple networks and per-network config:** *(Done 2026-10-31.)* autojoin, perform, SOCKS5, IPv6, SASL EXTERNAL, SCRAM-SHA-256. *Accept:* SCRAM test vectors from RFC 7677 pass, and Ergo's CertFP login works.
- **M5. IRCv3 caps:** *(Done 2026-10-31.)* `message-tags`, `server-time`, `account-tag`, `account-notify`, `away-notify`, `extended-join`, `multi-prefix`, `userhost-in-names`, `chghost`, `setname`, `echo-message`, `labeled-response`, `batch`, `invite-notify`, `cap-notify`, `sasl`, `standard-replies` and `monitor`. *Accept:* each cap has a mock-server test. Echo-message dedup and labeled-response correlation are tested.
- **M6. Full command set, aliases, `/set` and `/bind`, ignore, highlight.** The command set includes `/whois` and `/whowas` (replies gathered into one readable block in the current buffer rather than raw numerics in the server buffer), `/who`, `/names`, `/topic`, `/mode`, `/op`, `/deop`, `/voice`, `/kick`, `/ban`, `/invite`, `/away`, `/notice`, `/ctcp`, `/list`, `/clear` and `/window`. Until then, `/raw WHOIS <nick>` works and its replies appear in the server buffer. Also: start the UI with no config file (an empty server buffer with a hint), and `/server add|del|update|list` to manage networks from inside Irsty. Both `/set` and `/server` write `config.toml` back with `toml_edit`, so comments and layout survive, and secrets stay as keyring or command references. *Accept:* docs/COMMANDS.md is generated from or checked against the command table. A first run with no config reaches a connected channel using only commands, and a round-trip test shows edits keep comments.
- **M7. Input polish:** tab completion, multi-line paste prompt, emacs keys, Alt+number, fuzzy switcher.
- **M8. Formatting, nick colours, URLs, activity and highlight tracking, notifications, CTCP with flood protection, long-message splitting, log rotation.**

### Phase 3: modern and bouncer

- **M9.** chathistory, read-marker, soju bouncer-networks, ZNC playback. *Accept:* tested against soju in Docker.
- **M10.** typing, reply, react, multiline.
- **M11.** Search (SQLite index), jump-to-unread, split windows and layouts.
- **M12.** Themes, colour fallback, NO_COLOR, `/reload`, `--check-config`, `--print-default-config`.
- **M16. DCC:** `/dcc chat|send|get|resume|close|list`, as in irssi. CHAT, SEND and RESUME/ACCEPT, plus passive (reverse) DCC for users behind NAT. Nothing is ever accepted automatically: offers wait for `/dcc get` or `/dcc chat`. File names are untrusted (no path components, sanitised, never overwrite) and go to a configurable download directory. A configurable port range and advertised IP, a warning the first time an offer reveals your IP, and size checks against the advertised length. (Numbered after M15 to keep existing references stable.) *Accept:* a transfer and a chat work between two Irsty instances and against irssi, and tests cover hostile file names, truncated transfers and resume offsets.
- *(Optional)* inline images behind a feature flag.

### Phase 4: extensibility and polish

- **M13.** Scripting API (events, commands, buffers, timers), sandboxed by default.
- **M14.** Accessibility mode, high-contrast theme.
- **M15.** Man page, completions, cargo-dist releases, Homebrew, AUR and Nix notes.

## 5. Things I'd push back on or do differently from the brief

- **Hot-reload of the config file:** use an explicit `/reload` only. File watchers fire on partial writes from editors, and silently reloading a broken config mid-session is worse than a command.
- **Inline images:** keep this out of scope until Phase 3 is done. The terminal graphics protocols interact badly with ratatui's diffing and with tmux.
- **The `irc-parser-tests` corpus is from 2023 and doesn't cover everything.** It is still worth vendoring, but our own IRCv3 tag cases (client tags, escaping edge cases, byte limits) will form most of the corpus.
- **Fuzzing needs nightly.** It runs in its own CI job with a nightly toolchain, separate from the stable pin.

## 6. Decisions (agreed 2026-10-30)

1. **Keybindings:** irssi is the default; a `keymap = "weechat"` preset covers the conflicts.
2. **Config shape:** a single `config.toml` for now.
3. **Windows:** best effort, with no CI gate.
4. **GitHub:** a public repository at `gawbul/irsty`.
5. **Scripting runtime:** Luau, through mlua's `luau` feature (ADR 0008).
