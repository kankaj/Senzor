#!/usr/bin/env bash
# Lays out the window's MSIX package for the Microsoft Store from its unzipped Windows release
# archive (senzor-<target>.zip, as dist makes it): senzor.exe, LICENSE and NOTICE, the manifest
# from crates/senzor/msix/AppxManifest.xml with this version and architecture, and the logos from
# assets/msix/. The release workflow (msix.yml) then indexes the logos with makepri, using
# crates/senzor/msix/priconfig.xml, and packs and bundles the folders with makeappx, on Windows.
#
# The version is the workspace's from Cargo.toml (SENZOR_VERSION overrides it, for the tests) as
# X.Y.Z.0: the Store keeps the fourth part for itself, and a pre-release has no such form.
#
# Usage: dev/msix-layout.sh UNZIPPED_ARCHIVE x64|arm64 OUT_DIR
set -euo pipefail

from=$1
arch=$2
out=$3
root=$(cd "$(dirname "$0")/.." && pwd)

case $arch in
    x64 | arm64) ;;
    *) echo "msix-layout: the architecture is x64 or arm64, not $arch" >&2; exit 1 ;;
esac
version=${SENZOR_VERSION:-$(awk '/^\[workspace\.package\]/{i=1;next} /^\[/{i=0} i&&/^version *=/{gsub(/.*= *"|".*/,"");print;exit}' \
    "$root/Cargo.toml")}
if [[ ! $version =~ ^[1-9][0-9]*\.[0-9]+\.[0-9]+$ ]]; then
    echo "msix-layout: the Store needs a version of three numbers, not $version" >&2
    exit 1
fi

mkdir -p "$out/Assets"
cp "$from/senzor.exe" "$from/LICENSE" "$from/NOTICE" "$out/"
cp "$root"/assets/msix/*.png "$out/Assets/"
sed -e "s/@VERSION@/$version.0/" -e "s/@ARCH@/$arch/" \
    "$root/crates/senzor/msix/AppxManifest.xml" > "$out/AppxManifest.xml"
