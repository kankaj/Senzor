#!/usr/bin/env bash
# Tests for dev/crate-files.sh in a copy of the repository's tracked files: it copies every file a
# crate needs from outside its folder into the crate, and --check fails, naming the copy, when a
# copy differs from its source. Run: dev/crate-files.test.sh
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
failures=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; failures=$((failures + 1)); }
check() {
    local name=$1 condition=$2
    if eval "$condition"; then ok "$name"; else bad "$name"; fi
}

# The tracked files as they are in the working tree, with this script.
git -C "$root" ls-files -z --cached --others --exclude-standard \
    | (cd "$root" && xargs -0 cp --parents -t "$work" 2> /dev/null || true)
cp "$here/crate-files.sh" "$work/dev/"
files() { "$work/dev/crate-files.sh" --root "$work" "$@"; }

files > "$work/out" 2>&1 && ok "it copies the files" || bad "it copies the files: $(cat "$work/out")"
check "LICENSE and NOTICE in every published crate" \
    "for c in senzor senzor-cli senzor-core senzor-mcp; do cmp -s '$work/LICENSE' '$work/crates/'\$c/LICENSE && cmp -s '$work/NOTICE' '$work/crates/'\$c/NOTICE || exit 1; done"
check "the MCP reference pages in senzor-mcp" \
    "cmp -s '$work/website/content/docs/reference/generators.mdx' '$work/crates/senzor-mcp/docs/generators.mdx' && cmp -s '$work/website/content/docs/concepts/scripts.mdx' '$work/crates/senzor-mcp/docs/scripts.mdx'"
check "the examples, the changelog and the icons in senzor" \
    "cmp -s '$work/examples/fleet.toml' '$work/crates/senzor/bundled/examples/fleet.toml' && cmp -s '$work/examples/proto/sensor.proto' '$work/crates/senzor/bundled/examples/proto/sensor.proto' && cmp -s '$work/CHANGELOG.md' '$work/crates/senzor/CHANGELOG.md' && cmp -s '$work/assets/icon/senzor.ico' '$work/crates/senzor/bundled/icon/senzor.ico'"
check "--check passes when every copy matches" "files --check > /dev/null 2>&1"

echo "- A change." >> "$work/CHANGELOG.md"
files --check > "$work/out" 2>&1 && bad "--check fails after a source changed" || ok "--check fails after a source changed"
check "and names the copy and the command that fixes it" \
    "grep -q 'crates/senzor/CHANGELOG.md' '$work/out' && grep -q 'dev/crate-files.sh' '$work/out'"
rm "$work/crates/senzor-mcp/docs/generators.mdx"
check "--check fails when a copy is missing" "! files --check > /dev/null 2>&1"
files > /dev/null
check "copying again brings them back in step" "files --check > /dev/null 2>&1"

if ((failures)); then
    echo "$failures failed"
    exit 1
fi
echo "all passed"
