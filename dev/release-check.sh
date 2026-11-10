#!/usr/bin/env bash
# Checks that a release can be made: the version in Cargo.toml has a CHANGELOG.md section with
# Highlights, and their Czech version in crates/senzor/locales/cs/whats-new.md. With a tag
# (vX.Y.Z, as the release workflow passes it), the tag must name that version and the section must
# be dated; a pre-release (vX.Y.Z-rc.1) uses the X.Y.Z section, dated or not.
#
# Usage: dev/release-check.sh [--root DIR] [TAG]
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
tag=""
while [[ $# -gt 0 ]]; do
    case $1 in
        --root) root=$2; shift 2 ;;
        -h|--help) sed -n '2,7p' "$0"; exit 0 ;;
        *) tag=$1; shift ;;
    esac
done

fail() { echo "release check: $*" >&2; exit 1; }

# The first `version = "…"` after [workspace.package].
version=$(awk '
    /^\[workspace\.package\]/ { inside = 1; next }
    /^\[/ { inside = 0 }
    inside && /^version *=/ { gsub(/.*= *"|".*/, ""); print; exit }
' "$root/Cargo.toml")
[[ -n $version ]] || fail "Cargo.toml has no [workspace.package] version"

if [[ -n $tag && $tag != "v$version" ]]; then
    fail "tag $tag does not match version $version in Cargo.toml"
fi

# A pre-release is described by the section of the version it leads to.
base=${version%%-*}
prerelease=$([[ $version == *-* ]] && echo yes || echo no)

heading=$(grep -E "^## \[${base//./\\.}\]" "$root/CHANGELOG.md" | head -n 1 || true)
[[ -n $heading ]] || fail "CHANGELOG.md has no section for $base"
if [[ -n $tag && $prerelease == no && ! $heading =~ \ -\ [0-9]{4}-[0-9]{2}-[0-9]{2}$ ]]; then
    fail "CHANGELOG.md: the $base section has no date (\"## [$base] - YYYY-MM-DD\"): $heading"
fi

# The lines of that section, up to the next "## [".
section=$(awk -v h="$heading" '
    $0 == h { inside = 1; next }
    inside && /^## \[/ { exit }
    inside { print }
' "$root/CHANGELOG.md")
grep -q '^### Highlights' <<< "$section" || fail "CHANGELOG.md: the $base section has no Highlights"

czech="$root/crates/senzor/locales/cs/whats-new.md"
grep -qE "^## \[${base//./\\.}\]" "$czech" \
    || fail "crates/senzor/locales/cs/whats-new.md has no section for $base (the Czech Highlights)"

echo "release check: $version is ready${tag:+ to release as $tag}"
