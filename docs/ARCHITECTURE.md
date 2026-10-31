# Architecture

This is a summary; the reasoning lives in the [ADRs](adr/).

## Crates

```
irsty  ──►  irsty-tui  ──►  irsty-core  ──►  irsty-proto
(binary)    (ratatui)       (tokio, state)   (sans-IO, no async)
```

- **irsty-proto:** parses and serialises IRC messages, handles tags, `RPL_ISUPPORT` and casemapping, parses and sanitises mIRC formatting, and implements CAP and SASL as state machines. It has no I/O, so everything is deterministic and fuzzable ([ADR 0001](adr/0001-own-protocol-crate.md)).
- **irsty-core:** runs connection tasks (TCP, TLS, SOCKS5, the rate-limited send queue, reconnect) and holds client state (networks, buffers, channels, users). `State::apply` turns each network event into state changes and `Effect`s; logging, the config model and editing the config file live here too. It has no UI dependencies.
- **irsty-tui:** renders from `&State` and turns keys and commands into calls on the `Client`. It includes our own scrollback and input widgets ([ADR 0003](adr/0003-bespoke-scrollback-and-input.md)), the keymap, completion, notifications, the theme, and the terminal guard (panic hook, signals, suspend and resume).
- **irsty:** the CLI, tracing setup (writes to a file only), wiring, and headless mode.

## Event flow

```
 conn task (net A) ──NetEvent (bounded 1024, shared)──► ┌──────────────────┐
 conn task (net B) ────────────────────────────────────►│ Client::next     │
        ▲                                               │  State::apply    │──► Effects ─┬► NetCommand (try_send, bounded 256)
        └────────────────── NetCommand ◄────────────────│  (owns State)    │             ├► log writer thread (try_send, bounded 4096)
                                                        └────────┬─────────┘             ├► notifications (queued for the UI)
                                                                 │                       └► changed buffers
                                                                 ▼
 crossterm EventStream ─┐                              ┌──────────────────┐
 SIGTERM, SIGHUP, SIGTSTP ┼─────────────────────────────►│ UI loop (run.rs) │──► render if dirty (at most every 16 ms)
 one-second clock ──────┘                              │ select! over all │
                                                        └──────────────────┘
```

One owner holds `State`: the `Client`, which the UI loop drives. `Client::next` waits for one network event, then drains up to 512 more without waiting, applies each with `State::apply`, carries out the effects, and returns the buffers that changed. `apply` mutates state and returns `Effect`s without doing any I/O, so it is tested with plain values. `NetEvent` plays the part of the `Action` in [ADR 0002](adr/0002-unidirectional-state.md); commands from the UI call `Client` methods (`say`, `send`, `add_network` and so on). `Client::next` is cancel-safe, so the UI loop can `select!` on it alongside terminal input, signals and the clock, and bursts collapse into one frame.

## Connections

Each network has one connection task (`irsty_core::net::connection`). It:

- opens TCP, racing IPv6 and IPv4 addresses (`net::dial`, a simplified Happy Eyeballs) or going through a SOCKS5 proxy (`net::socks`, which hands host names to the proxy unresolved), then TLS with `rustls` and the OS trust store (`rustls-platform-verifier`, wrapped so rejected certificates can be explained in plain words), optionally presenting a client certificate;
- registers (`PASS`, `CAP LS 302`, `NICK`, `USER`), walking the nick list on 433 and friends;
- drives `irsty-proto`'s CAP and SASL state machines, holding `CAP END` until SASL finishes. SCRAM gets its nonce from the TLS provider's secure random source, and a mechanism the server doesn't list is never tried, nor swapped for another;
- answers `PING` at once, ahead of the queue, and sends its own after 90 s of silence, giving up 60 s later;
- sends everything else through a token bucket (a burst of 5, then one every 2 s) so servers don't disconnect us for flooding. Registration lines (`CAP`, `AUTHENTICATE`, `PASS`, `NICK`, `USER`) skip it until the server welcomes us, so a SASL login isn't held back by seconds;
- on failure, rotates to the next server and backs off exponentially with equal jitter (2 s doubling to 5 min, never immediate); a connection that stays registered for a minute resets the backoff.

Joining channels is not its job: on `Registered`, `State::apply` returns `Send` effects for the network's perform lines, then autojoin plus every channel that was joined when the link dropped.

## IRCv3

- **Batches** (`irsty_core::batch`): messages tagged with an open batch are held until `BATCH -ref`, then handled together. Netsplits and netjoins become one line per channel; labelled batches are handled with their label; other types are handled in order. Open batches, batch sizes and the bytes held across all batches (4 MiB) are capped, so a hostile server can't make us hold messages forever.
- **Our own messages** with `echo-message`: `State::own_message` records the line as `Delivery::Pending` and, with `labeled-response`, returns a label for the outgoing message. The echo (matched by label, or else by kind and text, oldest first) confirms it in place with the server's time; a labelled error, or a disconnect, marks it `Failed`. Pending lines are logged only once confirmed (`Effect::Log`); `Effect::Updated` redraws without logging. Echoes of services passwords are redacted like the lines we typed.
- **Users**: `Network::user` keeps account, `user@host`, real name and away message for people who share a channel with us, fed by `extended-join`, `account-tag`, `account-notify`, `chghost`, `setname`, `away-notify` and `userhost-in-names`, and forgotten when we no longer share a channel.
- **`MONITOR`**: the per-network watch list is sent once `RPL_ISUPPORT` advertises it; 730 and 731 update `Network::watches`.

