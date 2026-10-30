# 0005: Untrusted text is sanitised once, at ingest

- Status: accepted
- Date: 2026-10-30

## Context

Any text from the network (messages, nicks, topics, realnames, CTCP replies, quit reasons) can contain terminal control sequences. If they reach the terminal, they can set the window title, write to the clipboard (OSC 52), forge hyperlinks (OSC 8), move the cursor or spoof other lines. Bidi overrides in nicks can visually disguise who is speaking.

## Decision

`irsty_proto::format::parse` is the only way to turn remote text into `StyledText`. The TUI renders only `StyledText`; the type has no constructor that takes a raw string and keeps it unchanged.

- mIRC codes (bold, colour, hex colour, reset, monospace, reverse, italics, strikethrough, underline) become styles.
- Every other C0 control, DEL, and C1 control (U+0080 to U+009F) is replaced with a visible placeholder or dropped. ESC is never passed through, so no CSI, OSC or DCS sequence can survive.
- Bidi overrides and isolates (U+202A to U+202E, U+2066 to U+2069) are stripped from identifiers (nicks, channel names, hostnames). In message bodies, open overrides are closed at the end of the line, so they can't leak into the next one.

## Consequences

- One function to audit and fuzz. Tests cover CSI, OSC 0/2/8/52, DCS, 8-bit C1 CSI (U+009B), and malformed or truncated sequences.
- Logs store the original text with formatting codes but with control characters escaped, so viewing a log with `cat` is also safe.
