#!/usr/bin/env bash
# Tests for dev/macos-app.sh on any system: a fake release archive goes in, Senzor-<target>.zip
# must come out with a correct bundle. Signing is skipped off macOS. Run: dev/macos-app.test.sh
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
failures=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; failures=$((failures + 1)); }
check() {
    local name=$1 condition=$2
    if eval "$condition"; then ok "$name"; else bad "$name"; fi
}

# A release archive as dist makes it: senzor-<target>/ with the binary and the text files.
target=aarch64-apple-darwin
mkdir -p "$work/in/senzor-$target"
printf '#!/bin/sh\necho senzor\n' > "$work/in/senzor-$target/senzor"
chmod +x "$work/in/senzor-$target/senzor"
for f in LICENSE NOTICE README.md CHANGELOG.md; do echo "$f" > "$work/in/senzor-$target/$f"; done
tar -C "$work/in" -cJf "$work/senzor-$target.tar.xz" "senzor-$target"

out="$work/out"
if "$here/macos-app.sh" "$work/senzor-$target.tar.xz" "$out" > "$work/log" 2>&1; then
    ok "it builds"
else
    bad "it builds: $(cat "$work/log")"
fi

zip="$out/Senzor-$target.zip"
check "the zip is named after the target" "[[ -f '$zip' ]]"
check "a checksum like dist's" "grep -qE '^[0-9a-f]{64} \*Senzor-$target.zip$' '$zip.sha256'"
check "the checksum is right" "(cd '$out' && sha256sum -c --quiet 'Senzor-$target.zip.sha256')"

mkdir "$work/unzipped"
unzip -q "$zip" -d "$work/unzipped"
app="$work/unzipped/Senzor.app/Contents"
check "the binary, executable" "[[ -x '$app/MacOS/senzor' ]]"
check "the icon" "cmp -s '$app/Resources/senzor.icns' '$here/../assets/icon/senzor.icns'"
check "LICENSE and NOTICE" "[[ -f '$app/Resources/LICENSE' && -f '$app/Resources/NOTICE' ]]"

plist="$app/Info.plist"
value() {
    local key=$1
    python3 -I -c "import plistlib,sys; print(plistlib.load(open(sys.argv[1],'rb'))[sys.argv[2]])" "$plist" "$key"
}
version=$(awk '/^\[workspace\.package\]/{i=1;next} /^\[/{i=0} i&&/^version *=/{gsub(/.*= *"|".*/,"");print;exit}' \
    "$here/../Cargo.toml")
check "the bundle identifier is the app ID" "[[ \$(value CFBundleIdentifier) == cz.kankaj.Senzor ]]"
check "the executable" "[[ \$(value CFBundleExecutable) == senzor ]]"
check "the version from Cargo.toml" "[[ \$(value CFBundleShortVersionString) == '$version' ]]"
check "the icon file" "[[ \$(value CFBundleIconFile) == senzor ]]"
check "the name" "[[ \$(value CFBundleName) == Senzor ]]"

if [[ $failures -gt 0 ]]; then
    echo "$failures failed"
    exit 1
fi
echo "all passed"
