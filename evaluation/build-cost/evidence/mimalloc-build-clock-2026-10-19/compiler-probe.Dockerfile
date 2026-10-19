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
# Mandatory immutable named context: sdk-input=oci-layout:///...@sha256:...
# An absent/incompatible SDK fails; original source Dockerfile remains separate.
FROM sdk-input AS native-sdk-input
FROM native-tools AS native-build
ARG TARGETPLATFORM
ARG BUILD_CPUSET=0,1
ARG NATIVE_SDK_RECIPE_SHA256
ARG NATIVE_SDK_CONTENT_SHA256
COPY docker/native.Dockerfile /sdk-input/docker/native.Dockerfile
COPY scripts/native-sdk.py scripts/native-build-tools.lock scripts/build-native-runtime.sh scripts/record-native-runtime.sh scripts/collect-runtime-materials.sh /sdk-input/scripts/
COPY --from=native-sdk-input /sdk/rootfs/native /native
COPY --from=native-sdk-input /sdk/rootfs/glibc-source /glibc-source
COPY --from=native-sdk-input /sdk/recipe.json /sdk/content.json /sdk-admission/
RUN python3 /sdk-input/scripts/native-sdk.py verify --inputs /sdk-input --sdk /sdk-admission --rootfs / --platform "$TARGETPLATFORM" --recipe-sha256 "$NATIVE_SDK_RECIPE_SHA256" --content-sha256 "$NATIVE_SDK_CONTENT_SHA256" --debian-snapshot "$DEBIAN_SNAPSHOT" --build-cpuset "$BUILD_CPUSET"

# This export target contains build evidence, not a runnable OCR service.
FROM scratch AS native-evidence
COPY --from=native-build /native/evidence /evidence

FROM native-build AS mimalloc-clock-proof
COPY mimalloc /probe/mimalloc
RUN /bin/bash -euo pipefail -c 'mkdir /clock-proof; cd /probe; \
    gcc --version > /clock-proof/gcc-version.txt; \
    sha256sum "$(readlink -f "$(command -v gcc)")" > /clock-proof/gcc.sha256; \
    compile() { gcc -O3 -ffunction-sections -fdata-sections -fPIC -m64 -ftls-model=initial-exec -Wno-error=date-time -DMI_DEBUG=0 -DMI_BUILD_RELEASE -DNDEBUG -I mimalloc/include -I mimalloc/src -c mimalloc/src/static.c -o "/clock-proof/$1.o"; }; \
    unset SOURCE_DATE_EPOCH; compile clock-first; sleep 2; compile clock-second; \
    export SOURCE_DATE_EPOCH=0; compile epoch-first; sleep 2; compile epoch-second; \
    sha256sum /clock-proof/*.o > /clock-proof/objects.sha256; \
    ! cmp -s /clock-proof/clock-first.o /clock-proof/clock-second.o; \
    cmp /clock-proof/epoch-first.o /clock-proof/epoch-second.o; \
    for file in /clock-proof/*.o; do strings "$file" | grep -E "^[0-9]{2}:[0-9]{2}:[0-9]{2}$|^Jan  1 1970$" > "$file.dates.txt"; done'
FROM scratch AS mimalloc-clock-export
COPY --from=mimalloc-clock-proof /clock-proof /
