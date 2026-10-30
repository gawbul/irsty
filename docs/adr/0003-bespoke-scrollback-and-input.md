# 0003: Our own scrollback and input widgets

- Status: accepted
- Date: 2026-10-30

## Context

We considered `tui-textarea` (stale since 2024-10), `ratatui-textarea` 0.9, `tui-input` 0.15 and `tui-scrollview` 0.6. The input crates are general text editors that know nothing about tab completion, IRC formatting codes or command history. `tui-scrollview` scrolls a buffer rendered in full ahead of time, which doesn't scale to 100k-line channels and can't do the nick-aligned hanging indent that IRC clients use.

## Decision

Write both widgets ourselves.

- **Scrollback:** a `VecDeque<Line>` per buffer, capped by config. Each `Line` holds pre-parsed styled spans. A wrap cache keyed by width lays out only the visible window, using grapheme clusters and `unicode-width`.
- **Input line:** grapheme-aware editing, emacs keys, history, a completion hook, multi-line paste detection, and insertion of formatting codes.

## Consequences

- Each widget is a few hundred lines that we fully control, with snapshot tests and benchmarks.
- We give up features that general-purpose editors have, such as undo trees and selections. Single-level undo and kill-ring yank are enough for IRC input.
