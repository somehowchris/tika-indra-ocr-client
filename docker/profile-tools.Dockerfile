# Diagnostic builder only; never published as a serving runtime.
ARG NATIVE_TOOLS_IMAGE=indra-native-tools:issue13
FROM ${NATIVE_TOOLS_IMAGE} AS profile-tools
RUN apt-get update \
    && apt-get install -y --no-install-recommends linux-perf librsvg2-bin jq \
    && rm -rf /var/lib/apt/lists/*
RUN git clone https://github.com/jonhoo/inferno.git /inferno \
    && git -C /inferno checkout 929be68e19a27da062623c6da5255e8b96faebbc \
    && test "$(git -C /inferno rev-parse HEAD)" = 929be68e19a27da062623c6da5255e8b96faebbc \
    && RUSTFLAGS=-Ctarget-feature=+crt-static cargo install --locked --path /inferno \
       --target x86_64-unknown-linux-gnu --root /profile-tools --no-default-features --features cli \
       --bin inferno-collapse-perf --bin inferno-flamegraph --jobs 2
ENV PERF=/usr/bin/perf \
    INFERNO_COLLAPSE=/profile-tools/bin/inferno-collapse-perf \
    INFERNO_FLAMEGRAPH=/profile-tools/bin/inferno-flamegraph

COPY docker/profile-tools.Dockerfile /profile-tools/build-recipe.Dockerfile
RUN dpkg-query -W > /profile-tools/packages.tsv \
    && git -C /inferno rev-parse HEAD > /profile-tools/inferno-source.txt \
    && cp /inferno/Cargo.lock /profile-tools/inferno.Cargo.lock \
    && cd / \
    && sha256sum usr/bin/perf usr/bin/rsvg-convert \
       profile-tools/bin/inferno-collapse-perf profile-tools/bin/inferno-flamegraph \
       profile-tools/packages.tsv profile-tools/inferno-source.txt \
       profile-tools/inferno.Cargo.lock profile-tools/build-recipe.Dockerfile \
       > /profile-tools/build-identities.sha256
