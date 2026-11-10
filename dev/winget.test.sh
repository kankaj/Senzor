#!/usr/bin/env bash
# Tests for dev/winget.sh: both packages get their three manifests (version, installer, defaultLocale)
# for a release, with the release's Windows files and checksums. Run: dev/winget.test.sh
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

x64=$(printf 'a%.0s' {1..64})
arm=$(printf 'b%.0s' {1..64})
for id in Kankaj.Senzor Kankaj.SenzorCLI; do
    out="$work/$id"
    if "$here/winget.sh" "$id" 1.2.3 "$x64" "$arm" "$out" 2026-11-20 2> "$work/err"; then
        ok "$id: it writes the manifests"
    else
        bad "$id: it writes the manifests: $(cat "$work/err")"
        continue
    fi
    dir="$out/manifests/k/Kankaj/${id#Kankaj.}/1.2.3"
    check "$id: the three files where winget-pkgs keeps them" \
        "[[ -f '$dir/$id.yaml' && -f '$dir/$id.installer.yaml' && -f '$dir/$id.locale.en-US.yaml' ]]"
    for f in "$dir"/*.yaml; do
        check "$id: $(basename "$f"): id, version, schema 1.10.0" \
            "grep -q '^PackageIdentifier: $id\$' '$f' && grep -q '^PackageVersion: 1.2.3\$' '$f' && grep -q '^ManifestVersion: 1.10.0\$' '$f'"
        if python3 -c 'import yaml' 2> /dev/null; then
            check "$id: $(basename "$f"): valid YAML" "python3 -I -c 'import sys, yaml; yaml.safe_load(open(sys.argv[1], encoding=\"utf-8\"))' '$f'"
        fi
    done
    installer="$dir/$id.installer.yaml"
    check "$id: both architectures with upper-case checksums" \
        "grep -q 'Architecture: x64' '$installer' && grep -q 'Architecture: arm64' '$installer' && grep -q 'InstallerSha256: ${x64^^}' '$installer' && grep -q 'InstallerSha256: ${arm^^}' '$installer'"
    check "$id: the release date" "grep -q '^ReleaseDate: 2026-11-20\$' '$installer'"
    check "$id: the publisher" "grep -q '^Publisher: Jaroslav Kaňka\$' '$dir/$id.locale.en-US.yaml'"
    check "$id: the docs of its version" "grep -q 'DocumentUrl: https://senzor.kankaj.cz/1.2/docs/' '$dir/$id.locale.en-US.yaml'"
done

gui="$work/Kankaj.Senzor/manifests/k/Kankaj/Senzor/1.2.3/Kankaj.Senzor.installer.yaml"
cli="$work/Kankaj.SenzorCLI/manifests/k/Kankaj/SenzorCLI/1.2.3/Kankaj.SenzorCLI.installer.yaml"
check "the window: the MSI" "grep -q '^InstallerType: wix\$' '$gui' && grep -q 'senzor-x86_64-pc-windows-msvc.msi\$' '$gui'"
check "the window: the MSI's upgrade code and publisher" \
    "grep -q \"UpgradeCode: '{0046E879-2FB5-4DCB-98A7-564C93156A90}'\" '$gui' && grep -q 'Publisher: Jaroslav Kanka' '$gui'"
check "the CLI: the zip, as a portable command" \
    "grep -q '^InstallerType: zip\$' '$cli' && grep -q '^NestedInstallerType: portable\$' '$cli' && grep -q 'PortableCommandAlias: senzor-cli' '$cli'"

# The submission, as requests (WINGET_DRY_RUN): one commit with the three files, then the pull request.
submit() { WINGET_DRY_RUN=1 WINGET_DRY_RUN_TREE="$work/tree.json" "$@"; }
submit "$here/winget-submit.sh" "$work/Kankaj.Senzor" 1.2.3 Kankaj.Senzor 2> "$work/requests" > /dev/null \
    && ok "a submission runs" || bad "a submission runs: $(cat "$work/requests")"
check "the commit holds the package's three files" \
    "[[ \$(jq -r '.tree[].path' '$work/tree.json' | sort | tr '\n' ' ') == 'manifests/k/Kankaj/Senzor/1.2.3/Kankaj.Senzor.installer.yaml manifests/k/Kankaj/Senzor/1.2.3/Kankaj.Senzor.locale.en-US.yaml manifests/k/Kankaj/Senzor/1.2.3/Kankaj.Senzor.yaml ' ]]"
check "with their contents" "jq -r '.tree[].content' '$work/tree.json' | grep -q 'InstallerSha256: ${x64^^}'"
check "on top of winget-pkgs' master" "jq -e '.base_tree == \"base-tree\"' '$work/tree.json' > /dev/null"
check "the fork is synced first" "grep -q 'api -X POST repos/tester/winget-pkgs/merge-upstream' '$work/requests'"
check "a first submission is a new package" "grep -q 'title=New package: Kankaj.Senzor version 1.2.3' '$work/requests'"
check "from the fork's branch" "grep -q 'head=tester:Kankaj.Senzor-1.2.3' '$work/requests'"
WINGET_DRY_RUN_EXISTS=1 submit "$here/winget-submit.sh" "$work/Kankaj.Senzor" 1.2.3 Kankaj.Senzor 2> "$work/requests" > /dev/null || true
check "a later one is a new version" "grep -q 'title=New version: Kankaj.Senzor version 1.2.3' '$work/requests'"
if submit "$here/winget-submit.sh" "$work/nothing" 1.2.3 Kankaj.Senzor > /dev/null 2>&1; then
    bad "missing manifests fail the submission"
else
    ok "missing manifests fail the submission"
fi

if "$here/winget.sh" Kankaj.Other 1.2.3 "$x64" "$arm" "$work/x" > /dev/null 2>&1; then bad "an unknown package is refused"; else ok "an unknown package is refused"; fi
if "$here/winget.sh" Kankaj.Senzor 1.2.3-rc.1 "$x64" "$arm" "$work/x" > /dev/null 2>&1; then bad "a pre-release is refused"; else ok "a pre-release is refused"; fi
if "$here/winget.sh" Kankaj.Senzor 1.2.3 short "$arm" "$work/x" > /dev/null 2>&1; then bad "a bad checksum is refused"; else ok "a bad checksum is refused"; fi

if [[ $failures -gt 0 ]]; then
    echo "$failures failed"
    exit 1
fi
echo "all passed"
