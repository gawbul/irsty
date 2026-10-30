#!/usr/bin/env bash
# Runs the full quality gate. Every milestone must pass this before it is done.
set -euo pipefail
cd "$(dirname "$0")/.."

msrv=$(sed -n 's/^rust-version = "\(.*\)"$/\1/p' Cargo.toml)

step() { printf '\n==> %s\n' "$*"; }

step "fmt";      cargo fmt --all --check
step "clippy";   cargo clippy --workspace --all-targets --all-features -- -D warnings
step "nextest";  cargo nextest run --workspace --all-features --no-tests=pass
step "doctests"; cargo test --workspace --all-features --doc
step "coverage (irsty-proto >= 90% lines)"
cargo llvm-cov -p irsty-proto --fail-under-lines 90 --summary-only 2>&1 | tail -3
step "deny";     cargo deny check
step "audit";    cargo audit --deny warnings
step "msrv ($msrv)"
if rustup toolchain list | grep -q "^$msrv"; then
    cargo "+$msrv" check --workspace --all-targets --all-features
else
    echo "toolchain $msrv not installed; run: rustup toolchain install $msrv --profile minimal"
    exit 1
fi
printf '\nAll gates passed.\n'
