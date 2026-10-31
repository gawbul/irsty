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
| `soju`, `soju:install`, `run:soju`, `test:soju` | A local soju bouncer in front of Ergo, Irsty connected to it, and its integration tests (see below). |
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

## Integration tests against soju

`crates/irsty-core/tests/soju.rs` checks Irsty against a real [soju](https://soju.im) bouncer: its networks open as networks of their own and follow the bouncer as networks are added and deleted, and a message sent while Irsty is away comes back from soju's history on the next connection. It is skipped unless `IRSTY_SOJU_PORT` is set, and doesn't run in CI yet.

soju ships source only (signed tarballs, no binaries or checksums), so build it once with Go:

```sh
mise run soju:install   # go install ...soju@v0.11.1 and sojuctl; Go checks every module against its checksum database
```

That puts `soju` and `sojuctl` in `$(go env GOPATH)/bin` (usually `~/go/bin`); `scripts/soju.sh` looks on your `PATH` and there. Then, each in its own terminal:

```sh
mise run ergo        # soju's upstream network
mise run soju        # soju on 127.0.0.1:16670; add --fresh to start over
mise run test:soju   # the tests
mise run run:soju    # or Irsty itself, connected to it
```

The first run creates the user `irsty` (password `irsty-test`) with two networks on the local Ergo, `ergo` and `ergo2`; later runs keep soju's database in `target/soju/run`. A test that fails part-way can leave a `tmp…` network behind on the bouncer; `--fresh` clears it. Repeated runs open many connections to Ergo, whose per-IP throttle then slows them down; restarting Ergo with `--fresh` resets it.

## Before you push

```sh
mise run gate   # or ./scripts/gate.sh
```

This checks that no iCloud sync duplicates are tracked, then runs fmt, clippy (warnings denied), nextest, doctests, coverage of `irsty-proto` (at least 90% of lines), cargo-deny, cargo-audit and an MSRV check. CI runs the same checks, with tests on Linux and macOS, plus the Ergo integration tests and a short fuzz run.

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
