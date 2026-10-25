# Native compilation is a separate cached layer from application source.
# Every source revision, package snapshot and downloaded build wheel is pinned.
FROM rust:1.99.1-trixie@sha256:e259494b14672950a909b4c54527e19d29cae6ebf980672114c8bb85e5594e4a AS native-tools
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
FROM native-tools AS native-build
# Hosted x86-64 runners expose CPUs 0 and 1; callers on other cpusets override.
# This is a compiler-affinity restriction, not a benchmark CFS attestation.
ARG BUILD_CPUSET=0,1
RUN taskset --cpu-list "$BUILD_CPUSET" bash scripts/build-native-runtime.sh /native

# This export target contains build evidence, not a runnable OCR service.
FROM scratch AS native-evidence
COPY --from=native-build /native/evidence /evidence

# Application compilation is cached independently of the expensive archives.
# Fixed reproducibility metadata for C dependencies such as mimalloc. This is
# not the build time; source/run provenance is recorded separately. Keep this
# after the native delimiter so an existing verified SDK remains compatible.
FROM native-build AS application-build
ENV SOURCE_DATE_EPOCH=0
# Pinned cargo-auditable embeds the exact dependency tree in every shipped
# executable, so the signed SBOM names the Rust packages inside a scratch image.
# It is installed after the native delimiter, so the verified native SDK prefix
# is unchanged, and it never enters a runtime stage. The pinned Python of the
# builder extracts the xz archive; no extra package is needed.
ARG CARGO_AUDITABLE_VERSION=0.7.7
ARG CARGO_AUDITABLE_SHA256=4c8d3b08c3c34e9f2333367224dbbfb4aab68067698b43de59fe19aded3416dd
RUN curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 \
      --connect-timeout 15 --max-time 180 --retry 2 --retry-max-time 240 \
      "https://github.com/rust-secure-code/cargo-auditable/releases/download/v${CARGO_AUDITABLE_VERSION}/cargo-auditable-x86_64-unknown-linux-gnu.tar.xz" \
      --output /tmp/cargo-auditable.tar.xz \
    && printf '%s  /tmp/cargo-auditable.tar.xz\n' "$CARGO_AUDITABLE_SHA256" | sha256sum --check --status \
    && python3 -I -c 'import tarfile; tarfile.open("/tmp/cargo-auditable.tar.xz").extract("cargo-auditable-x86_64-unknown-linux-gnu/cargo-auditable", path="/tmp/cargo-auditable", filter="data")' \
    && install -m 0755 /tmp/cargo-auditable/cargo-auditable-x86_64-unknown-linux-gnu/cargo-auditable /usr/local/bin/cargo-auditable \
    && rm -r /tmp/cargo-auditable /tmp/cargo-auditable.tar.xz \
    && test -x /usr/local/bin/cargo-auditable

