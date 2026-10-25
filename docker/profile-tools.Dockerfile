# Diagnostic builder only; never published as a serving runtime.
ARG NATIVE_TOOLS_IMAGE=indra-native-tools:issue13
FROM ${NATIVE_TOOLS_IMAGE} AS profile-tools
RUN apt-get update \
    && apt-get install -y --no-install-recommends linux-perf librsvg2-bin jq \
    && rm -rf /var/lib/apt/lists/*
COPY scripts/patch-inferno-profile.sh /indra-profile/patch-inferno-profile.sh
COPY scripts/patches/inferno-anonymous-namespace.patch /indra-profile/patches/inferno-anonymous-namespace.patch
RUN git clone https://github.com/jonhoo/inferno.git /inferno-upstream \
    && git -C /inferno-upstream checkout 929be68e19a27da062623c6da5255e8b96faebbc \
    && test "$(git -C /inferno-upstream rev-parse HEAD)" = 929be68e19a27da062623c6da5255e8b96faebbc \
    && mkdir -p /inferno /profile-tools/source-evidence \
    && git -C /inferno-upstream archive 929be68e19a27da062623c6da5255e8b96faebbc > /profile-tools/source-evidence/inferno-source.tar \
    && tar -xf /profile-tools/source-evidence/inferno-source.tar -C /inferno \
    && bash /indra-profile/patch-inferno-profile.sh /inferno \
       /profile-tools/source-evidence/inferno-source.tar /profile-tools/source-evidence \
    && RUSTFLAGS=-Ctarget-feature=+crt-static cargo install --locked --path /inferno \
       --target x86_64-unknown-linux-gnu --root /profile-tools --no-default-features --features cli \
       --bin inferno-collapse-perf --bin inferno-flamegraph --jobs 2
# Rust v0 symbol demangler: rustfilt 0.2.2 with its packaged lock
# (rustc-demangle 0.1.28); crates.io verifies the crate checksum.
RUN RUSTFLAGS=-Ctarget-feature=+crt-static cargo install --locked rustfilt --version =0.2.2 \
       --target x86_64-unknown-linux-gnu --root /profile-tools --jobs 2 \
    && /profile-tools/bin/rustfilt --version > /profile-tools/rustfilt-version.txt
ENV PERF=/usr/bin/perf \
    INFERNO_COLLAPSE=/profile-tools/bin/inferno-collapse-perf \
    INFERNO_FLAMEGRAPH=/profile-tools/bin/inferno-flamegraph \
    RUSTFILT=/profile-tools/bin/rustfilt

COPY docker/profile-tools.Dockerfile /profile-tools/build-recipe.Dockerfile
RUN dpkg-query -W > /profile-tools/packages.tsv \
    && git -C /inferno-upstream rev-parse HEAD > /profile-tools/inferno-source.txt \
    && cp /inferno/Cargo.lock /profile-tools/inferno.Cargo.lock \
    && cd / \
    && sha256sum usr/bin/perf usr/bin/rsvg-convert \
       profile-tools/bin/inferno-collapse-perf profile-tools/bin/inferno-flamegraph \
       profile-tools/bin/rustfilt profile-tools/rustfilt-version.txt \
       profile-tools/packages.tsv profile-tools/inferno-source.txt \
       profile-tools/inferno.Cargo.lock profile-tools/build-recipe.Dockerfile \
       profile-tools/source-evidence/* \
       > /profile-tools/build-identities.sha256
