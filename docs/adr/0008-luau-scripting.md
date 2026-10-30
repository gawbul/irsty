# 0008: Luau for scripting

- Status: accepted
- Date: 2026-10-30

## Context

Phase 4 adds scripting with a strict default: no filesystem or network access. mlua (0.12) supports Lua 5.1 to 5.4, LuaJIT and Luau. Lua 5.4 has to be sandboxed by removing globals by hand, which is fragile. Luau, Roblox's Lua dialect, has a built-in sandbox mode (read-only globals and per-script environments), interrupt callbacks for limiting runaway scripts, and optional gradual typing.

## Decision

Use mlua with the `luau` feature, vendored. Scripts get only the documented Irsty API (events, commands, buffers, timers). Filesystem and network access require explicit capabilities granted per script in the config.

## Consequences

- Scripts written for WeeChat's Lua API won't run unchanged. We don't plan to support that API anyway.
- Luau differs slightly from Lua 5.4 (no `goto`, different integer handling). The script docs will note the differences.
