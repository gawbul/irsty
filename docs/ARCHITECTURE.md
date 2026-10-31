# Architecture

This is a summary; the reasoning lives in the [ADRs](adr/).

## Crates

```
irsty  ──►  irsty-tui  ──►  irsty-core  ──►  irsty-proto
(binary)    (ratatui)       (tokio, state)   (sans-IO, no async)
```

- **irsty-proto:** parses and serialises IRC messages, handles tags, `RPL_ISUPPORT` and casemapping, parses and sanitises mIRC formatting, and implements CAP and SASL as state machines. It has no I/O, so everything is deterministic and fuzzable ([ADR 0001](adr/0001-own-protocol-crate.md)).
- **irsty-core:** runs connection tasks (TCP, TLS, SOCKS5, the rate-limited send queue, reconnect) and holds client state (networks, buffers, channels, users). The `Action`/`update`/`Effect` loop lives here, along with logging and the config model. It has no UI dependencies.
- **irsty-tui:** renders from `&State` and maps terminal events to `Action`s. It includes our own scrollback and input widgets ([ADR 0003](adr/0003-bespoke-scrollback-and-input.md)), keymaps, themes, and the terminal guard (panic hook, signals, suspend and resume).
- **irsty:** the CLI, tracing setup (writes to a file only), and wiring.

## Event flow

```
 crossterm EventStream ──► input task ──(bounded 64)───┐
 signals ──► signal task ──────────────────────────────┤
 timers ───────────────────────────────────────────────┤
                                                       ▼
 conn task (net A) ──(bounded 1024, shared)──► ┌────────────────┐ ──► render if dirty (≤ 60 fps)
 conn task (net B) ───────────────────────────►│ app loop       │
        ▲                                      │ update(state,  │ ──► Effects
        └────────── NetCommand (bounded 256) ◄─│        action) │      ├► conn tasks (try_send)
                                               └────────────────┘      ├► log writer thread
                                                                       └► notifier, sqlite (later)
```

One task owns `State`. Each event becomes an `Action`. `update` mutates state and returns `Effects` without doing any I/O. Pending events are drained before each render, so bursts collapse into one frame ([ADR 0002](adr/0002-unidirectional-state.md)).

## Connections

Each network has one connection task (`irsty_core::net::connection`). It:

- opens TCP, racing IPv6 and IPv4 addresses (`net::dial`, a simplified Happy Eyeballs) or going through a SOCKS5 proxy (`net::socks`, which hands host names to the proxy unresolved), then TLS with `rustls` and the OS trust store (`rustls-platform-verifier`, wrapped so rejected certificates can be explained in plain words), optionally presenting a client certificate;
- registers (`PASS`, `CAP LS 302`, `NICK`, `USER`), walking the nick list on 433 and friends;
- drives `irsty-proto`'s CAP and SASL state machines, holding `CAP END` until SASL finishes. SCRAM gets its nonce from the TLS provider's secure random source, and a mechanism the server doesn't list is never tried, nor swapped for another;
- answers `PING` at once, ahead of the queue, and sends its own after 90 s of silence, giving up 60 s later;
- sends everything else through a token bucket (a burst of 5, then one every 2 s) so servers don't disconnect us for flooding. Registration lines (`CAP`, `AUTHENTICATE`, `PASS`, `NICK`, `USER`) skip it until the server welcomes us, so a SASL login isn't held back by seconds;
- on failure, rotates to the next server and backs off exponentially with equal jitter (2 s doubling to 5 min, never immediate); a connection that stays registered for a minute resets the backoff.

Joining channels is not its job: on `Registered`, `State::apply` returns `Send` effects for the network's perform lines, then autojoin plus every channel that was joined when the link dropped.

`irsty_core::Client` owns `State`, the event receiver and the per-network command senders. `Client::next` waits for one event, then drains up to 512 more without waiting, applies them, runs the effects, and returns the buffers that changed. It is cancel-safe, so a frontend can `select!` on it alongside terminal input.

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
| App to log writer | `try_send` plus a bounded overflow buffer | Dropped lines are counted, and a marker line is written once there is space. |
| Input to app | bounded 64 | crossterm buffers input in the meantime. |

## Untrusted text

Remote text becomes `StyledText` once, at ingest, through `irsty_proto::format::parse`, and the TUI renders nothing else ([ADR 0005](adr/0005-untrusted-text.md)).

## Terminal safety

A guard enables raw mode, the alternate screen, bracketed paste, focus events and (optionally) mouse capture, and restores the terminal on drop, on panic (via a chained hook), on SIGINT and SIGTERM, and around SIGTSTP and SIGCONT.
