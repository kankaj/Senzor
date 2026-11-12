#!/usr/bin/env bash
# Tests for dev/homebrew-cask.sh: the cask it writes names the release's two Mac apps with their
# checksums, and is valid Ruby. Run: dev/homebrew-cask.test.sh
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

arm=$(printf 'a%.0s' {1..64})
intel=$(printf 'b%.0s' {1..64})
cask="$work/senzor.rb"
if "$here/homebrew-cask.sh" 1.2.3 "$arm" "$intel" > "$cask" 2> "$work/err"; then
    ok "it writes a cask"
else
    bad "it writes a cask: $(cat "$work/err")"
fi

check "the cask is senzor" "grep -q '^cask \"senzor\" do\$' '$cask'"
check "the version" "grep -q '^  version \"1.2.3\"\$' '$cask'"
check "a checksum per Mac" "grep -q 'sha256 arm:   \"$arm\",' '$cask' && grep -q 'intel: \"$intel\"' '$cask'"
check "the release's zips" "grep -qF 'url \"https://github.com/kankaj/Senzor/releases/download/v#{version}/Senzor-#{arch}-apple-darwin.zip\"' '$cask'"
check "no verified: on the url (Homebrew deprecated it)" "! grep -q 'verified:' '$cask'"
check "no depends_on macos (Homebrew refuses a minimum as old as ours)" "! grep -q 'depends_on' '$cask'"
check "the arch names of the zips" "grep -q 'arch arm: \"aarch64\", intel: \"x86_64\"' '$cask'"
check "the app" "grep -q '^  app \"Senzor.app\"\$' '$cask'"
check "how to open an unsigned app" "grep -q 'Open Anyway' '$cask'"
check "the settings are zapped" "grep -q 'Application Support/cz.kankaj.Senzor' '$cask'"
if command -v ruby > /dev/null; then
    check "valid Ruby" "ruby -c '$cask' > /dev/null"
fi

if "$here/homebrew-cask.sh" 1.2.3 short "$intel" > /dev/null 2>&1; then
    bad "a checksum that is not SHA-256 is refused"
else
    ok "a checksum that is not SHA-256 is refused"
fi
if "$here/homebrew-cask.sh" 1.2 "$arm" "$intel" > /dev/null 2>&1; then
    bad "a version that is not X.Y.Z is refused"
else
    ok "a version that is not X.Y.Z is refused"
fi

if [[ $failures -gt 0 ]]; then
    echo "$failures failed"
    exit 1
fi
echo "all passed"
