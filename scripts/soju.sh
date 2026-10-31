#!/usr/bin/env bash
# Runs a local soju bouncer for integration tests and manual testing, on
# 127.0.0.1:16670 (plain text), in front of the local Ergo (mise run ergo).
# Creates the user "irsty" (password "irsty-test") with two networks on Ergo,
# "ergo" and "ergo2", the first time. Pass --fresh to start from scratch.
#
# soju ships source only. Build it with mise run soju:install, which runs
# go install and so checks every module against Go's checksum database.
set -euo pipefail
cd "$(dirname "$0")/.."

find_tool() {
    local name=$1
    if command -v "$name" >/dev/null; then
        command -v "$name"
        return
    fi
    if command -v go >/dev/null; then
        local dir
        dir=$(go env GOBIN)
        [[ -n $dir ]] || dir="$(go env GOPATH)/bin"
        if [[ -x $dir/$name ]]; then
            echo "$dir/$name"
            return
        fi
    fi
    echo "cannot find $name; build it with: mise run soju:install" >&2
    exit 1
}
soju=$(find_tool soju)
sojuctl=$(find_tool sojuctl)

if ! (exec 3<>/dev/tcp/127.0.0.1/16667) 2>/dev/null; then
    echo "soju needs the local Ergo on 127.0.0.1:16667 as its upstream: start mise run ergo first" >&2
    exit 1
fi

run="$PWD/target/soju/run"
if [[ ${1:-} == --fresh ]]; then
    rm -rf "$run"
fi
mkdir -p "$run"
config=$run/config
cat >"$config" <<EOF
listen irc+insecure://127.0.0.1:16670
listen unix+admin://$run/admin.sock
hostname soju.test
title "Irsty test bouncer"
db sqlite3 $run/soju.db
message-store db
EOF

"$soju" -config "$config" &
pid=$!
trap 'kill $pid 2>/dev/null; wait $pid 2>/dev/null' EXIT INT TERM

for _ in $(seq 1 50); do
    [[ -S $run/admin.sock ]] && break
    sleep 0.1
done
ctl() { "$sojuctl" -config "$config" "$@"; }
# The first time only: a user with two networks on Ergo.
if [[ ! -f $run/.seeded ]]; then
    ctl user create -username irsty -password irsty-test
    ctl user run irsty network create -addr irc+insecure://127.0.0.1:16667 -name ergo -nick sojutest
    ctl user run irsty network create -addr irc+insecure://127.0.0.1:16667 -name ergo2 -nick sojutest2
    touch "$run/.seeded"
fi

echo "soju on 127.0.0.1:16670 (plain text): user irsty, password irsty-test, networks ergo and ergo2; Ctrl+C to stop"
wait $pid
