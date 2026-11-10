#!/usr/bin/env bash
# Prepares a release: sets the version, dates its CHANGELOG.md section, checks everything, and
# makes the release commit and the annotated tag. It asks before each step and never pushes: the
# push it prints starts the release workflow (.github/workflows/release.yml).
#
# Usage: dev/release.sh X.Y.Z        (or X.Y.Z-rc.N for a pre-release)
#
# Before running it: CHANGELOG.md has a "## [X.Y.Z] - Unreleased" section with Highlights, and
# crates/senzor/locales/cs/whats-new.md has their Czech version under "## [X.Y.Z]".
set -euo pipefail

cd "$(dirname "$0")/.."

fail() { echo "release: $*" >&2; exit 1; }
ask() {
    local answer
    read -r -p "$1 [y/N] " answer || true
    [[ $answer == y || $answer == Y || $answer == yes ]] || fail "stopped; nothing after this step was done"
}

version=${1:-}
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] \
    || fail "give the version as X.Y.Z or X.Y.Z-rc.N, not \"$version\""
tag="v$version"
base=${version%%-*}

[[ $(git branch --show-current) == main ]] || fail "release from main, not from $(git branch --show-current)"
[[ -z $(git status --porcelain) ]] || fail "the working tree has uncommitted changes"
git rev-parse -q --verify "refs/tags/$tag" > /dev/null && fail "tag $tag exists already"

if [[ $version == "$base" ]]; then
    ask "Set the version to $version in Cargo.toml and date the $base section of CHANGELOG.md?"
else
    ask "Set the version to $version in Cargo.toml (a pre-release: the $base section stays undated)?"
fi
# Only the version of [workspace.package]: the first `version =` line after that header.
awk -v v="$version" '
    /^\[workspace\.package\]/ { inside = 1 }
    /^\[/ && !/^\[workspace\.package\]/ { inside = 0 }
    inside && !done && /^version *=/ { print "version = \"" v "\""; done = 1; next }
    { print }
' Cargo.toml > Cargo.toml.new && mv Cargo.toml.new Cargo.toml
if [[ $version == "$base" ]]; then
    today=$(date +%F)
    sed -i.bak "s/^## \[${base//./\\.}\] - Unreleased$/## [$base] - $today/" CHANGELOG.md && rm CHANGELOG.md.bak
fi
if [[ -z ${RELEASE_SKIP_CARGO:-} ]]; then
    cargo update --workspace --offline
fi

dev/release-check.sh "$tag"

if [[ -z ${RELEASE_SKIP_CARGO:-} ]]; then
    ask "Run the tests and dist plan?"
    cargo test --workspace
    mise exec -- dist plan --tag="$tag" > /dev/null
fi

git diff --stat
ask "Commit \"release: $version\" and tag $tag?"
git add -A
git commit -q -m "release: $version"
# Unsigned, like the commits of this repository (CLAUDE.md, "Commits").
git -c tag.gpgSign=false tag -a "$tag" -m "Senzor $version"

echo
echo "release: $tag is ready. Pushing it starts the release workflow:"
echo
echo "    git push origin main $tag"
