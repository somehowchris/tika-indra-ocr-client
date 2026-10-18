# Native compilation is a separate cached layer from application source.
# Every source revision, package snapshot and downloaded build wheel is pinned.
FROM rust:1.99.1-trixie@sha256:e259494b14672950a909b4c54527e19d29cae6ebf980672114c8bb85e5594e4a AS native-build
ARG DEBIAN_SNAPSHOT=20261017T000000Z
RUN rm -f /etc/apt/sources.list.d/debian.sources \
    && printf 'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/%s trixie main\ndeb-src [check-valid-until=no] http://snapshot.debian.org/archive/debian/%s trixie main\n' "$DEBIAN_SNAPSHOT" "$DEBIAN_SNAPSHOT" > /etc/apt/sources.list \
    && apt-get update \
    && apt-get install -y --no-install-recommends \
       ca-certificates git curl build-essential binutils pkg-config \
       python3-dev python3-venv clang-18 libclang-18-dev nasm util-linux \
    && mkdir /glibc-source \
    && cd /glibc-source \
    && apt-get source --download-only --only-source "glibc=$(dpkg-query -W -f='${source:Version}' libc6-dev)" \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY scripts/native-build-tools.lock scripts/build-native-runtime.sh scripts/record-native-runtime.sh ./scripts/
RUN python3 -m venv /opt/build-tools \
    && /bin/bash -c 'source scripts/native-build-tools.lock; printf "cmake==%s --hash=sha256:%s\nninja==%s --hash=sha256:%s\nlibclang==%s --hash=sha256:%s\n" "$CMAKE_VERSION" "$CMAKE_WHEEL_SHA256" "$NINJA_VERSION" "$NINJA_WHEEL_SHA256" "$LIBCLANG_VERSION" "$LIBCLANG_WHEEL_SHA256" > /build/build-tools.requirements' \
    && /opt/build-tools/bin/pip install --no-cache-dir --only-binary=:all: --require-hashes -r /build/build-tools.requirements
ENV PATH=/opt/build-tools/bin:/usr/local/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    CLANG_PATH=/usr/bin/clang-18 \
    LIBCLANG_PATH=/opt/build-tools/lib/python3.13/site-packages/clang/native \
    CARGO_BUILD_JOBS=2 \
    CARGO_INCREMENTAL=0 \
    CMAKE_BUILD_PARALLEL_LEVEL=2
# Hosted x86-64 runners expose CPUs 0 and 1; callers on other cpusets override.
# This is a compiler-affinity restriction, not a benchmark CFS attestation.
ARG BUILD_CPUSET=0,1
RUN taskset --cpu-list "$BUILD_CPUSET" bash scripts/build-native-runtime.sh /native

# This export target contains build evidence, not a runnable OCR service.
FROM scratch AS native-evidence
COPY --from=native-build /native/evidence /evidence

# Application compilation is cached independently of the expensive archives.
FROM native-build AS engine-build
WORKDIR /app
COPY runtime /app/runtime
COPY backend/tests/data /app/backend/tests/data
RUN /bin/bash -euo pipefail -c 'source /native/cargo-env.sh; \
    cargo test --locked --release --manifest-path runtime/Cargo.toml --target x86_64-unknown-linux-gnu -p indra-engine --lib --test native --no-run; \
    cargo build --locked --release --manifest-path runtime/Cargo.toml --target x86_64-unknown-linux-gnu -p indra-engine --examples; \
    mkdir -p /out/gates /out/build-evidence; \
    for pair in "indra_engine unit" "native native"; do \
      read -r pattern name <<< "$pair"; count=0; \
      for candidate in runtime/target/x86_64-unknown-linux-gnu/release/deps/"$pattern"-*; do \
        if [[ -f "$candidate" && -x "$candidate" ]]; then cp "$candidate" "/out/gates/$name"; count=$((count + 1)); fi; \
      done; \
      [[ "$count" = 1 ]]; \
    done; \
    cp runtime/target/x86_64-unknown-linux-gnu/release/examples/ocr /out/ocr; \
    cp runtime/target/x86_64-unknown-linux-gnu/release/examples/acceptance /out/engine-acceptance; \
    for binary in /out/ocr /out/engine-acceptance /out/gates/*; do \
      readelf -W -l "$binary" > "/out/build-evidence/$(basename "$binary").program-headers.txt"; \
      readelf -W -d "$binary" > "/out/build-evidence/$(basename "$binary").dynamic.txt"; \
      ! grep -q "INTERP" "/out/build-evidence/$(basename "$binary").program-headers.txt"; \
      ! grep -q "NEEDED" "/out/build-evidence/$(basename "$binary").dynamic.txt"; \
      sha256sum "$binary" >> /out/build-evidence/binaries.sha256; \
    done; \
    rustc -Vv > /out/build-evidence/rustc.txt; \
    cp -a /native/evidence/. /out/build-evidence/'

