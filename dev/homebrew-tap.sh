#!/usr/bin/env bash
# Puts a release into Senzor's Homebrew tap (kankaj/homebrew-tap): the window's cask as
# Casks/senzor.rb (written by dev/homebrew-cask.sh) and senzor-cli's formula as
# Formula/senzor-cli.rb (written by dist, a file of the release), in one commit. It is the tap's
# only writer: two jobs pushing to it side by side made one of them fail at 1.0.0. A commit
# pushed in between is still taken in: the push is retried on top of it.
#
# Usage: dev/homebrew-tap.sh TAP_CHECKOUT CASK FORMULA VERSION
set -euo pipefail

tap=$1
cask=$(cd "$(dirname "$2")" && pwd)/$(basename "$2")
formula=$(cd "$(dirname "$3")" && pwd)/$(basename "$3")
version=$4

# The runner has no git identity: the commit and a rebase onto a commit pushed in between both
# need one.
export GIT_AUTHOR_NAME='github-actions[bot]' GIT_COMMITTER_NAME='github-actions[bot]'
export GIT_AUTHOR_EMAIL='41898282+github-actions[bot]@users.noreply.github.com'
export GIT_COMMITTER_EMAIL=$GIT_AUTHOR_EMAIL

cd "$tap"
mkdir -p Casks Formula
cp "$cask" Casks/senzor.rb
cp "$formula" Formula/senzor-cli.rb
git add Casks/senzor.rb Formula/senzor-cli.rb
if git diff --cached --quiet; then
    echo "homebrew-tap: the tap is up to date"
    exit 0
fi
git commit -q -m "senzor and senzor-cli $version"
for _ in 1 2 3; do
    git push -q origin HEAD && exit 0
    git pull -q --rebase origin "$(git branch --show-current)"
done
echo "homebrew-tap: could not push to the tap" >&2
exit 1
