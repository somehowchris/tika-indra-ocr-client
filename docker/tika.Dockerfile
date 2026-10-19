ARG BRIDGE_IMAGE=indra-bridge:dev
ARG GATEWAY_IMAGE=indra-gateway:dev
ARG DEBIAN_IMAGE=debian:trixie-slim@sha256:a29215f6a35e51e22adffa17f89e9d2ef06214e64a2bad10d765c46aea49f11f
ARG JAVA_IMAGE=gcr.io/distroless/java21-debian13:nonroot@sha256:51c242e7ddaba3666d47115a76a19cfa5076b2ad3ed6077b30d7bac689d98fa1

FROM ${BRIDGE_IMAGE} AS bridge
FROM ${GATEWAY_IMAGE} AS gateway
FROM ${DEBIAN_IMAGE} AS trace-build
ARG DEBIAN_SNAPSHOT=20261017T000000Z
RUN rm -f /etc/apt/sources.list.d/debian.sources \
    && printf 'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/%s trixie main\n' "$DEBIAN_SNAPSHOT" > /etc/apt/sources.list \
    && apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates curl patch openjdk-21-jdk-headless \
    && rm -rf /var/lib/apt/lists/*
ARG TIKA_VERSION=3.3.2
ARG TIKA_SHA512=fb1f2fe57ac458b09d44d41d816f582e1d2fc93488acff6275caf414d8d5ef94e42166edc0b488dc2fb6ef3aa21fab62b107c43b9060385ff6d675e393c2c9e9
RUN curl --fail --show-error --location --retry 3 \
      "https://archive.apache.org/dist/tika/${TIKA_VERSION}/tika-server-standard-${TIKA_VERSION}.jar" \
      --output /tika-server.jar \
    && printf '%s  /tika-server.jar\n' "$TIKA_SHA512" | sha512sum --check --status

WORKDIR /app
COPY tika-trace /app/tika-trace
COPY shared/trace-context/cases.tsv /app/shared/trace-context/cases.tsv
COPY scripts/build-tika-trace.sh /app/scripts/build-tika-trace.sh
RUN bash scripts/build-tika-trace.sh /tika-server.jar /tika-traced

# Export executable Java tests and exact source materials without runtime tools.
FROM scratch AS trace-evidence
COPY --from=trace-build /tika-traced /tika-traced

FROM ${JAVA_IMAGE} AS runtime
LABEL org.opencontainers.image.title="Apache Tika 3.3.2 with request-scoped trace hooks, native OCR admission and drain" \
      org.opencontainers.image.source="https://github.com/somehowchris/tika-indra-ocr-client"
COPY --from=trace-build --chown=65532:65532 /tika-traced /opt/tika
COPY --from=bridge --chown=65532:65532 --chmod=0555 /indra-tesseract /opt/indra/tesseract
COPY --from=gateway --chown=65532:65532 --chmod=0555 /indra-gateway /indra-gateway
COPY --from=bridge --chown=65532:65532 /relink /relink/bridge
COPY --from=gateway --chown=65532:65532 /relink /relink/gateway
COPY --from=gateway /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --chown=65532:65532 deploy/base/tika-config.xml /etc/tika/tika-config.xml
ENV INDRA_ADDR=127.0.0.1:8081 \
    INDRA_TIMEOUT_SECONDS=70 \
    INDRA_CONNECT_TIMEOUT_MS=1000 \
    XDG_CACHE_HOME=/tmp/.cache \
    SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt
USER 65532:65532
WORKDIR /tmp
RUN ["/opt/indra/tesseract", "--version"]
RUN ["/indra-gateway", "check-config"]
EXPOSE 9998
# Optional OCR dependency probing belongs to deployment args, so the identical
# gateway image also supports the Tesseract comparison without an OCR server.
ENTRYPOINT ["/indra-gateway"]
CMD ["serve"]
