#!/usr/bin/env bash
# Tests for dev/docker-image.sh: from two Linux archives as dist makes them, it builds the
# senzor-cli image with every tag of the release, and the image runs the CLI as a non-root user
# with the CA certificates, LICENSE and NOTICE. Builds for this machine only (no push). Needs
# Docker with buildx. Run: dev/docker-image.test.sh
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
image="senzor-cli-test-$$"
cleanup() {
    docker image ls --format '{{.Repository}}:{{.Tag}}' "$image" | xargs -r docker image rm -f > /dev/null
    rm -rf "$work"
}
trap cleanup EXIT
failures=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; failures=$((failures + 1)); }
check() {
    local name=$1 condition=$2
    if eval "$condition"; then ok "$name"; else bad "$name"; fi
}

# Archives laid out as dist's: senzor-cli-<target>/{senzor-cli,LICENSE,NOTICE,...} plus a
# .sha256 file each. The "binary" is a script that says which architecture it is.
mkdir -p "$work/archives"
for target in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu; do
    dir="$work/src/senzor-cli-$target"
    mkdir -p "$dir"
    printf '#!/bin/sh\necho "senzor-cli %s $*"\n' "$target" > "$dir/senzor-cli"
    chmod +x "$dir/senzor-cli"
    echo "licence text" > "$dir/LICENSE"
    echo "notice text" > "$dir/NOTICE"
    echo "readme" > "$dir/README.md"
    tar -C "$work/src" -cJf "$work/archives/senzor-cli-$target.tar.xz" "senzor-cli-$target"
    (cd "$work/archives" && sha256sum --binary "senzor-cli-$target.tar.xz" > "senzor-cli-$target.tar.xz.sha256")
done

build() { DOCKER_IMAGE_LOAD=1 "$here/docker-image.sh" "$@"; }

if build "$work/archives" 1.2.3 "$image" > "$work/out" 2>&1; then
    ok "it builds the image"
else
    bad "it builds the image: $(cat "$work/out")"
    echo "$failures failed"
    exit 1
fi
for tag in 1.2.3 1.2 1 latest; do
    check "tagged $tag" "docker image inspect '$image:$tag' > /dev/null 2>&1"
done
case $(uname -m) in
    x86_64) arch=x86_64-unknown-linux-gnu ;;
    aarch64 | arm64) arch=aarch64-unknown-linux-gnu ;;
esac
check "senzor-cli is the entry point, with this machine's binary" \
    "[[ \$(docker run --rm '$image:1.2.3' --version) == 'senzor-cli $arch --version' ]]"
check "it runs as a user other than root" \
    "[[ \$(docker run --rm --entrypoint id '$image:1.2.3' -u) != 0 ]]"
check "a project mounted at /work is the working directory" \
    "[[ \$(docker run --rm --entrypoint pwd '$image:1.2.3') == /work ]]"
check "the CA certificates for TLS" \
    "docker run --rm --entrypoint test '$image:1.2.3' -s /etc/ssl/certs/ca-certificates.crt"
check "LICENSE and NOTICE" \
    "docker run --rm --entrypoint cat '$image:1.2.3' /usr/share/doc/senzor-cli/LICENSE /usr/share/doc/senzor-cli/NOTICE | grep -q 'notice text'"
check "labels: version and source repository" \
    "[[ \$(docker image inspect -f '{{index .Config.Labels \"org.opencontainers.image.version\"}} {{index .Config.Labels \"org.opencontainers.image.source\"}}' '$image:1.2.3') == '1.2.3 https://github.com/kankaj/Senzor' ]]"

# Refusals, before anything is built.
check "a pre-release version is refused" "! build '$work/archives' 1.2.3-rc.1 '$image' > /dev/null 2>&1"
echo "tampered" >> "$work/archives/senzor-cli-x86_64-unknown-linux-gnu.tar.xz"
check "an archive that fails its checksum is refused" "! build '$work/archives' 1.2.3 '$image' > /dev/null 2>&1"

if ((failures)); then
    echo "$failures failed"
    exit 1
fi
echo "all passed"
