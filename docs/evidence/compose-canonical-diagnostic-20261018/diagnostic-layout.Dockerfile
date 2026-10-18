# Diagnostic layout correction only. Native binary/gates/fixtures are inherited unchanged.
FROM indra-native:canonical-issue13
COPY --chown=65532:65532 models /models
