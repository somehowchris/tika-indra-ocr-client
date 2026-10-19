# Only for acceptance: assemble exact production executable/JAR identities in
# one cgroup. No compiler, shell, Docker client or credentials enter this image.
ARG TIKA_IMAGE=tika-indra:dev
ARG NATIVE_IMAGE=indra-native:dev
FROM ${NATIVE_IMAGE} AS native
FROM ${TIKA_IMAGE} AS trace-acceptance
COPY --from=native --chown=65532:65532 /indra-server /indra-server
COPY --from=trace-helper --chown=65532:65532 /indra-trace-acceptance /indra-trace-acceptance
ENTRYPOINT ["/indra-trace-acceptance"]
CMD ["--gateway", "/indra-gateway", "--java", "/usr/bin/java", "--jar", "/opt/tika/tika-server.jar", "--bridge", "/opt/indra/tesseract", "--native", "/indra-server", "--models", "/models", "--evidence", "/evidence/run"]
