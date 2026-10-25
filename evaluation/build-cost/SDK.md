# Local immutable native SDK experiment

`scripts/native-sdk.py` packages the original native build's link closure into a
scratch artifact and generates a consumer that restores it into the pinned
`native-tools` stage. The original `docker/native.Dockerfile` remains the explicit
source-build route. The consumer never falls back to compilation.

Three identities remain distinct:

- Recipe: exact native Docker prefix, build scripts/locks, packaging/notice code,
  platform, actual Debian snapshot/compiler-affinity build arguments, resolved
  Debian package inventory, compiler/tool executable hashes
  and versions, and complete native configuration evidence. Application Git
  commit is producer provenance, not a dependency cache key.
- Content: every extracted file/directory path, type, mode, size and SHA256.
  This covers all original static archives, OpenCV headers, Cargo link
  environment, complete native evidence, glibc source archives and all 31 notice
  source paths consumed by the unchanged material collector. The notice paths
  come directly from that collector's existing here-document.
- OCI distribution: immutable index/platform-manifest/config/layer digests and
  lengths. The reader hashes every referenced blob, decompresses the single
  scratch COPY layer, checks its uncompressed diff-id and materializes the full
  bounded content inventory. A cache manifest or valid old blob alone is
  insufficient.

Synthetic integrity tests and generated Docker graph checks pass. A tiny
synthetic OCI round-trip established transport support first; a subsequent
[actual native SDK receipt](evidence/native-sdk-2026-10-18/summary.json) now binds
400 files, 557 closure entries and all 31 required source notice paths. Every
referenced OCI blob and gzip diff-id was read and verified, and the complete
native closure was materialized. An actual shared-builder consumer restored and
admitted exact compiler/package/configuration/file/mode/archive evidence. The
producer reused its native build vertex from cache. Its explicitly mode-qualified
source transport and unsuccessful development attempts remain retained.

This proves actual packaging and shared-builder admission, not fresh-builder
linking, executable byte equivalence, lifecycle acceptance or speedup. Those gates
remain pending. No image was published and signing trust is unchanged.

Use a committed, clean source context. Keep every attempt in a new absolute
output directory. Preserve original builder/host/resource identity, stdout,
stderr and metadata. Reuse the existing native builder for the first producer;
creating a fresh producer builder would deliberately recompile its native input.
The following commands run compilation only when explicitly invoked:

```sh
SDK_ATTEMPT=/absolute/new-sdk-attempt
mkdir "$SDK_ATTEMPT"
python3 -B scripts/native-sdk.py render-producer \
  --source docker/native.Dockerfile --output "$SDK_ATTEMPT/producer.Dockerfile"
python3 -B scripts/native-sdk.py render-consumer \
  --source docker/native.Dockerfile --output "$SDK_ATTEMPT/consumer.Dockerfile"
docker buildx build --builder EXISTING_NATIVE_BUILDER --platform linux/amd64 \
  --file "$SDK_ATTEMPT/producer.Dockerfile" --target native-sdk \
  --build-arg SOURCE_COMMIT="$(git rev-parse HEAD)" --build-arg BUILD_CPUSET=0,1 \
  --provenance=false --sbom=false \
  --output "type=oci,dest=$SDK_ATTEMPT/oci,tar=false,compression=gzip" \
  --metadata-file "$SDK_ATTEMPT/producer-metadata.json" .
SDK_DIGEST=$(jq -er '."containerimage.digest"' "$SDK_ATTEMPT/producer-metadata.json")
python3 -B scripts/native-sdk.py materialize-oci \
  --layout "$SDK_ATTEMPT/oci" --digest "$SDK_DIGEST" --inputs "$PWD" \
  --output "$SDK_ATTEMPT/materialized"
SDK_RECIPE=$(jq -er '.recipe_sha256' "$SDK_ATTEMPT/materialized/materialization.json")
SDK_CONTENT=$(jq -er '.content_sha256' "$SDK_ATTEMPT/materialized/materialization.json")
```

`--provenance=false --sbom=false` applies only to this unsigned, local diagnostic
scratch artifact and keeps a single platform image. It does not alter production
release attestations. A registry producer and signature admission require their
own existing trusted release integration before commercial distribution.

Now create an independently named empty builder for the **consumer**, recording
its exact BuildKit image digest/version and resource settings. Do not import
native cache state. Supply its name explicitly; never change the user's default
builder. Export and retain actual binaries from all required engine/server
targets, for example:

