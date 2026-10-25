# This builder has no ONNX Runtime, OpenCV or Python build dependency.
FROM rust:1.99.1-trixie@sha256:e259494b14672950a909b4c54527e19d29cae6ebf980672114c8bb85e5594e4a AS gateway-build
ARG DEBIAN_SNAPSHOT=20261017T000000Z
RUN rm -f /etc/apt/sources.list.d/debian.sources \
    && printf 'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/%s trixie main\ndeb-src [check-valid-until=no] http://snapshot.debian.org/archive/debian/%s trixie main\n' "$DEBIAN_SNAPSHOT" "$DEBIAN_SNAPSHOT" > /etc/apt/sources.list \
    && apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates curl xz-utils libc6-dev binutils build-essential \
    && mkdir /glibc-source \
    && cd /glibc-source \
    && apt-get source --download-only --only-source "glibc=$(dpkg-query -W -f='${source:Version}' libc6-dev)" \
    && rm -rf /var/lib/apt/lists/*
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
    && test -x /usr/local/bin/cargo-auditable
ENV CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0 RUSTFLAGS="-Ctarget-feature=+crt-static"
WORKDIR /app
COPY Cargo.toml Cargo.lock /app/
COPY src /app/src
COPY tests /app/tests
COPY benches /app/benches
COPY runtime /app/runtime
COPY shared /app/shared
COPY backend/tests/data /app/backend/tests/data
COPY backend/models.lock.json /app/backend/models.lock.json
COPY scripts /app/scripts
COPY docker /app/docker
COPY docs /app/docs
COPY LICENSE THIRD_PARTY_NOTICES.md /app/
COPY licenses /app/licenses
ARG PROFILE=release
# The release workflow passes the commit SHA. Telemetry reports it in
# service.version and the Sentry release; empty keeps the crate version.
ARG SOURCE_REVISION=
RUN /bin/bash -euo pipefail -c 'case "$PROFILE" in release|maxperf|profiling) ;; *) printf "unsupported production profile: %s\n" "$PROFILE" >&2; exit 1;; esac; \
    if [[ -n "$SOURCE_REVISION" ]]; then [[ "$SOURCE_REVISION" =~ ^[0-9a-f]{40}$ ]] || { printf "SOURCE_REVISION must be a 40-hex commit: %s\n" "$SOURCE_REVISION" >&2; exit 1; }; export INDRA_SOURCE_REVISION="$SOURCE_REVISION"; fi; \
    mkdir -p /out/gates /out/build-evidence; \
    cargo auditable test --locked --profile "$PROFILE" --manifest-path runtime/Cargo.toml --target x86_64-unknown-linux-gnu -p indra-gateway --lib --no-run; \
    count=0; for file in runtime/target/x86_64-unknown-linux-gnu/"$PROFILE"/deps/indra_gateway-*; do \
      if [[ -f "$file" && -x "$file" ]]; then cp "$file" /out/gates/gateway-unit; count=$((count + 1)); fi; \
    done; [[ "$count" = 1 ]]; \
    cargo auditable test --locked --profile "$PROFILE" --manifest-path runtime/Cargo.toml --target x86_64-unknown-linux-gnu -p indra-gateway --test process_lifecycle --no-run; \
    count=0; for file in runtime/target/x86_64-unknown-linux-gnu/"$PROFILE"/deps/process_lifecycle-*; do \
      if [[ -f "$file" && -x "$file" ]]; then cp "$file" /out/gates/gateway-process; count=$((count + 1)); fi; \
    done; [[ "$count" = 1 ]]; \
    cargo auditable build --locked --profile "$PROFILE" --manifest-path runtime/Cargo.toml --target x86_64-unknown-linux-gnu -p indra-gateway --bin indra-gateway --example stack_acceptance --example trace_acceptance --example telemetry_http; \
    cp runtime/target/x86_64-unknown-linux-gnu/"$PROFILE"/indra-gateway /out/indra-gateway; \
    cp runtime/target/x86_64-unknown-linux-gnu/"$PROFILE"/examples/stack_acceptance /out/indra-stack-acceptance; \
    cp runtime/target/x86_64-unknown-linux-gnu/"$PROFILE"/examples/trace_acceptance /out/indra-trace-acceptance; \
    cp runtime/target/x86_64-unknown-linux-gnu/"$PROFILE"/examples/telemetry_http /out/indra-telemetry-http; \
    for binary in /out/indra-gateway /out/indra-trace-acceptance /out/indra-stack-acceptance /out/indra-telemetry-http /out/gates/*; do \
      readelf -W -l "$binary" > "/out/build-evidence/$(basename "$binary").program-headers.txt"; \
      readelf -W -d "$binary" > "/out/build-evidence/$(basename "$binary").dynamic.txt"; \
      if grep -q INTERP "/out/build-evidence/$(basename "$binary").program-headers.txt"; then exit 1; fi; \
      if grep -q NEEDED "/out/build-evidence/$(basename "$binary").dynamic.txt"; then exit 1; fi; \
      sha256sum "$binary" >> /out/build-evidence/binaries.sha256; \
    done; \
    /out/indra-gateway check-config > /out/build-evidence/default-config-check.txt; \
    /out/indra-gateway openapi > /out/build-evidence/openapi.json; \
    rustc -Vv > /out/build-evidence/rustc.txt; \
    printf "profile=%s\n" "$PROFILE" > /out/build-evidence/profile.txt; \
    bash scripts/collect-runtime-materials.sh gateway /out/relink /out/indra-gateway'

# The runnable Tika image copies this binary into its distroless Java runtime.
FROM scratch AS gateway
LABEL org.opencontainers.image.title="Tika admission and shutdown gateway" \
      org.opencontainers.image.source="https://github.com/somehowchris/tika-indra-ocr-client"
COPY --from=gateway-build --chown=65532:65532 /out/indra-gateway /indra-gateway
COPY --from=gateway-build --chown=65532:65532 /out/relink /relink
COPY --from=gateway-build /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
ENV SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt
USER 65532:65532
ENTRYPOINT ["/indra-gateway"]

# No Java is needed: the process harness is its own controlled Rust child.
FROM gateway AS gateway-acceptance
COPY --from=gateway-build /out/gates /gates
COPY --from=gateway-build /out/build-evidence /build-evidence
COPY --from=gateway-build /out/indra-stack-acceptance /indra-stack-acceptance
COPY --from=gateway-build /out/indra-trace-acceptance /indra-trace-acceptance
# Static driver of the telemetry-overhead study, built with the same toolchain.
COPY --from=gateway-build /out/indra-telemetry-http /indra-telemetry-http
ENV INDRA_TEST_GATEWAY_BIN=/indra-gateway

# Export this target with Buildx --output type=local,dest=... . The host supplies
# docker compose or kubectl; neither CLI nor its credentials belong in a service.
FROM scratch AS acceptance-tools
COPY --from=gateway-build /out/indra-stack-acceptance /indra-stack-acceptance
COPY --from=gateway-build /out/indra-trace-acceptance /indra-trace-acceptance
