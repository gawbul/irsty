# Contributing

## Setup

- Rust: rustup installs the toolchain in `rust-toolchain.toml` on first use.
- Gate tools: `mise install` (versions in `mise.toml`), or install `cargo-nextest`, `cargo-deny` and `cargo-audit` yourself.
- MSRV toolchain: `rustup toolchain install 1.97 --profile minimal`.

## Before you push

```sh
./scripts/gate.sh
```

This runs fmt, clippy (warnings denied), nextest, doctests, cargo-deny, cargo-audit and an MSRV check. CI runs the same checks on Linux and macOS.

## Style

- British English. No em dashes anywhere; use commas, colons, semicolons or parentheses.
- Commits use [Conventional Commits](https://www.conventionalcommits.org/) (`feat(proto): …`, `fix(tui): …`, `docs: …`). Keep commits small and focused.
- Every feature ships with tests and docs. Record significant decisions as an ADR in `docs/adr/`.
- No `unsafe`. No panics on input from the network or the user.

## Lints

`clippy::pedantic` is on for the whole workspace, plus `unwrap_used`, `expect_used`, `panic`, `indexing_slicing`, `print_stdout`, `print_stderr`, `dbg_macro` and `todo`. Tests are exempt from the unwrap, expect, panic, indexing and print lints (`clippy.toml`).

Documented exceptions to pedantic:

| Lint | Why it is allowed |
|---|---|
| `module_name_repetitions` | `proto::message::MessageRef` and similar names read better than the alternatives. |
| `must_use_candidate` | Too noisy on simple getters; we add `#[must_use]` where ignoring the result is a real bug. |
| `assert_is_empty` | Its suggested rewrite (`assert_eq!(v, [] as [T; 0])`) is noisier than `assert!(v.is_empty())`, and a failure still names the line. |
| `missing_errors_doc` | Our error enums document themselves. We'll reconsider before 1.0. |

Use a local `#[allow(...)]` with a comment explaining why when a lint is wrong in one specific place.
