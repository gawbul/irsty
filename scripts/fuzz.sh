#!/usr/bin/env bash
# Runs every fuzz target for the given number of seconds (default 600) in parallel.
# Needs nightly: rustup toolchain install nightly.
set -euo pipefail
cd "$(dirname "$0")/../fuzz"
seconds=${1:-600}
targets=$(cargo +nightly fuzz list)
for t in $targets; do
    mkdir -p "corpus/$t"
    cargo +nightly fuzz run -O "$t" -- -max_total_time="$seconds" -rss_limit_mb=2048 >"fuzz-$t.log" 2>&1 &
done
fail=0
for job in $(jobs -p); do wait "$job" || fail=1; done
for t in $targets; do
    printf '%-16s %s\n' "$t" "$(grep -Eo '#[0-9]+ +DONE.*' "fuzz-$t.log" | head -1 || echo 'see log')"
done
exit $fail
