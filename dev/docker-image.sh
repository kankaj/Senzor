#!/usr/bin/env bash
# Builds the senzor-cli container image (crates/senzor-cli/Dockerfile) for linux/amd64 and
# linux/arm64 from a release's two Linux archives, as dist makes them, after checking each against
# its .sha256 file, and pushes it tagged X.Y.Z, X.Y, X and latest. The release workflow runs it
# (ghcr.yml) after logging in to ghcr.io.
#
# DOCKER_IMAGE_LOAD=1 builds for this machine only and loads the image into Docker instead of
# pushing it, for the tests and for trying an image locally.
#
# Usage: dev/docker-image.sh ARCHIVE_DIR VERSION IMAGE
#   ARCHIVE_DIR holds senzor-cli-{x86_64,aarch64}-unknown-linux-gnu.tar.xz and their .sha256
#   IMAGE is the name without a tag, e.g. ghcr.io/kankaj/senzor-cli
set -euo pipefail

archives=$1
version=$2
image=$3
root=$(cd "$(dirname "$0")/.." && pwd)
fail() { echo "docker-image: $*" >&2; exit 1; }
[[ $version =~ ^([0-9]+)\.([0-9]+)\.[0-9]+$ ]] \
    || fail "give the release's version as X.Y.Z, not \"$version\" (pre-releases get no image)"
major=${BASH_REMATCH[1]}
minor=$major.${BASH_REMATCH[2]}

context=$(mktemp -d)
trap 'rm -rf "$context"' EXIT
for pair in x86_64:amd64 aarch64:arm64; do
    target=${pair%%:*}-unknown-linux-gnu
    arch=${pair##*:}
    archive=senzor-cli-$target.tar.xz
    (cd "$archives" && sha256sum --check --quiet "$archive.sha256") \
        || fail "$archive does not match its checksum"
    tar -C "$context" -xJf "$archives/$archive"
    mkdir -p "$context/$arch"
    mv "$context/senzor-cli-$target/senzor-cli" "$context/$arch/senzor-cli"
done
cp "$context/senzor-cli-x86_64-unknown-linux-gnu/LICENSE" "$context/senzor-cli-x86_64-unknown-linux-gnu/NOTICE" "$context/"

args=(--file "$root/crates/senzor-cli/Dockerfile" --build-arg "VERSION=$version")
for tag in "$version" "$minor" "$major" latest; do
    args+=(--tag "$image:$tag")
done
if [[ ${DOCKER_IMAGE_LOAD:-} == 1 ]]; then
    docker buildx build "${args[@]}" --load "$context"
else
    # Docker's own builder cannot push an image for two platforms; a container builder can.
    docker buildx inspect senzor-image > /dev/null 2>&1 \
        || docker buildx create --name senzor-image --driver docker-container > /dev/null
    docker buildx build "${args[@]}" --builder senzor-image \
        --platform linux/amd64,linux/arm64 --push "$context"
fi