FROM application-build AS engine-build
WORKDIR /app
COPY scripts/select-cargo-artifact.py /build-tools/select-cargo-artifact.py
COPY Cargo.toml Cargo.lock /app/
COPY src /app/src
COPY tests /app/tests
COPY benches /app/benches
COPY runtime /app/runtime
COPY shared /app/shared
COPY backend/tests/data /app/backend/tests/data
COPY backend/models.lock.json /app/backend/models.lock.json
RUN /bin/bash -euo pipefail -c 'source /native/cargo-env.sh; \
    mkdir -p /out/gates /out/build-evidence; \
    cargo auditable test --locked --release --manifest-path runtime/Cargo.toml --target x86_64-unknown-linux-gnu -p indra-engine --lib --test native --test allocations --test continuous_plan --test request_allocations --no-run --message-format=json-render-diagnostics > /out/build-evidence/engine-tests.artifacts.jsonl; \
    cargo auditable build --locked --release --manifest-path runtime/Cargo.toml --target x86_64-unknown-linux-gnu -p indra-engine --examples --message-format=json-render-diagnostics > /out/build-evidence/engine-examples.artifacts.jsonl; \
    select_engine() { \
      python3 /build-tools/select-cargo-artifact.py --json "/out/build-evidence/$1.artifacts.jsonl" \
        --package-id "path+file:///app/runtime/crates/indra-engine#0.2.0" --manifest-path /app/runtime/crates/indra-engine/Cargo.toml --name "$2" --kind "$3" --test "$4" \
        --artifact-root /app/runtime/target/x86_64-unknown-linux-gnu/release --destination "/out/$5" \
        --receipt "/out/build-evidence/$6.selection.json"; \
    }; \
    select_engine engine-tests indra_engine lib true gates/unit unit; \
    select_engine engine-tests native test true gates/native native; \
    select_engine engine-tests allocations test true gates/allocations allocations; \
    select_engine engine-tests continuous_plan test true gates/continuous-plan continuous-plan; \
    select_engine engine-tests request_allocations test true gates/request-allocations request-allocations; \
    select_engine engine-examples ocr example false ocr ocr; \
    select_engine engine-examples acceptance example false engine-acceptance engine-acceptance; \
    for binary in /out/ocr /out/engine-acceptance /out/gates/*; do \
      readelf -W -l "$binary" > "/out/build-evidence/$(basename "$binary").program-headers.txt"; \
      readelf -W -d "$binary" > "/out/build-evidence/$(basename "$binary").dynamic.txt"; \
      if grep -q "INTERP" "/out/build-evidence/$(basename "$binary").program-headers.txt"; then echo "dynamic ELF interpreter: $binary" >&2; exit 1; fi; \
      if grep -q "NEEDED" "/out/build-evidence/$(basename "$binary").dynamic.txt"; then echo "dynamic ELF dependency: $binary" >&2; exit 1; fi; \
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
FROM application-build AS server-build
WORKDIR /app
COPY scripts/select-cargo-artifact.py /build-tools/select-cargo-artifact.py
COPY Cargo.toml Cargo.lock /app/
COPY src /app/src
COPY tests /app/tests
COPY benches /app/benches
COPY runtime /app/runtime
COPY shared /app/shared
COPY backend/tests/data /app/backend/tests/data
COPY backend/models.lock.json /app/backend/models.lock.json
ARG PROFILE=release
# The release workflow passes the commit SHA. Telemetry reports it in
# service.version and the Sentry release; empty keeps the crate version.
ARG SOURCE_REVISION=
RUN /bin/bash -euo pipefail -c 'source /native/cargo-env.sh; \
    if [[ -n "$SOURCE_REVISION" ]]; then [[ "$SOURCE_REVISION" =~ ^[0-9a-f]{40}$ ]] || { printf "SOURCE_REVISION must be a 40-hex commit: %s\n" "$SOURCE_REVISION" >&2; exit 1; }; export INDRA_SOURCE_REVISION="$SOURCE_REVISION"; fi; \
    mkdir -p /out/gates /out/build-evidence /out/models; \
    cargo auditable test --locked --profile "$PROFILE" --manifest-path runtime/Cargo.toml --target x86_64-unknown-linux-gnu -p indra-server --lib --no-run --message-format=json-render-diagnostics > /out/build-evidence/server-unit.artifacts.jsonl; \
    python3 /build-tools/select-cargo-artifact.py --json /out/build-evidence/server-unit.artifacts.jsonl \
      --package-id "path+file:///app/runtime/crates/indra-server#0.2.0" --manifest-path /app/runtime/crates/indra-server/Cargo.toml --name indra_server --kind lib --test true \
      --artifact-root "/app/runtime/target/x86_64-unknown-linux-gnu/$PROFILE" --destination /out/gates/server-unit \
      --receipt /out/build-evidence/server-unit.selection.json; \
    cargo auditable test --locked --profile "$PROFILE" --manifest-path runtime/Cargo.toml --target x86_64-unknown-linux-gnu -p indra-server --test lifecycle_native --test service_allocations --no-run --message-format=json-render-diagnostics > /out/build-evidence/server-native.artifacts.jsonl; \
    select_server() { \
      python3 /build-tools/select-cargo-artifact.py --json /out/build-evidence/server-native.artifacts.jsonl \
        --package-id "path+file:///app/runtime/crates/indra-server#0.2.0" --manifest-path /app/runtime/crates/indra-server/Cargo.toml --name "$1" --kind "$2" --test "$3" \
        --artifact-root "/app/runtime/target/x86_64-unknown-linux-gnu/$PROFILE" --destination "/out/$4" \
        --receipt "/out/build-evidence/$5.selection.json"; \
    }; \
    select_server lifecycle_native test true gates/server-native server-native; \
    select_server service_allocations test true gates/server-allocations server-allocations; \
    select_server indra-server bin false indra-server indra-server; \
    for binary in /out/indra-server /out/gates/*; do \
      readelf -W -l "$binary" > "/out/build-evidence/$(basename "$binary").program-headers.txt"; \
      readelf -W -d "$binary" > "/out/build-evidence/$(basename "$binary").dynamic.txt"; \
      if grep -q INTERP "/out/build-evidence/$(basename "$binary").program-headers.txt"; then echo "dynamic ELF interpreter: $binary" >&2; exit 1; fi; \
      if grep -q NEEDED "/out/build-evidence/$(basename "$binary").dynamic.txt"; then echo "dynamic ELF dependency: $binary" >&2; exit 1; fi; \
      sha256sum "$binary" >> /out/build-evidence/binaries.sha256; \
    done; \
    /out/indra-server check-config > /out/build-evidence/default-config-check.txt; \
    /out/indra-server openapi > /out/build-evidence/openapi.json; \
    rustc -Vv > /out/build-evidence/rustc.txt; \
    cp -a /native/evidence/. /out/build-evidence/'

