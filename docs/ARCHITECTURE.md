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
