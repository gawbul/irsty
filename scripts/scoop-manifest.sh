#!/usr/bin/env bash
# Prints the Scoop manifest (irsty.json) for a release: the Windows archive
# that dist builds, and its SHA-256. Usage: scoop-manifest.sh VERSION SHA256.
# .github/workflows/publish-scoop.yml pushes it to gawbul/scoop-bucket.
set -euo pipefail

version=${1:?version, such as 0.1.0}
hash=${2:?the SHA-256 of the archive}
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "bad version: $version" >&2; exit 1; }
[[ "$hash" =~ ^[0-9a-f]{64}$ ]] || { echo "bad SHA-256: $hash" >&2; exit 1; }

jq -n --arg version "$version" --arg hash "$hash" '{
  version: $version,
  description: "Modern, keyboard-driven terminal IRC client",
  homepage: "https://gawbul.github.io/irsty/",
  license: "Apache-2.0",
  architecture: {
    "64bit": {
      url: "https://github.com/gawbul/irsty/releases/download/v\($version)/irsty-x86_64-pc-windows-msvc.zip",
      hash: $hash
    }
  },
  bin: "irsty.exe",
  notes: [
    "The man page and completions are in $dir.",
    "For PowerShell completions, add this to your $PROFILE:",
    "  irsty --completions powershell | Out-String | Invoke-Expression"
  ]
}'
