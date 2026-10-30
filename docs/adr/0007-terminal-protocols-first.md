# 0007: OSC 52 and OSC 777 before native clipboard and notifications

- Status: accepted
- Date: 2026-10-30

## Context

Many people run IRC clients on a remote host inside tmux or screen, over SSH. Native clipboard (`arboard`) and desktop notifications (`notify-rust`) then act on the remote machine, which is useless.

## Decision

- **Clipboard:** OSC 52 is the default. `arboard` is available behind a cargo feature and a config option, for local-only users.
- **Notifications:** OSC 777 (`notify`) and OSC 9 are the defaults, with the terminal bell as the fallback. `notify-rust` is available behind a feature.
- When running under tmux, sequences are wrapped for passthrough, and the docs explain `set -g allow-passthrough on`.

## Consequences

- Behaviour depends on the terminal. We document which terminals support what and detect what we can (`TERM_PROGRAM`, `TMUX`).
- Fewer native dependencies in the default build.
