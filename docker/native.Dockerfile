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
