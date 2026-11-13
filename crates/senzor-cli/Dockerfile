# senzor-cli as a container image, ghcr.io/kankaj/senzor-cli, for linux/amd64 and linux/arm64.
# It packs the release's own Linux binaries rather than building them again: dev/docker-image.sh
# lays out the build context from the release archives (amd64/ and arm64/ with senzor-cli, plus
# LICENSE and NOTICE) and builds it; the release workflow pushes it (ghcr.yml).
#
# Debian rather than an image without a shell: CI jobs run their steps in it (bash, kill, wait),
# and the binaries need glibc 2.35 or newer. Pinned by digest; to update it, take the digest of
# the index from `docker buildx imagetools inspect debian:trixie-slim`.
ARG DEBIAN=debian:trixie-slim@sha256:a72b4433de1cf7d6bae0c4cee74d0547f58601a643b3375f070c51600d63b6b2

# The CA bundle is the same file on every architecture: fetch it once, on the build machine, so
# the image needs no emulation to build for the other architecture.
FROM --platform=$BUILDPLATFORM ${DEBIAN} AS certs
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

FROM ${DEBIAN}
ARG TARGETARCH
ARG VERSION
LABEL org.opencontainers.image.title="senzor-cli" \
      org.opencontainers.image.description="MQTT sensor simulator: run Senzor projects headless" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.source="https://github.com/kankaj/Senzor" \
      org.opencontainers.image.url="https://senzor.kankaj.cz" \
      org.opencontainers.image.licenses="Apache-2.0" \
      org.opencontainers.image.vendor="Jaroslav Kaňka"
COPY --from=certs /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --chmod=755 ${TARGETARCH}/senzor-cli /usr/local/bin/senzor-cli
COPY LICENSE NOTICE /usr/share/doc/senzor-cli/
# Mount the project here: docker run -v "$PWD:/work" ghcr.io/kankaj/senzor-cli run project.toml
WORKDIR /work
USER 10001:10001
ENTRYPOINT ["senzor-cli"]
CMD ["--help"]