```sh
docker buildx build --builder FRESH_CONSUMER_BUILDER --platform linux/amd64 \
  --file "$SDK_ATTEMPT/consumer.Dockerfile" --target server-acceptance \
  --build-context "sdk-input=oci-layout://$SDK_ATTEMPT/oci@$SDK_DIGEST" \
  --build-arg "NATIVE_SDK_RECIPE_SHA256=$SDK_RECIPE" \
  --build-arg "NATIVE_SDK_CONTENT_SHA256=$SDK_CONTENT" \
  --metadata-file "$SDK_ATTEMPT/consumer-metadata.json" \
  --output "type=oci,dest=$SDK_ATTEMPT/consumer-oci,tar=false" .
```

For a producer using nondefault snapshot or affinity arguments, pass the same
recorded build-argument values to admission; compiler affinity is provenance,
not a runtime CPU affinity requirement.

The fresh consumer still provisions the pinned native toolchain. It restores
only `/native` and `/glibc-source`, verifies the expected manifest hashes and
actual compiler/package/native configuration, checks every restored mode/hash
and rechecks original native evidence/archive manifests before Rust compilation.
`server-materials` runs the original collector against the preserved notice
paths. Missing or changed inputs fail. Byte equality of every exported
executable and the static, lifecycle and material inventory gates remain required.
A source rebuild is a separate operator decision with separately recorded costs.

Repeat with missing, truncated and corrupt copied OCI blobs, a wrong recipe,
content hash, platform and toolchain, retaining each failure. Never edit the
canonical successful layout. Only after these gates and the frozen paired cost
schedule pass can SDK restore time be compared with source compilation.

[Docker documents immutable local OCI named contexts](https://docs.docker.com/reference/cli/docker/buildx/build/#use-an-oci-layout-directory-as-build-context)
and the [OCI exporter parameters](https://docs.docker.com/build/exporters/oci-docker/),
including `tar=false`. Exporter support depends on the actual driver/image store;
the retained synthetic local round-trip verifies this session's support.
Cargo chef remains the next separate experiment, preserving root/runtime/shared
workspace layout and binding cooked dependency layers to the admitted SDK.

[The real 2026-10-18 local SDK receipt](evidence/native-sdk-2026-10-18/summary.json)
now records successful packaging from immutable source `2a16bb3`, complete OCI
reading/materialization and actual pinned-toolchain consumer admission. The
400 materialized files include SDK metadata; the closure has 557 file/directory
entries. The native build vertex was cached. Source transport required explicitly
reproducing the completed local worktree's three native-input file modes
(664/775 rather than archive 644/755); original contexts and the cancelled miss
remain intact. An accidental short source-commit invocation was also rejected
and retained. These development attempts are not performance comparisons.

This consumer used the existing shared builder. It establishes neither empty
builder state nor executable-role equivalence. `--no-cache` does not prove a fresh
builder. Use an already authorized empty daemon or hosted runner for the separate
fresh-state gate; do not create privileged BuildKit containers, mount host sockets
into containers, or prune shared state to manufacture that claim.

## Fresh hosted proof

The trusted release's separate `native-build-proof` job now schedules these
actual-input controls through `scripts/verify-sdk-invalidation.py --index INDEX
--inputs FRESH_PROOF_DIRECTORY --output NEW_DIRECTORY`. It runs only after the
positive SDK/Chef restores and runtime gates. Three sequential independent OCI copies
exercise missing, one-byte-truncated and same-size-corrupt actual layer bytes.
The original descriptor, exact mutation rule, before/after hashes, full case
inventory and raw failure outputs remain retained. Only after an exit-1 typed
rejection is each owned copy removed; unexpected cases remain for diagnosis.
The expected cases can be recreated from the canonical digest and retained
rules; their redundant unchanged blobs are not archived three times. The
original signed layout is re-inventoried unchanged. Seven consumer cases
check the expected recipe/content identities, a copied GCC executable with one
appended byte, a changed SDK source script, changed restored link environment,
wrong verifier platform, and wrong compiler-affinity recipe argument. These are
explicit diagnostic recipes and unsigned deliberate mutations, not admissible
replacement SDKs. They stop at the real consumer's `native-build` verifier and
require exit status 1 and its specific rejection. Success, timeout, signal
termination (including 130/137/143) and unrelated errors fail the control. These
cases never compile native sources or application code. They remain distinct
from the five Chef-key controls and same-path C/Rust linkage fixture. Real execution remains pending until a corrected signed producer exists.

Fresh proof and paired cost material receipts independently bind every required
original source file to the actual clean Git context (or mutated cost context),
check nonempty vendor/configuration and all six exact outer documentation/license copies,
and compare all 31 notice mappings with their actual SDK source bytes. Full
material equality remains an additional check, rather than a substitute for that
completeness and currency evidence: incomplete-but-equal or stale-but-rehashed
bundles fail. All 18 material-exporting server targets among the 36 timed builds
run this check against the actual cell context, including its source/doc edit,
after build timing and before duplicate exports are deleted. Those checks are
outside the reported build timing; their receipts remain. No production lock is
enabled by these tools.