# Source/vendor/relink collection is a leaf of the exact executable build.
# This is an independent candidate; equivalence and speed remain gated by WP0.
FROM server-build AS server-materials
COPY scripts /app/scripts
COPY docker /app/docker
COPY docs /app/docs
COPY LICENSE THIRD_PARTY_NOTICES.md /app/
COPY licenses /app/licenses
RUN bash scripts/collect-runtime-materials.sh native /out/relink /out/indra-server

FROM scratch AS server
LABEL org.opencontainers.image.title="Native Indra OCR service" \
      org.opencontainers.image.source="https://github.com/somehowchris/tika-indra-ocr-client"
COPY --from=server-build --chown=65532:65532 /out/indra-server /indra-server
COPY --from=server-materials --chown=65532:65532 /out/relink /relink
COPY --from=server-build --chown=65532:65532 /out/models /models
COPY --from=native-build /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
ENV SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt \
    OMP_NUM_THREADS=1 OMP_THREAD_LIMIT=1 OPENBLAS_NUM_THREADS=1 MKL_NUM_THREADS=1
USER 65532:65532
ENTRYPOINT ["/indra-server"]

FROM server AS server-acceptance
COPY --from=server-build /out/gates /gates
COPY --from=server-build /out/build-evidence /build-evidence

# A diagnostic export with unstripped Rust DWARF and complete relink materials.
# It never publishes an image or changes the production release profile.
FROM application-build AS engine-profile-build
WORKDIR /app
RUN apt-get update \
    && apt-get install -y --no-install-recommends jq \
    && rm -rf /var/lib/apt/lists/*
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
ARG SOURCE_COMMIT
ARG PROFILE_LTO=thin
RUN /bin/bash -euo pipefail -c 'source /native/cargo-env.sh; \
    [[ "$SOURCE_COMMIT" =~ ^[0-9a-f]{40}$ ]]; \
    [[ "$PROFILE_LTO" = thin || "$PROFILE_LTO" = fat ]]; \
    export CARGO_PROFILE_PROFILING_LTO="$PROFILE_LTO"; \
    mkdir -p /profile-out/gates /profile-out/build-evidence /profile-out/fixtures; \
    copy_one() { \
      local directory=$1 pattern=$2 destination=$3 candidate count=0; \
      for candidate in "$directory"/"$pattern"-*; do \
        if [[ -f "$candidate" && -x "$candidate" ]]; then cp "$candidate" "$destination"; count=$((count + 1)); fi; \
      done; [[ "$count" = 1 ]]; \
    }; \
    deps=runtime/target/x86_64-unknown-linux-gnu/profiling/deps; \
    examples=runtime/target/x86_64-unknown-linux-gnu/profiling/examples; \
    cargo test --locked --profile profiling --manifest-path runtime/Cargo.toml --target x86_64-unknown-linux-gnu \
      -p indra-engine --lib --test native --test allocations --test perf_control --no-run; \
    copy_one "$deps" indra_engine /profile-out/gates/unit; \
    copy_one "$deps" native /profile-out/gates/native; \
    copy_one "$deps" allocations /profile-out/gates/allocations; \
    copy_one "$deps" perf_control /profile-out/gates/perf-control; \
    cargo test --locked --profile profiling --manifest-path runtime/Cargo.toml --target x86_64-unknown-linux-gnu \
      -p indra-engine --example ocr --no-run; \
    copy_one "$examples" ocr /profile-out/gates/ocr-cli; \
    cargo build --locked --profile profiling --manifest-path runtime/Cargo.toml --target x86_64-unknown-linux-gnu \
      -p indra-engine --examples; \
    cp "$examples/ocr" /profile-out/ocr-profiling; \
    cp "$examples/acceptance" /profile-out/engine-acceptance; \
    for pair in "indra-engine postprocessing" "indra-core control_path" "indra-models verification" "indra-telemetry propagation"; do \
      read -r crate bench <<< "$pair"; \
      cargo bench --locked --profile profiling --manifest-path runtime/Cargo.toml --target x86_64-unknown-linux-gnu \
        -p "$crate" --bench "$bench" --no-run; \
      copy_one "$deps" "$bench" "/profile-out/$bench"; \
    done; \
    for pair in "indra-core core-allocations" "indra-telemetry telemetry-allocations"; do \
      read -r crate name <<< "$pair"; \
      cargo test --locked --profile profiling --manifest-path runtime/Cargo.toml --target x86_64-unknown-linux-gnu \
        -p "$crate" --test allocations --no-run --message-format=json-render-diagnostics \
        > "/profile-out/build-evidence/$name.artifacts.jsonl"; \
      binary=$(jq -ser "[.[] | select(.reason == \"compiler-artifact\" and .target.name == \"allocations\" and .profile.test == true and .executable != null)] | if length == 1 then .[0].executable else error(\"ambiguous allocation executable\") end" "/profile-out/build-evidence/$name.artifacts.jsonl"); \
      [[ -f "$binary" && -x "$binary" ]]; cp "$binary" "/profile-out/gates/$name"; \
    done; \
    cargo build --locked --profile profiling --manifest-path runtime/Cargo.toml --target x86_64-unknown-linux-gnu \
      -p indra-models --example allocations; \
    cp "$examples/allocations" /profile-out/gates/model-allocations; \
    cp backend/tests/data/two-pages.tiff runtime/crates/indra-engine/tests/data/math.png \
      runtime/crates/indra-engine/tests/data/two-pages.expected.txt \
      runtime/crates/indra-engine/tests/data/math-page.expected.txt /profile-out/fixtures/; \
    cp runtime/validation/native-profile-protocol.json runtime/validation/native-ablation-protocol.json /profile-out/; \
    for binary in /profile-out/ocr-profiling /profile-out/engine-acceptance /profile-out/postprocessing \
      /profile-out/control_path /profile-out/verification /profile-out/propagation /profile-out/gates/*; do \
      readelf -W -l "$binary" > "/profile-out/build-evidence/$(basename "$binary").program-headers.txt"; \
      readelf -W -d "$binary" > "/profile-out/build-evidence/$(basename "$binary").dynamic.txt"; \
      if grep -q INTERP "/profile-out/build-evidence/$(basename "$binary").program-headers.txt"; then echo "dynamic ELF interpreter: $binary" >&2; exit 1; fi; \
      if grep -q NEEDED "/profile-out/build-evidence/$(basename "$binary").dynamic.txt"; then echo "dynamic ELF dependency: $binary" >&2; exit 1; fi; \
    done; \
    readelf -W -S /profile-out/ocr-profiling > /profile-out/build-evidence/ocr-profiling.sections.txt; \
    grep -q "[.]debug_line" /profile-out/build-evidence/ocr-profiling.sections.txt; \
    rustc -Vv > /profile-out/build-evidence/rustc.txt; \
    cp runtime/Cargo.toml runtime/Cargo.lock /profile-out/build-evidence/; \
    cp -a /native/evidence/. /profile-out/build-evidence/; \
    printf "source_commit=%s\ncargo_profile=profiling\nlto=%s\n" "$SOURCE_COMMIT" "$PROFILE_LTO" > /profile-out/build-evidence/execution-source.txt; \
    bash scripts/collect-runtime-materials.sh native /profile-out/relink /profile-out/ocr-profiling; \
    cd /profile-out; \
    find . -type f ! -name artifacts.sha256 -print0 | sort -z | xargs -0 -r sha256sum > artifacts.sha256; \
    sha256sum --check --status artifacts.sha256'

FROM scratch AS engine-profile-export
COPY --from=engine-profile-build /profile-out /
