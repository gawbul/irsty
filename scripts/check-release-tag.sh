#!/usr/bin/env bash
# Checks that a workflow_run publish job is for a real release, and fails
# closed otherwise: TAG must be a release version (vX.Y.Z, no prereleases),
# point at HEAD_SHA, be on main, and match the version in HEAD_SHA's
# Cargo.toml. Used by .github/workflows/publish-crates.yml and
# publish-scoop.yml. They run it from a checkout of main with full history,
# never from the commit being checked, which could carry its own copy; it
# reads that commit only through git.
set -euo pipefail

: "${TAG:?TAG is the release tag}"
: "${HEAD_SHA:?HEAD_SHA is the commit the release was built from}"

fail() { echo "::error::$*"; exit 1; }

[[ "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "not a release tag: $TAG"
[[ "$(git rev-parse "refs/tags/$TAG^{commit}")" == "$HEAD_SHA" ]] || fail "$TAG does not point at $HEAD_SHA"
# In full, so a tag named origin/main can't shadow the branch.
git merge-base --is-ancestor "$HEAD_SHA" refs/remotes/origin/main || fail "$HEAD_SHA is not on main"
version=$(git show "$HEAD_SHA:Cargo.toml" | sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"$/\1/p')
[[ "$version" == "${TAG#v}" ]] || fail "Cargo.toml version $version does not match $TAG"
echo "$TAG is release $version at $HEAD_SHA, on main"
