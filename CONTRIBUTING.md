# Contributing

## Setup

- Rust: rustup installs the toolchain in `rust-toolchain.toml` on first use.
- Gate tools: `mise install` (versions in `mise.toml`), or install `cargo-nextest`, `cargo-deny` and `cargo-audit` yourself.
- MSRV toolchain: `rustup toolchain install 1.97 --profile minimal`.

## Integration tests against Ergo

`crates/irsty-core/tests/ergo.rs` runs against a real [Ergo](https://ergo.chat) server and is skipped unless `IRSTY_ERGO_PORT` is set. CI uses the official container. Locally, either:

```sh
docker run --rm -d --name irsty-ergo -p 127.0.0.1:16667:6667 -p 127.0.0.1:16697:6697 ghcr.io/ergochat/ergo:v2.19.1
IRSTY_ERGO_PORT=16667 IRSTY_ERGO_TLS_PORT=16697 cargo nextest run -p irsty-core --test ergo
```

or run the release binary: download `ergo-<version>-<platform>.tar.gz` and the checksums file from the [releases page](https://github.com/ergochat/ergo/releases), verify the checksum, then in a scratch directory copy `default.yaml` to `ircd.yaml`, change the listeners to `127.0.0.1:16667` and `127.0.0.1:16697`, and run `ergo mkcerts --conf ircd.yaml && ergo run --conf ircd.yaml`.

The suite registers its own accounts and uses unique nicks and channels, so it can run repeatedly against the same server. Ergo's default limit of 32 connections per IP per 10 minutes is ample.

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
