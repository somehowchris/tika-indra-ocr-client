FROM rust:1.99.1-trixie@sha256:e259494b14672950a909b4c54527e19d29cae6ebf980672114c8bb85e5594e4a AS build
ARG DEBIAN_SNAPSHOT=20261017T000000Z
RUN rm -f /etc/apt/sources.list.d/debian.sources \
    && printf 'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/%s trixie main\ndeb-src [check-valid-until=no] http://snapshot.debian.org/archive/debian/%s trixie main\n' "$DEBIAN_SNAPSHOT" "$DEBIAN_SNAPSHOT" > /etc/apt/sources.list \
    && apt-get update && apt-get install -y --no-install-recommends libc6-dev binutils python3 \
    && mkdir /glibc-source \
    && cd /glibc-source \
    && apt-get source --download-only --only-source "glibc=$(dpkg-query -W -f='${source:Version}' libc6-dev)" \
    && rm -rf /var/lib/apt/lists/*
RUN rustup target add x86_64-unknown-linux-gnu
WORKDIR /build
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY src ./src
COPY shared ./shared
COPY benches ./benches
COPY tests ./tests
COPY LICENSE THIRD_PARTY_NOTICES.md ./
COPY licenses/INDRA-MODEL-LICENSE.txt ./licenses/INDRA-MODEL-LICENSE.txt
COPY docker/bridge.Dockerfile ./docker/bridge.Dockerfile
COPY docs/relinking.md ./docs/relinking.md
COPY scripts/collect_relink_materials.py ./scripts/collect_relink_materials.py
ARG PROFILE=release
RUN cargo fetch --locked
RUN RUSTFLAGS='-C target-feature=+crt-static' \
    cargo build --locked --offline --jobs 2 --profile "${PROFILE}" --target x86_64-unknown-linux-gnu \
    && cp "target/x86_64-unknown-linux-gnu/${PROFILE}/indra-tesseract" /indra-tesseract \
    && readelf -l /indra-tesseract > /program-headers \
    && readelf -d /indra-tesseract > /dynamic-section \
    && ! grep -q INTERP /program-headers \
    && ! grep -q NEEDED /dynamic-section \
    && /indra-tesseract --version
RUN python3 scripts/collect_relink_materials.py \
    --project /build --glibc-source /glibc-source --output /relink --binary /indra-tesseract

FROM scratch
COPY --from=build --chown=65532:65532 /indra-tesseract /indra-tesseract
COPY --from=build --chown=65532:65532 /relink /relink
USER 65532:65532
ENTRYPOINT ["/indra-tesseract"]
