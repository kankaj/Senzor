#!/usr/bin/env bash
# Copies into each crate the files it needs from outside its folder, because crates.io only takes
# a crate's own folder: LICENSE and NOTICE for every published crate, the MCP reference pages
# for senzor-mcp, and the examples, CHANGELOG.md and icons for senzor. The copies are committed;
# the code reads them, never the originals. Edit the originals, then run this script; --check
# (CI, dev/release-check.sh) fails when a copy differs from its original.
#
# Usage: dev/crate-files.sh [--root DIR] [--check]
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
check=no
while [[ $# -gt 0 ]]; do
    case $1 in
        --root) root=$2; shift 2 ;;
        --check) check=yes; shift ;;
        -h|--help) sed -n '2,8p' "$0"; exit 0 ;;
        *) echo "crate-files: unknown argument $1" >&2; exit 2 ;;
    esac
done

# Copy, then original, both from the repository root.
pairs=()
for crate in senzor senzor-cli senzor-core senzor-mcp; do
    pairs+=("crates/$crate/LICENSE" LICENSE "crates/$crate/NOTICE" NOTICE)
done
pairs+=(
    crates/senzor-mcp/docs/project-file.mdx website/content/docs/reference/project-file.mdx
    crates/senzor-mcp/docs/generators.mdx website/content/docs/reference/generators.mdx
    crates/senzor-mcp/docs/template-variables.mdx website/content/docs/reference/template-variables.mdx
    crates/senzor-mcp/docs/scripts.mdx website/content/docs/concepts/scripts.mdx
    crates/senzor/CHANGELOG.md CHANGELOG.md
    crates/senzor/bundled/icon/senzor-256.png assets/icon/senzor-256.png
    crates/senzor/bundled/icon/senzor-macos-1024.png assets/icon/senzor-macos-1024.png
    crates/senzor/bundled/icon/senzor.ico assets/icon/senzor.ico
    crates/senzor/bundled/examples/proto/sensor.proto examples/proto/sensor.proto
)
for example in quick-start ikea-zigbee2mqtt home-assistant-statestream home-assistant-discovery \
    thermostat binary-payloads fleet showcase; do
    pairs+=("crates/senzor/bundled/examples/$example.toml" "examples/$example.toml")
done

stale=()
for ((i = 0; i < ${#pairs[@]}; i += 2)); do
    copy=${pairs[i]}
    original=${pairs[i + 1]}
    if [[ $check == yes ]]; then
        cmp -s "$root/$original" "$root/$copy" || stale+=("$copy (from $original)")
    else
        mkdir -p "$(dirname "$root/$copy")"
        cp "$root/$original" "$root/$copy"
    fi
done

if ((${#stale[@]})); then
    echo "crate-files: these copies differ from their originals; run dev/crate-files.sh:" >&2
    printf '  %s\n' "${stale[@]}" >&2
    exit 1
fi
