# 0002: Unidirectional state and a single update step

- Status: accepted
- Date: 2026-10-30

## Context

IRC clients receive events concurrently from several networks, from user input, from timers and from signals. Shared mutable state with locks spread across tasks is hard to reason about and hard to test.

## Decision

Use an Elm or Flux-style loop, with one task owning `State`:

1. Connection, input, signal and timer tasks send events to the app loop over bounded `mpsc` channels.
2. Each event becomes an `Action`, and `update(&mut State, Action) -> Effects` applies it. `update` performs no I/O.
3. `Effects` (send a message, write a log line, notify) are dispatched to the tasks that own those resources, using `try_send` so the loop never blocks.
4. When state is dirty, the view renders from `&State`, at most 60 times a second, after the loop has drained pending events.

Back-pressure is deliberate: network to app uses `send`, so a slow UI slows reads and TCP pushes back on the server. App to network and app to logs use `try_send`, and a full queue becomes a visible error rather than a stall.

## Consequences

- `update` can be unit tested with plain values, without tokio.
- Bursts (netsplits, bouncer playback) collapse into a single frame.
- Everything has to pass through `Action`. That is more ceremony than calling functions directly, but the flow stays easy to trace.
