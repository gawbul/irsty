#!/usr/bin/env bash
# Runs a local Ergo server for integration tests and manual testing, listening
# on 127.0.0.1:16667 (plain) and 127.0.0.1:16697 (TLS, self-signed).
# Downloads the release once into target/ergo, verifies its checksum and keeps
# its database between runs. Pass --fresh to start from an empty database.
set -euo pipefail
cd "$(dirname "$0")/.."

version=${ERGO_VERSION:-2.19.1}
root=target/ergo
run=$root/run

case "$(uname -s)" in
    Darwin) os=macos ;;
    Linux) os=linux ;;
    *) echo "unsupported OS: $(uname -s); use the container (see CONTRIBUTING.md)" >&2; exit 1 ;;
esac
case "$(uname -m)" in
    arm64 | aarch64) arch=arm64 ;;
    x86_64 | amd64) arch=x86_64 ;;
    *) echo "unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac

name=ergo-$version-$os-$arch
dir=$root/$name
if [[ ! -x $dir/ergo ]]; then
    base=https://github.com/ergochat/ergo/releases/download/v$version
    mkdir -p "$root"
    echo "downloading $name"
    curl -fsSL -o "$root/$name.tar.gz" "$base/$name.tar.gz"
    curl -fsSL -o "$root/checksums.txt" "$base/ergo-$version-checksums.txt"
    expected=$(awk -v f="$name.tar.gz" '$2 == f || $2 == "*" f { print $1 }' "$root/checksums.txt")
    if command -v sha256sum >/dev/null; then
        actual=$(sha256sum "$root/$name.tar.gz" | awk '{ print $1 }')
    else
        actual=$(shasum -a 256 "$root/$name.tar.gz" | awk '{ print $1 }')
    fi
    if [[ -z $expected || $expected != "$actual" ]]; then
        echo "checksum mismatch for $name.tar.gz" >&2
        rm -f "$root/$name.tar.gz"
        exit 1
    fi
    tar -xzf "$root/$name.tar.gz" -C "$root"
    rm "$root/$name.tar.gz"
fi

if [[ ${1:-} == --fresh ]]; then
    rm -rf "$run"
fi
if [[ ! -f $run/ircd.yaml ]]; then
    mkdir -p "$run"
    cp -R "$dir/languages" "$dir/ergo.motd" "$run/"
    sed -e 's/"127\.0\.0\.1:6667":/"127.0.0.1:16667":/' \
        -e '/"\[::1\]:6667":/d' \
        -e 's/":6697":/"127.0.0.1:16697":/' \
        "$dir/default.yaml" >"$run/ircd.yaml"
    (cd "$run" && "../$name/ergo" mkcerts --conf ircd.yaml)
fi

echo "Ergo $version on 127.0.0.1:16667 (plain) and 127.0.0.1:16697 (TLS); Ctrl+C to stop"
cd "$run"
exec "../$name/ergo" run --conf ircd.yaml
