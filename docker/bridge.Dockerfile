FROM rust:1.99.1-trixie@sha256:e259494b14672950a909b4c54527e19d29cae6ebf980672114c8bb85e5594e4a AS build
ARG DEBIAN_SNAPSHOT=20261017T000000Z
RUN rm -f /etc/apt/sources.list.d/debian.sources \
    && printf 'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/%s trixie main\ndeb-src [check-valid-until=no] http://snapshot.debian.org/archive/debian/%s trixie main\n' "$DEBIAN_SNAPSHOT" "$DEBIAN_SNAPSHOT" > /etc/apt/sources.list \
    && apt-get update && apt-get install -y --no-install-recommends ca-certificates curl xz-utils libc6-dev binutils \
    && mkdir /glibc-source \
    && cd /glibc-source \
    && apt-get source --download-only --only-source "glibc=$(dpkg-query -W -f='${source:Version}' libc6-dev)" \
    && rm -rf /var/lib/apt/lists/*
RUN rustup target add x86_64-unknown-linux-gnu
# Pinned cargo-auditable embeds the exact dependency tree in every shipped
# executable, so the signed SBOM names the Rust packages inside a scratch image.
# It is a build-only tool and never enters a runtime stage.
ARG CARGO_AUDITABLE_VERSION=0.7.7
ARG CARGO_AUDITABLE_SHA256=4c8d3b08c3c34e9f2333367224dbbfb4aab68067698b43de59fe19aded3416dd
RUN curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 \
      --connect-timeout 15 --max-time 180 --retry 2 --retry-max-time 240 \
      "https://github.com/rust-secure-code/cargo-auditable/releases/download/v${CARGO_AUDITABLE_VERSION}/cargo-auditable-x86_64-unknown-linux-gnu.tar.xz" \
      --output /tmp/cargo-auditable.tar.xz \
    && printf '%s  /tmp/cargo-auditable.tar.xz\n' "$CARGO_AUDITABLE_SHA256" | sha256sum --check --status \
    && tar -xJOf /tmp/cargo-auditable.tar.xz cargo-auditable-x86_64-unknown-linux-gnu/cargo-auditable > /usr/local/bin/cargo-auditable \
    && chmod 0755 /usr/local/bin/cargo-auditable \
    && rm /tmp/cargo-auditable.tar.xz \
    && cargo auditable --version
WORKDIR /build
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY src ./src
COPY shared ./shared
COPY benches ./benches
COPY tests ./tests
COPY LICENSE THIRD_PARTY_NOTICES.md ./
COPY licenses/INDRA-MODEL-LICENSE.txt ./licenses/INDRA-MODEL-LICENSE.txt
COPY licenses/INDRA-CODE-LICENSE.txt ./licenses/INDRA-CODE-LICENSE.txt
COPY docker/bridge.Dockerfile ./docker/bridge.Dockerfile
COPY docs/relinking.md ./docs/relinking.md
COPY scripts/collect-runtime-materials.sh ./scripts/collect-runtime-materials.sh
ARG PROFILE=release
RUN cargo fetch --locked
RUN RUSTFLAGS='-C target-feature=+crt-static' \
    cargo auditable build --locked --offline --jobs 2 --profile "${PROFILE}" --target x86_64-unknown-linux-gnu \
    && cp "target/x86_64-unknown-linux-gnu/${PROFILE}/indra-tesseract" /indra-tesseract \
    && readelf -l /indra-tesseract > /program-headers \
    && readelf -d /indra-tesseract > /dynamic-section \
    && ! grep -q INTERP /program-headers \
    && ! grep -q NEEDED /dynamic-section \
    && /indra-tesseract --version
RUN bash scripts/collect-runtime-materials.sh bridge /relink /indra-tesseract

FROM scratch
COPY --from=build --chown=65532:65532 /indra-tesseract /indra-tesseract
COPY --from=build --chown=65532:65532 /relink /relink
USER 65532:65532
ENTRYPOINT ["/indra-tesseract"]