## Keys and input

`irsty_tui::keymap` maps key chords (`ctrl-n`, `alt-left`) to named `Action`s: the shared bindings plus the `irssi` or `weechat` preset, then `[keys]` overrides. `app::handle_key` asks the keymap first; unbound plain characters are typed. Three modes take keys before the keymap while they are open: the multi-line paste prompt, the buffer switcher and WeeChat's two-digit jump. `irsty_tui::complete` finds Tab completions from `State` and the input (pure, so tested directly) and scores the switcher's fuzzy matches.

## Notifications and CTCP

`State::push_line` marks a line as a highlight when it mentions our nick or a highlight rule, and emits `Effect::Notify` for highlights and private messages from others. The client queues them (`Client::take_notifications`) and the UI's run loop sends them through `irsty_tui::notify`: it builds the OSC 777 or OSC 9 sequence (wrapped for tmux), strips every control character from the text so nothing can end the sequence early, and rate-limits. They are only sent while the terminal is unfocused, unless configured otherwise.

CTCP requests are answered in `State` (VERSION, PING, TIME, CLIENTINFO, SOURCE) as `Send` effects, three per ten seconds per network at most, using the state's clock so tests are deterministic.

## Changing the config

`irsty_core::config_file::ConfigFile` holds `config.toml` as a `toml_edit` document, so commands can change it without losing comments or layout. Every change is made to a copy, parsed as a full `Config` (so validation is the same as at startup), and only then written, through a temporary file and a rename with mode `0600`. The UI then applies what can change live: `Client::apply_config` updates scrollback, ignores and highlights, and `Client::add_network`, `update_network` (which reconnects) and `remove_network` follow `/server`. Network ids come from a counter that never goes backwards, so a removed network's late events can't land on a new one.

## Replies to commands

`/whois`, `/who`, `/names` and `/list` register what they are waiting for with `State::expect_reply`, naming the buffer they were typed in. Replies are formatted by `irsty_core::replies` (pure functions) and routed there; a WHOIS's details also fill in the user table. Labelled replies (`labeled-response`) route the same way.

## Terminal UI

`irsty-tui` has one async loop (`run.rs`) that selects over `Client::next`, crossterm's `EventStream`, SIGTERM, SIGHUP and SIGTSTP, and a one-second clock. Each wake-up marks the screen dirty; drawing happens at most every 16 ms, so a burst of events costs one frame.

- `view::render` draws everything from `&State` and `&mut Ui` and nothing else, so it is snapshot-tested with ratatui's `TestBackend` and `insta`.
- `Scrollback` lays lines out from the bottom of the screen upwards and stops at the top, so only visible lines are wrapped. A 100,000-line buffer draws in about 0.3 ms at 200×50, wherever it is scrolled to.
- `Ui` holds what isn't client state: the active buffer, per-buffer scroll offsets (anchored so new lines don't move a scrolled-back view), the input line and its history.
- `TerminalGuard` enters raw mode and the alternate screen with bracketed paste and focus reporting (and mouse capture if configured), and restores the terminal on drop, on panic through a chained hook, and around suspend. The UI never asks the terminal for the cursor position, so a terminal that doesn't answer such queries can't stall it.
- On `/quit`, the terminal is restored before waiting for networks to close, so a slow server never leaves the screen frozen.

## Back-pressure

| Edge | Mechanism | When full |
|---|---|---|
| Network to app | bounded `send().await` | The connection pauses reads, and TCP pushes back on the server. Nothing is dropped. |
| App to network | `try_send` | A "send queue full" error appears in the buffer. The app loop never blocks. |
| App to log writer | `try_send` into a queue of 4096 | Dropped lines are counted, and a marker line is written once there is space. |

## Untrusted text

Remote text becomes `StyledText` once, at ingest, through `StyledText::parse` (or `StyledText::plain`, which also drops formatting) in `irsty_proto::format`, and the TUI renders nothing else ([ADR 0005](adr/0005-untrusted-text.md)). Nicks and channel names shown in the UI go through `sanitise_identifier`, and text placed inside terminal escape sequences (notifications) has every control character removed.

## Logging

`irsty_core::log::LogWriter` writes one plain-text file per buffer on its own thread, behind a bounded queue, so the app loop never waits on disk. Each `Effect::Line` names the line it added (by its sequence number in the buffer), so several lines added by one event are all logged. With `[logging] rotate`, the file name carries the line's UTC date. Files are created with mode `0600`.

## Terminal safety

A guard enables raw mode, the alternate screen, bracketed paste, focus events and (optionally) mouse capture, and restores the terminal on drop, on panic (via a chained hook), on SIGTERM and SIGHUP (which quit cleanly), and around SIGTSTP (Ctrl+Z suspends; `fg` resumes). In raw mode Ctrl+C is an ordinary key, used for formatting.
