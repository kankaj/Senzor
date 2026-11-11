#!/usr/bin/env bash
# Tests for dev/homebrew-tap.sh on any system, against throwaway repositories: the cask and the
# formula land in the tap in one commit, a commit someone else pushed meanwhile is kept, and an
# unchanged release adds nothing. Run: dev/homebrew-tap.test.sh
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
quiet_git() { git -c init.defaultBranch=main -c commit.gpgsign=false "$@" > /dev/null 2>&1; }

# The tap on GitHub, and the job's checkout of it.
quiet_git init --bare "$work/tap.git"
quiet_git clone "$work/tap.git" "$work/seed"
echo "Senzor's Homebrew tap" > "$work/seed/README.md"
quiet_git -C "$work/seed" add README.md
quiet_git -C "$work/seed" -c user.name=t -c user.email=t@t commit -m init
quiet_git -C "$work/seed" push origin HEAD:main
quiet_git clone "$work/tap.git" "$work/job"
git -C "$work/job" config commit.gpgsign false

# Someone else pushes after the job checked the tap out, as a second job would.
echo "more" >> "$work/seed/README.md"
quiet_git -C "$work/seed" -c user.name=t -c user.email=t@t commit -am "someone else"
quiet_git -C "$work/seed" push origin HEAD:main

echo 'cask "senzor" do end' > "$work/senzor.rb"
echo 'class SenzorCli < Formula; end' > "$work/senzor-cli.rb"

if "$here/homebrew-tap.sh" "$work/job" "$work/senzor.rb" "$work/senzor-cli.rb" 1.0.1 > "$work/log" 2>&1; then
    ok "it pushes"
else
    bad "it pushes: $(cat "$work/log")"
fi
quiet_git clone "$work/tap.git" "$work/after"
check "the cask is in the tap" "cmp -s '$work/after/Casks/senzor.rb' '$work/senzor.rb'"
check "the formula is in the tap" "cmp -s '$work/after/Formula/senzor-cli.rb' '$work/senzor-cli.rb'"
check "both in one commit" \
    "[[ \$(git -C '$work/after' show --name-only --format= HEAD | sort | tr '\n' ' ') == 'Casks/senzor.rb Formula/senzor-cli.rb ' ]]"
check "the commit names the release" "[[ \$(git -C '$work/after' log -1 --format=%s) == 'senzor and senzor-cli 1.0.1' ]]"
check "the other commit stays" "git -C '$work/after' log --format=%s | grep -qx 'someone else'"

before=$(git -C "$work/after" rev-parse HEAD)
quiet_git -C "$work/job" pull --rebase
"$here/homebrew-tap.sh" "$work/job" "$work/senzor.rb" "$work/senzor-cli.rb" 1.0.1 > "$work/log" 2>&1
quiet_git -C "$work/after" pull
check "an unchanged release adds nothing" "[[ \$(git -C '$work/after' rev-parse HEAD) == '$before' ]]"

if [[ $failures -gt 0 ]]; then
    echo "$failures failed"
    exit 1
fi
echo "all passed"
