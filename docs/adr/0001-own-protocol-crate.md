# 0001: Our own sans-IO protocol crate

- Status: accepted
- Date: 2026-10-30

## Context

The obvious existing crates are `irc` and `irc-proto` (1.1.0). Their last release was on 2025-03-24. Pull requests are occasionally merged, there are 37 open issues, and they are licensed MPL-2.0. Their IRCv3 support is partial, with gaps around client-only tags, tag size limits, and caps such as `labeled-response`, `batch` and `chathistory`. Parsing allocates for every message, and the `irc` crate ties protocol logic to its own tokio client, so we can't drive the CAP and SASL logic from our state machine or test it without a socket.

## Decision

Write `irsty-proto`, a pure crate with no I/O and no async. It provides:

- a zero-copy `MessageRef<'a>` borrowing from the line buffer, plus an owned `Message` for passing across channels;
- a serialiser that enforces the 512-byte body limit and the separate 8191-byte tag limit;
- tag escaping, `RPL_ISUPPORT`, casemapping, and mIRC formatting with sanitisation;
- CAP (302) and SASL (PLAIN, EXTERNAL, SCRAM-SHA-256) as state machines: feed in messages, get back messages to send.

Correctness is anchored by the `ircdocs/parser-tests` corpus (CC0) plus our own cases. A fuzz target and proptest round-trip properties catch panics and regressions.

## Consequences

- We own about 1,500 lines of protocol code and its tests. That is acceptable for the core of an IRC client.
- Connection and UI code never see raw strings until they have been parsed and validated.
- We can borrow ideas from irc-proto, but we will not copy its code, which keeps the licence boundary clean.
