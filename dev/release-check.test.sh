#!/usr/bin/env bash
# Tests for dev/release-check.sh: each case builds a small project in a temporary folder and
# checks the script's exit status and message. Run: dev/release-check.test.sh
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
check="$here/release-check.sh"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
failures=0

# project DIR VERSION CHANGELOG_HEADING CZECH_HEADING [HIGHLIGHTS] [DOCS_RELEASE]
project() {
    local dir=$1 version=$2 heading=$3 czech=$4 highlights=${5:-yes} docs=${6:-$2}
    mkdir -p "$dir/crates/senzor/locales/cs" "$dir/website/public"
    printf '{\n  "versions": [{ "minor": "%s", "release": "%s" }]\n}\n' "$(cut -d. -f1,2 <<< "$docs")" "$docs" \
        > "$dir/website/public/versions.json"
    printf '[workspace]\nmembers = ["crates/*"]\n\n[workspace.package]\nversion = "%s"\nedition = "2024"\n' \
        "$version" > "$dir/Cargo.toml"
    {
        printf '# Changelog\n\n%s\n\nThe release.\n\n' "$heading"
        [[ $highlights == yes ]] && printf '### Highlights\n\n- One.\n\n'
        printf '### Added\n\n- Things.\n\n## [0.9.0] - 2026-01-01\n\n### Highlights\n\n- Old.\n'
    } > "$dir/CHANGELOG.md"
    printf '# Co je nového\n\n%s\n\n### Highlights\n\n- Jedna.\n' "$czech" \
        > "$dir/crates/senzor/locales/cs/whats-new.md"
}

# expect NAME STATUS MESSAGE_PART -- ARGS...
expect() {
    local name=$1 status=$2 message=$3
    shift 4
    local output actual=0
    output=$("$check" "$@" 2>&1) || actual=$?
    if [[ $actual -ne $status || $output != *"$message"* ]]; then
        echo "FAIL $name: exit $actual (want $status), output: $output"
        failures=$((failures + 1))
    else
        echo "ok   $name"
    fi
}

project "$work/good" 1.0.0 '## [1.0.0] - 2026-11-20' '## [1.0.0]'
expect "a dated release passes" 0 "1.0.0 is ready" -- --root "$work/good" v1.0.0
expect "the tag must match Cargo.toml" 1 'tag v1.0.1 does not match version 1.0.0' -- --root "$work/good" v1.0.1
expect "a tag needs its v" 1 'tag 1.0.0 does not match version 1.0.0' -- --root "$work/good" 1.0.0
expect "without a tag, the version is checked" 0 "1.0.0 is ready" -- --root "$work/good"

project "$work/unreleased" 1.0.0 '## [1.0.0] - Unreleased' '## [1.0.0]'
expect "a tag needs a dated section" 1 'CHANGELOG.md: the 1.0.0 section has no date' -- --root "$work/unreleased" v1.0.0
expect "without a tag, Unreleased is fine" 0 "1.0.0 is ready" -- --root "$work/unreleased"

project "$work/no-section" 1.0.0 '## [1.1.0] - 2026-11-20' '## [1.0.0]'
expect "the version needs a CHANGELOG section" 1 'CHANGELOG.md has no section for 1.0.0' -- --root "$work/no-section" v1.0.0

project "$work/no-highlights" 1.0.0 '## [1.0.0] - 2026-11-20' '## [1.0.0]' no
expect "the section needs Highlights" 1 'the 1.0.0 section has no Highlights' -- --root "$work/no-highlights" v1.0.0

project "$work/no-czech" 1.0.0 '## [1.0.0] - 2026-11-20' '## [0.9.0]'
expect "the Czech highlights are needed" 1 'whats-new.md has no section for 1.0.0' -- --root "$work/no-czech" v1.0.0

project "$work/old-docs" 1.0.1 '## [1.0.1] - 2026-11-20' '## [1.0.1]' yes 1.0.0
expect "a tag needs its docs in the versions list" 1 'website/public/versions.json does not list 1.0.1' -- --root "$work/old-docs" v1.0.1
expect "without a tag, the list is not checked" 0 "1.0.1 is ready" -- --root "$work/old-docs"

project "$work/rc" 1.1.0-rc.1 '## [1.1.0] - Unreleased' '## [1.1.0]'
expect "a pre-release may be undated" 0 "1.1.0-rc.1 is ready" -- --root "$work/rc" v1.1.0-rc.1
expect "a pre-release tag must match too" 1 'tag v1.1.0-rc.2 does not match version 1.1.0-rc.1' -- --root "$work/rc" v1.1.0-rc.2

if [[ $failures -gt 0 ]]; then
    echo "$failures failed"
    exit 1
fi
echo "all passed"