FROM scratch AS engine-acceptance
COPY --from=engine-build /out /
COPY --from=native-build /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY backend/tests/data/smoke.png backend/tests/data/two-pages.tiff /fixtures/
ENV SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt \
    OMP_NUM_THREADS=1 OMP_THREAD_LIMIT=1 OPENBLAS_NUM_THREADS=1 MKL_NUM_THREADS=1
USER 65532:65532
ENTRYPOINT ["/engine-acceptance"]

# The service links the same verified archives. Signal/HTTP tests execute the
# real production binary from a scratch container, including both thread caps.
FROM native-build AS server-build
WORKDIR /app
COPY runtime /app/runtime
COPY backend/tests/data /app/backend/tests/data
COPY scripts /app/scripts
COPY docker /app/docker
COPY docs /app/docs
COPY LICENSE THIRD_PARTY_NOTICES.md /app/
COPY licenses /app/licenses
ARG PROFILE=release
RUN /bin/bash -euo pipefail -c 'source /native/cargo-env.sh; \
    mkdir -p /out/gates /out/build-evidence /out/models; \
    cargo test --locked --profile "$PROFILE" --manifest-path runtime/Cargo.toml --target x86_64-unknown-linux-gnu -p indra-server --lib --no-run; \
    count=0; for file in runtime/target/x86_64-unknown-linux-gnu/"$PROFILE"/deps/indra_server-*; do \
      if [[ -f "$file" && -x "$file" ]]; then cp "$file" /out/gates/server-unit; count=$((count + 1)); fi; \
    done; [[ "$count" = 1 ]]; \
    cargo test --locked --profile "$PROFILE" --manifest-path runtime/Cargo.toml --target x86_64-unknown-linux-gnu -p indra-server --test lifecycle_native --no-run; \
    count=0; for file in runtime/target/x86_64-unknown-linux-gnu/"$PROFILE"/deps/lifecycle_native-*; do \
      if [[ -f "$file" && -x "$file" ]]; then cp "$file" /out/gates/server-native; count=$((count + 1)); fi; \
    done; [[ "$count" = 1 ]]; \
    cp runtime/target/x86_64-unknown-linux-gnu/"$PROFILE"/indra-server /out/indra-server; \
    for binary in /out/indra-server /out/gates/*; do \
      readelf -W -l "$binary" > "/out/build-evidence/$(basename "$binary").program-headers.txt"; \
      readelf -W -d "$binary" > "/out/build-evidence/$(basename "$binary").dynamic.txt"; \
      ! grep -q INTERP "/out/build-evidence/$(basename "$binary").program-headers.txt"; \
      ! grep -q NEEDED "/out/build-evidence/$(basename "$binary").dynamic.txt"; \
      sha256sum "$binary" >> /out/build-evidence/binaries.sha256; \
    done; \
    /out/indra-server check-config > /out/build-evidence/default-config.json; \
    /out/indra-server openapi > /out/build-evidence/openapi.json; \
    rustc -Vv > /out/build-evidence/rustc.txt; \
    cp -a /native/evidence/. /out/build-evidence/; \
    bash scripts/collect-runtime-materials.sh native /out/relink /out/indra-server'

FROM scratch AS server
LABEL org.opencontainers.image.title="Native Indra OCR service" \
      org.opencontainers.image.source="https://github.com/somehowchris/tika-indra-ocr-client"
COPY --from=server-build --chown=65532:65532 /out/indra-server /indra-server
COPY --from=server-build --chown=65532:65532 /out/relink /relink
COPY --from=server-build --chown=65532:65532 /out/models /models
COPY --from=native-build /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
ENV SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt \
    OMP_NUM_THREADS=1 OMP_THREAD_LIMIT=1 OPENBLAS_NUM_THREADS=1 MKL_NUM_THREADS=1
USER 65532:65532
ENTRYPOINT ["/indra-server"]

FROM server AS server-acceptance
COPY --from=server-build /out/gates /gates
COPY --from=server-build /out/build-evidence /build-evidence
