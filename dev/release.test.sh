#!/usr/bin/env bash
# Tests for dev/release.sh in a throwaway git repository (no cargo: RELEASE_SKIP_CARGO=1).
# Run: dev/release.test.sh
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
failures=0

ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; failures=$((failures + 1)); }

# A repository on main with an Unreleased 1.0.0 section and its Czech highlights.
repo() {
    local dir=$1
    mkdir -p "$dir/dev" "$dir/crates/senzor/locales/cs"
    cp "$here/release.sh" "$here/release-check.sh" "$dir/dev/"
    printf '[workspace]\nmembers = []\n\n[workspace.package]\nversion = "0.9.0"\nedition = "2024"\n\n[dependencies]\nversion = "9"\n' \
        > "$dir/Cargo.toml"
    printf '# Changelog\n\n## [1.0.0] - Unreleased\n\n### Highlights\n\n- New.\n\n## [0.9.0] - 2026-01-01\n\n### Highlights\n\n- Old.\n' \
        > "$dir/CHANGELOG.md"
    printf '# Co je nového\n\n## [1.0.0]\n\n### Highlights\n\n- Nové.\n' > "$dir/crates/senzor/locales/cs/whats-new.md"
    git -C "$dir" init -q -b main
    # As in Senzor's own .git/config: no signing, so no GPG prompt opens during the tests.
    git -C "$dir" config commit.gpgsign false
    git -C "$dir" config tag.gpgSign false
    git -C "$dir" -c user.name=t -c user.email=t@t add -A
    git -C "$dir" -c user.name=t -c user.email=t@t -c commit.gpgsign=false commit -q -m "start"
}

# Yes to every question. (Not `yes`: it dies of SIGPIPE when the script exits, which pipefail
# would count as a failure.)
answers() { printf 'y\ny\ny\ny\ny\n'; }

run() { (cd "$1" && shift && RELEASE_SKIP_CARGO=1 GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@t \
    GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@t dev/release.sh "$@"); }

# A release, every question answered yes.
repo "$work/a"
if out=$(answers | run "$work/a" 1.0.0 2>&1); then ok "a release runs"; else bad "a release runs: $out"; fi
grep -q '^version = "1.0.0"' "$work/a/Cargo.toml" && ok "the version is set" || bad "the version is set"
grep -q '^version = "9"' "$work/a/Cargo.toml" && ok "other version keys stay" || bad "other version keys stay"
grep -qE "^## \[1\.0\.0\] - $(date +%F)$" "$work/a/CHANGELOG.md" && ok "the section is dated today" \
    || bad "the section is dated today"
[[ $(git -C "$work/a" log -1 --format=%s) == "release: 1.0.0" ]] && ok "the release commit" || bad "the release commit"
[[ $(git -C "$work/a" cat-file -t v1.0.0 2>/dev/null) == tag ]] && ok "an annotated tag" || bad "an annotated tag"
[[ $out == *"git push origin main v1.0.0"* ]] && ok "it tells the push command" || bad "it tells the push command: $out"
[[ -z $(git -C "$work/a" ls-remote 2>/dev/null || true) ]] && ok "nothing is pushed" || bad "nothing is pushed"

# Answering no stops before the tree changes.
repo "$work/b"
if out=$(printf 'n\n' | run "$work/b" 1.0.0 2>&1); then bad "no stops: exit 0"; else ok "no stops"; fi
grep -q '^version = "0.9.0"' "$work/b/Cargo.toml" && ok "no leaves the version" || bad "no leaves the version"

# A dirty tree is refused.
repo "$work/c"
echo x >> "$work/c/CHANGELOG.md"
if out=$(answers | run "$work/c" 1.0.0 2>&1); then bad "a dirty tree is refused"; else
    [[ $out == *"uncommitted changes"* ]] && ok "a dirty tree is refused" || bad "a dirty tree is refused: $out"
fi

# Another branch is refused.
repo "$work/d"
git -C "$work/d" switch -q -c topic
if out=$(answers | run "$work/d" 1.0.0 2>&1); then bad "another branch is refused"; else
    [[ $out == *"release from main"* ]] && ok "another branch is refused" || bad "another branch is refused: $out"
fi

# A pre-release keeps the section undated.
repo "$work/e"
if out=$(answers | run "$work/e" 1.0.0-rc.1 2>&1); then ok "a pre-release runs"; else bad "a pre-release runs: $out"; fi
grep -q '^## \[1\.0\.0\] - Unreleased$' "$work/e/CHANGELOG.md" && ok "a pre-release leaves Unreleased" \
    || bad "a pre-release leaves Unreleased"
[[ $(git -C "$work/e" cat-file -t v1.0.0-rc.1 2>/dev/null) == tag ]] && ok "the pre-release tag" || bad "the pre-release tag"

# A version that is not X.Y.Z is refused.
repo "$work/f"
if out=$(answers | run "$work/f" 1.0 2>&1); then bad "a bad version is refused"; else ok "a bad version is refused"; fi

if [[ $failures -gt 0 ]]; then
    echo "$failures failed"
    exit 1
fi
echo "all passed"
