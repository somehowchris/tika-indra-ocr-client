The optional SDK/Chef action creates a generated Dockerfile under `docker/` to
retain the existing provenance path contract. Git ignoring that file alone does
not remove it from Docker's `COPY docker` input. Without the exact Docker-ignore
entry, the release source bundle would contain an ephemeral recipe absent from
a fresh Git checkout, defeating complete-material equivalence.

Actual BuildKit proof at source `3624337a0bcc253fd6b162389bf3065458e908dc`
uses the repository's complete Docker-ignore bytes and explicitly selects that
ignored Dockerfile. Its scratch target copies the context. The required original
source survives; the generated recipe is absent from the exported context.
The actual recipe, context, command, logs, metadata and receipt are retained.
No native compilation, OCR, runtime measurement or speedup claim is involved.
The rendered production recipe and its hash remain separately retained in the
native-input and release evidence; this exclusion does not discard that evidence.

From this directory, reproduce into a new output using Docker Buildx:

```sh
sha256sum --check --strict SHA256SUMS
docker buildx build --platform linux/amd64 \
  --file context/docker/native-inputs.generated.Dockerfile \
  --target context-proof --provenance=false --sbom=false \
  --output type=local,dest=/absolute/new-context-proof context
test -f /absolute/new-context-proof/context/keep.txt
test ! -e /absolute/new-context-proof/context/docker/native-inputs.generated.Dockerfile
```
