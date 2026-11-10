#!/usr/bin/env bash
# Builds the docs of this checkout's version for its folder /<minor>/ and packs them as
# senzor-docs-<minor>.tar.gz (one folder, <minor>/) with a .sha256, into OUT_DIR. The release
# workflow attaches them to the GitHub Release (docs-archive.yml); the production build unpacks
# every version's archive (scripts/versions.mjs).
#
# Usage, in website/: scripts/docs-archive.sh OUT_DIR
set -euo pipefail

out=$(mkdir -p "$1" && cd "$1" && pwd)
version=$(awk '/^\[workspace\.package\]/{i=1;next} /^\[/{i=0} i&&/^version *=/{gsub(/.*= *"|".*/,"");print;exit}' \
    ../Cargo.toml)
minor=$(cut -d. -f1,2 <<< "${version%%-*}")

DOCS_BASE_PATH="/$minor" pnpm build
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
cp -r out "$stage/$minor"
name="senzor-docs-$minor.tar.gz"
tar -czf "$out/$name" -C "$stage" "$minor"
(cd "$out" && sha256sum -b "$name" > "$name.sha256")
echo "docs-archive: $out/$name ($version)"
