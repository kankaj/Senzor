#!/usr/bin/env bash
# Builds Senzor.app from the window's macOS release archive (senzor-<target>.tar.xz, as dist makes
# it) and zips it as Senzor-<target>.zip, with a checksum file like dist's. On macOS the app gets
# an ad-hoc signature (no Apple account involved): Apple Silicon runs only signed code, and an app
# whose bundle is not signed as a whole looks damaged to Gatekeeper. Elsewhere, for the tests,
# signing is skipped and plain zip is used. The release workflow runs it (macos-app.yml).
#
# Usage: dev/macos-app.sh ARCHIVE OUT_DIR
set -euo pipefail

archive=$1
out=$(mkdir -p "$2" && cd "$2" && pwd)
root=$(cd "$(dirname "$0")/.." && pwd)

name=$(basename "$archive" .tar.xz)            # senzor-aarch64-apple-darwin
target=${name#senzor-}
case $target in
    aarch64-apple-darwin) minimum=11.0 ;;
    x86_64-apple-darwin) minimum=10.13 ;;
    *) echo "macos-app: not a macOS archive: $archive" >&2; exit 1 ;;
esac
version=$(awk '/^\[workspace\.package\]/{i=1;next} /^\[/{i=0} i&&/^version *=/{gsub(/.*= *"|".*/,"");print;exit}' \
    "$root/Cargo.toml")

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
tar -C "$work" -xJf "$archive"
from="$work/$name"

app="$work/app/Senzor.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$from/senzor" "$app/Contents/MacOS/senzor"
cp "$root/assets/icon/senzor.icns" "$app/Contents/Resources/senzor.icns"
cp "$from/LICENSE" "$from/NOTICE" "$app/Contents/Resources/"
cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>cz.kankaj.Senzor</string>
    <key>CFBundleName</key>
    <string>Senzor</string>
    <key>CFBundleDisplayName</key>
    <string>Senzor</string>
    <key>CFBundleExecutable</key>
    <string>senzor</string>
    <key>CFBundleIconFile</key>
    <string>senzor</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>$version</string>
    <key>CFBundleVersion</key>
    <string>$version</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>LSMinimumSystemVersion</key>
    <string>$minimum</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.developer-tools</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSHumanReadableCopyright</key>
    <string>© Jaroslav Kaňka, Apache-2.0</string>
</dict>
</plist>
EOF

zip="$out/Senzor-$target.zip"
rm -f "$zip"
if [[ $(uname) == Darwin ]]; then
    codesign --force --deep --sign - "$app"
    codesign --verify --deep --strict "$app"
    # ditto keeps the bundle's metadata and signature as Finder's Compress does.
    ditto -c -k --sequesterRsrc --keepParent "$app" "$zip"
else
    (cd "$work/app" && zip -qry "$zip" Senzor.app)
fi
(cd "$out" && shasum -a 256 -b "Senzor-$target.zip" 2>/dev/null || sha256sum -b "Senzor-$target.zip") \
    > "$zip.sha256"
echo "macos-app: $zip"
