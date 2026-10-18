ARG TIKA_IMAGE=tika-indra:dev
ARG DEBIAN_IMAGE=debian:trixie-slim@sha256:a29215f6a35e51e22adffa17f89e9d2ef06214e64a2bad10d765c46aea49f11f

FROM ${DEBIAN_IMAGE} AS native
ARG DEBIAN_SNAPSHOT=20261017T000000Z
RUN rm -f /etc/apt/sources.list.d/debian.sources \
    && printf 'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/%s trixie main\n' "$DEBIAN_SNAPSHOT" > /etc/apt/sources.list \
    && apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates curl tesseract-ocr \
    && rm -rf /var/lib/apt/lists/*
ARG TESSDATA_BEST_REV=e12c65a915945e4c28e237a9b52bc4a8f39a0cec
ARG TESSDATA_BEST_ENG_SHA256=8280aed0782fe27257a68ea10fe7ef324ca0f8d85bd2fd145d1c2b560bcb66ba
COPY --chmod=0555 docker/copy-elf-dependencies.sh /usr/local/bin/copy-elf-dependencies
RUN mkdir -p /out/usr/bin /out/opt/tessdata/configs \
    && cp /usr/bin/tesseract /out/usr/bin/ \
    && /usr/local/bin/copy-elf-dependencies /usr/bin/tesseract /out \
    && cp /usr/share/tesseract-ocr/5/tessdata/configs/txt /out/opt/tessdata/configs/txt \
    && curl --fail --show-error --location --retry 3 \
      "https://raw.githubusercontent.com/tesseract-ocr/tessdata_best/${TESSDATA_BEST_REV}/eng.traineddata" \
      --output /out/opt/tessdata/eng.traineddata \
    && printf '%s  /out/opt/tessdata/eng.traineddata\n' "$TESSDATA_BEST_ENG_SHA256" | sha256sum --check --status \
    && tesseract --version > /out/opt/tessdata/engine-version.txt 2>&1 \
    && dpkg-query -W -f='${binary:Package}\t${Version}\n' > /out/opt/tessdata/build-packages.tsv \
    && cp /usr/share/common-licenses/Apache-2.0 /out/opt/tessdata/LICENSE

FROM ${TIKA_IMAGE} AS runtime
LABEL org.opencontainers.image.title="Apache Tika 3.3.2 Tesseract tessdata_best comparison"
COPY --from=native /out/ /
COPY --chown=65532:65532 deploy/tika-tesseract.xml /etc/tika/tika-config.xml
ENV OMP_THREAD_LIMIT=1 \
    OMP_NUM_THREADS=1 \
    TESSDATA_PREFIX=/opt/tessdata
USER 65532:65532
RUN ["/usr/bin/tesseract", "--version"]
