# Contributing

## Setup

- Rust: rustup installs the toolchain in `rust-toolchain.toml` on first use.
- Everything else: `mise run setup` installs the tools pinned in `mise.toml`, the MSRV toolchain and nightly (for fuzzing). Without mise, install `cargo-nextest`, `cargo-deny`, `cargo-audit`, `cargo-llvm-cov`, `cargo-insta` and `cargo-fuzz` yourself, then `rustup toolchain install 1.97 nightly --profile minimal`.

## Tasks

`mise tasks` lists them all. Each is a thin wrapper over `cargo` or a script in `scripts/`, and extra arguments go to the underlying command.

| Task | What it does |
|---|---|
| `build` (`b`), `build:release`, `check` (`c`) | Build or type-check. |
| `run` (`r`), `run:headless`, `run:ergo` | Run Irsty; `run:ergo` writes a config for the local Ergo server to `target/ergo/irsty.toml`. |
| `watch` (`w`) | bacon: recheck on every save. |
| `fmt`, `fmt:check`, `lint` (`l`), `lint:fix`, `fix` | Format and lint. `fix` applies clippy's suggestions, then formats. |
| `test` (`t`), `test:doc`, `test:all`, `test:ergo` | Tests. Arguments to `test` are nextest filters. |
| `snap`, `snap:test` | Review changed UI snapshots with cargo-insta. |
| `cov`, `cov:proto` | HTML coverage report, or the gate's 90% check on `irsty-proto`. |
| `deny`, `audit`, `msrv`, `gate` (`g`) | Supply-chain checks, the MSRV build and the full gate. |
| `fuzz`, `fuzz:smoke`, `bench` | Fuzz every target (default 600 seconds, or 30 for the smoke run), or run the benchmarks. |
| `ergo` | A local Ergo server (see below). |
| `doc`, `update`, `duplicates`, `setup` | API docs, `cargo update` followed by deny and audit, list iCloud sync duplicates, and setup. |

## Integration tests against Ergo

`crates/irsty-core/tests/ergo.rs` runs against a real [Ergo](https://ergo.chat) server and is skipped unless `IRSTY_ERGO_PORT` is set. CI uses the official container. Locally, the easiest way is:

```sh
mise run ergo        # in one terminal; add --fresh to wipe its accounts and channels
mise run test:ergo   # in another
```

`scripts/ergo.sh` downloads the release for your platform into `target/ergo` (set `ERGO_VERSION` to override 2.19.1), verifies its SHA-256 checksum, and runs it with plain text on 127.0.0.1:16667 and TLS (self-signed) on 127.0.0.1:16697. It supports macOS and Linux. Alternatively, either:

```sh
docker run --rm -d --name irsty-ergo -p 127.0.0.1:16667:6667 -p 127.0.0.1:16697:6697 ghcr.io/ergochat/ergo:v2.19.1
IRSTY_ERGO_PORT=16667 IRSTY_ERGO_TLS_PORT=16697 cargo nextest run -p irsty-core --test ergo
```

or run the release binary: download `ergo-<version>-<platform>.tar.gz` and the checksums file from the [releases page](https://github.com/ergochat/ergo/releases), verify the checksum, then in a scratch directory copy `default.yaml` to `ircd.yaml`, change the listeners to `127.0.0.1:16667` and `127.0.0.1:16697`, and run `ergo mkcerts --conf ircd.yaml && ergo run --conf ircd.yaml`.

The suite registers its own accounts and uses unique nicks and channels, so it can run repeatedly against the same server. Ergo's default limit of 32 connections per IP per 10 minutes is ample.

## Before you push

```sh
mise run gate   # or ./scripts/gate.sh
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
