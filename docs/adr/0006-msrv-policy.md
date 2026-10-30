# 0006: MSRV trails stable by two releases

- Status: accepted
- Date: 2026-10-30

## Context

Irsty is an application, so most users will install a binary. Distribution packagers (Debian, Fedora, Nix) build with whatever rustc they ship, which often lags stable by a few releases.

## Decision

- Develop on the latest stable, pinned exactly in `rust-toolchain.toml`.
- Set `rust-version` to stable minus two (currently 1.97, with stable at 1.99.1).
- CI checks the MSRV. `clippy::incompatible_msrv` flags newer standard library APIs.
- Bump the MSRV deliberately, in its own commit, with a CHANGELOG entry. Raising it is not a breaking change for us.

## Consequences

- A newer language feature has to wait about three months.
- Dependencies with a higher MSRV can't be adopted until our MSRV catches up, or until we decide to bump it.
