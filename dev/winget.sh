#!/usr/bin/env bash
# Writes the winget manifests of a release (schema 1.10.0): version, installer and en-US locale,
# under OUT_DIR/manifests/k/Kankaj/<name>/<version>/ as in microsoft/winget-pkgs. Kankaj.Senzor
# installs the window's MSI, Kankaj.SenzorCLI the CLI's zip as a portable command; both for x64
# and arm64. The release workflow runs it and submits the manifests to winget-pkgs (winget.yml).
#
# Usage: dev/winget.sh PACKAGE_ID VERSION X64_SHA256 ARM64_SHA256 OUT_DIR [RELEASE_DATE]
set -euo pipefail

id=$1
version=$2
x64=$3
arm=$4
date=${6:-$(date -u +%F)}
fail() { echo "winget: $*" >&2; exit 1; }
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "give the release's version as X.Y.Z, not \"$version\""
for sha in "$x64" "$arm"; do
    [[ $sha =~ ^[0-9a-f]{64}$ ]] || fail "\"$sha\" is not a SHA-256"
done
minor=$(cut -d. -f1,2 <<< "$version")
releases="https://github.com/kankaj/Senzor/releases/download/v$version"

case $id in
    Kankaj.Senzor)
        name=Senzor
        moniker=senzor
        short="MQTT sensor simulator: a window to edit, run and watch simulated sensors"
        ;;
    Kankaj.SenzorCLI)
        name="Senzor CLI"
        moniker=senzor-cli
        short="MQTT sensor simulator: run Senzor projects headless, in a terminal, Docker or CI"
        ;;
    *) fail "unknown package \"$id\": Kankaj.Senzor or Kankaj.SenzorCLI" ;;
esac
dir="$5/manifests/k/Kankaj/${id#Kankaj.}/$version"
mkdir -p "$dir"
header() { echo "# Written by Senzor's release workflow (dev/winget.sh in kankaj/Senzor)."; echo "# yaml-language-server: \$schema=https://aka.ms/winget-manifest.$1.1.10.0.schema.json"; echo; }

{
    header version
    echo "PackageIdentifier: $id"
    echo "PackageVersion: $version"
    echo "DefaultLocale: en-US"
    echo "ManifestType: version"
    echo "ManifestVersion: 1.10.0"
} > "$dir/$id.yaml"

{
    header installer
    echo "PackageIdentifier: $id"
    echo "PackageVersion: $version"
    if [[ $id == Kankaj.Senzor ]]; then
        # The MSI of crates/senzor/wix/main.wxs: installed for all users, found again by its
        # upgrade code, and named as Windows lists it under Apps (its publisher without the háček).
        echo "InstallerType: wix"
        echo "Scope: machine"
        echo "UpgradeBehavior: install"
        echo "ReleaseDate: $date"
        echo "AppsAndFeaturesEntries:"
        echo "- DisplayName: Senzor"
        echo "  Publisher: Jaroslav Kanka"
        echo "  UpgradeCode: '{0046E879-2FB5-4DCB-98A7-564C93156A90}'"
        echo "Installers:"
        echo "- Architecture: x64"
        echo "  InstallerUrl: $releases/senzor-x86_64-pc-windows-msvc.msi"
        echo "  InstallerSha256: ${x64^^}"
        echo "- Architecture: arm64"
        echo "  InstallerUrl: $releases/senzor-aarch64-pc-windows-msvc.msi"
        echo "  InstallerSha256: ${arm^^}"
    else
        echo "InstallerType: zip"
        echo "NestedInstallerType: portable"
        echo "NestedInstallerFiles:"
        echo "- RelativeFilePath: senzor-cli.exe"
        echo "  PortableCommandAlias: senzor-cli"
        echo "ReleaseDate: $date"
        echo "Installers:"
        echo "- Architecture: x64"
        echo "  InstallerUrl: $releases/senzor-cli-x86_64-pc-windows-msvc.zip"
        echo "  InstallerSha256: ${x64^^}"
        echo "- Architecture: arm64"
        echo "  InstallerUrl: $releases/senzor-cli-aarch64-pc-windows-msvc.zip"
        echo "  InstallerSha256: ${arm^^}"
    fi
    echo "ManifestType: installer"
    echo "ManifestVersion: 1.10.0"
} > "$dir/$id.installer.yaml"

{
    header defaultLocale
    echo "PackageIdentifier: $id"
    echo "PackageVersion: $version"
    echo "PackageLocale: en-US"
    echo "Publisher: Jaroslav Kaňka"
    echo "PublisherUrl: https://github.com/kankaj"
    echo "PublisherSupportUrl: https://github.com/kankaj/Senzor/issues"
    echo "Author: Jaroslav Kaňka"
    echo "PackageName: $name"
    echo "PackageUrl: https://senzor.kankaj.cz/"
    echo "License: Apache-2.0"
    echo "LicenseUrl: https://github.com/kankaj/Senzor/blob/v$version/LICENSE"
    echo "Copyright: Copyright 2026 Jaroslav Kaňka"
    echo "ShortDescription: \"$short\""
    echo "Description: |-"
    echo "  Senzor simulates MQTT sensors, so you can test whatever consumes them: a dashboard, a backend"
    echo "  that stores readings, a home automation setup. A template describes a kind of sensor once, and"
    echo "  Senzor runs as many copies of it as you ask for, each with its own connection to your broker."
    echo "  It comes as a desktop window and as senzor-cli for terminals, Docker and CI; an AI agent can"
    echo "  drive either over MCP."
    echo "Moniker: $moniker"
    echo "Tags:"
    for tag in mqtt mqtt5 iot simulator sensor testing home-assistant mcp; do echo "- $tag"; done
    echo "ReleaseNotesUrl: https://github.com/kankaj/Senzor/releases/tag/v$version"
    echo "Documentations:"
    echo "- DocumentLabel: Documentation"
    echo "  DocumentUrl: https://senzor.kankaj.cz/$minor/docs/"
    echo "ManifestType: defaultLocale"
    echo "ManifestVersion: 1.10.0"
} > "$dir/$id.locale.en-US.yaml"

echo "winget: $dir"
