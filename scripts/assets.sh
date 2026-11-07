#!/usr/bin/env bash
# Regenerates the man page and shell completions in assets/ from the
# command-line definition (crates/irsty/src/assets.rs). A test fails if they
# are out of date, so run this after changing a flag or the man page text.
set -euo pipefail
cd "$(dirname "$0")/.."
IRSTY_UPDATE_ASSETS=1 cargo test --quiet -p irsty assets_are_up_to_date
git status --short assets
