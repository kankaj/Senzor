#!/usr/bin/env bash
# Writes the Homebrew cask of the window, Casks/senzor.rb in kankaj/homebrew-tap, for a release:
# its version and the SHA-256 of its two Mac apps (Senzor-<arch>-apple-darwin.zip). The release
# workflow runs it and pushes the cask (homebrew-tap.yml); dist makes the formula of senzor-cli.
#
# Usage: dev/homebrew-cask.sh VERSION ARM_SHA256 INTEL_SHA256 > senzor.rb
set -euo pipefail

version=$1
arm=$2
intel=$3
fail() { echo "homebrew-cask: $*" >&2; exit 1; }
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "give the release's version as X.Y.Z, not \"$version\""
for sha in "$arm" "$intel"; do
    [[ $sha =~ ^[0-9a-f]{64}$ ]] || fail "\"$sha\" is not a SHA-256"
done

cat <<EOF
# Written by Senzor's release workflow (dev/homebrew-cask.sh in kankaj/Senzor): do not edit.
cask "senzor" do
  arch arm: "aarch64", intel: "x86_64"

  version "$version"
  sha256 arm:   "$arm",
         intel: "$intel"

  url "https://github.com/kankaj/Senzor/releases/download/v#{version}/Senzor-#{arch}-apple-darwin.zip"
  name "Senzor"
  desc "MQTT sensor simulator"
  homepage "https://senzor.kankaj.cz/"

  livecheck do
    url :url
    strategy :github_latest
  end

  app "Senzor.app"

  zap trash: [
    "~/Library/Application Support/cz.kankaj.Senzor",
    "~/Library/Saved Application State/cz.kankaj.Senzor.savedState",
  ]

  caveats <<~CAVEATS
    Senzor is not signed by Apple, so macOS blocks it the first time you open it.
    Open it once from Applications, then choose System Settings > Privacy & Security >
    Open Anyway (or, on older macOS, right-click Senzor.app and choose Open).
  CAVEATS
end
EOF
