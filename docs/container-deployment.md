# Run native Indra with Tika 3.3.2

The supplied stack exposes Tika on port 9998. A Rust gateway owns that listener
and the forked Java process group. Native Indra listens on loopback port 8081;
Java listens on loopback port 9999. The image and JAR pins are recorded in
[dependencies.lock.json](../deploy/dependencies.lock.json), the
[native build lock](../scripts/native-build-tools.lock) and the Dockerfiles.

The Rust bridge, gateway and OCR service are static GNU executables. Their final
images use scratch; Tika uses the pinned distroless Java 21 image. No serving
container needs a shell. The Tesseract comparison adds its Debian native shared
libraries to the same distroless Tika image. Static Rust linkage does not imply
that Java or Tesseract is static.

## Build and start locally

Use a Linux amd64 Docker environment with Buildx and Compose. Run these commands
from the repository root:

```sh
docker buildx build --load --platform linux/amd64 \
  -f docker/bridge.Dockerfile -t indra-bridge:dev .
docker buildx build --load --platform linux/amd64 \
  -f docker/gateway.Dockerfile --target gateway -t indra-gateway:dev .
docker buildx build --load --platform linux/amd64 \
  -f docker/native.Dockerfile --target server -t indra-native:dev .
docker buildx build --load --platform linux/amd64 \
  -f docker/tika.Dockerfile -t tika-indra:dev .

docker compose up --detach --wait --wait-timeout 600
curl --fail http://127.0.0.1:9998/readyz
curl --fail --request PUT \
  --header 'Content-Type: image/tiff' --header 'Accept: text/plain' \
  --data-binary @backend/tests/data/two-pages.tiff \
  http://127.0.0.1:9998/tika
```

The last response must contain `INVOICE 12345` followed by `SECOND PAGE 67890`.
The real stack shutdown gate below checks both lines and every response byte.

The native archive build is the expensive first step; application builds reuse
that Docker layer. Its default compiler affinity is CPU IDs `0,1`; use
`--build-arg BUILD_CPUSET=...` for another allowed host cpuset. The Rust build
profiles are `release`, `maxperf` and `profiling`; select one with
`--build-arg PROFILE=profiling`. Compare a profile under the actual workload
before making a performance claim.

Compose consumes built images. Set `INDRA_IMAGE` and `TIKA_IMAGE` to the verified
GHCR digest references when deploying published artifacts. `TIKA_PORT` changes
the local host port. The public binding stays on `127.0.0.1`.

`models-init` downloads and verifies all 14 locked model files, including math,
into the named `model-cache` volume. The pinned snapshot is 38,187,650 bytes;
its revision, names, sizes and SHA-256 values are in
[models.lock.json](../backend/models.lock.json). Subsequent starts reuse verified
files. Serving mounts the cache read-only and never downloads a model. The
image's `/models` directory belongs to UID/GID 65532, which Docker preserves when
initializing the empty named volume. A caller-provided bind directory must have
equivalent write access for initialization and read access for serving.

The runtime profile explicitly enables math recognition and uses one native
worker. This is a starting configuration, not a claim of universally highest
accuracy. The paired text/math and batching experiments determine workload
recommendations. To disable math in a local override, set
`INDRA__ENGINE__MATH: "false"` on `indra`; the full cached snapshot remains valid.

For two CPUs:

```sh
docker compose -f compose.yaml -f compose.2cpu.yaml \
  up --detach --wait --wait-timeout 600
```

| Runtime profile | Tika and gateway CPU | Native OCR CPU | ORT / OpenCV threads | Total memory ceiling |
| --- | ---: | ---: | ---: | ---: |
| Base / `1cpu` | 0.25 | 0.75 | 1 / 1 | 3 GiB |
| `2cpu` | 0.50 | 1.50 | 2 / 2 | 3 GiB |

Each native worker also holds the three decompressed n-gram language models
(about 107 MB) and the tall-window recogniser arena inside these ceilings.

The one-shot cache initializer has a one-CPU, 512-MiB ceiling and finishes before
either serving process starts. Serving CPU ceilings sum to one or two CPUs;
the two-CPU profile retains one worker. These deployment settings are distinct
from a scientific benchmark's recorded cgroup/affinity attestation. Use the
[evaluation runner](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/evaluation/runner/README.md) for matched measurements.

## Operate and stop

```sh
docker compose exec -T tika /indra-gateway probe ready
docker compose exec -T indra /indra-server probe ready
docker compose exec -T indra /indra-server --math true models verify
docker compose logs --follow tika indra
docker compose stop
```

Compose starts `models-init`, waits for successful cache initialization, starts
`indra`, waits for its readiness, then starts Tika. The OCR container owns the
shared network namespace and published port. On a normal whole-project stop,
Compose stops the dependent Tika container before OCR. The gateway immediately
removes readiness and rejects new document admissions, while accepted uploads
and parses finish. OCR stays ready for later pages of those accepted parses.
Compose allows 350 seconds for Tika/gateway and then 100 seconds for OCR.
The gateway defaults have a conservative 346-second shutdown allowance, including
Java termination/reaping and exporter flushing; the additional four seconds
allow scheduling overhead.
This ordering follows Docker's documented
[dependency startup and shutdown rules](https://docs.docker.com/compose/how-tos/startup-order/).

`docker compose stop` retains the model volume. `docker compose down` also retains
named volumes unless `--volumes` is supplied. A direct `indra-server drain` is an
OCR-service maintenance action; use whole-project stop when Tika must finish
multi-page parses. Forced termination cannot provide a graceful completion
guarantee. The [gateway guide](tika-gateway.md#shutdown-ordering-and-budgets)
explains the request, child-process and exporter deadlines.

Readiness reflects verified warm models, draining, terminal failures and the
configured OCR dependency. A temporarily full OCR queue does not flip readiness.
Liveness remains independent of transient dependency readiness. The gateway
image leaves its optional OCR dependency unset; only native deployment arguments
enable `--ocr-ready-endpoint 127.0.0.1:8081`.

For local Rust development, model commands, configuration precedence, OpenAPI,
completion scripts and OTLP/Sentry setup, use
[runtime-service.md](runtime-service.md) and [tika-gateway.md](tika-gateway.md).
Both servers accept flags, an explicit TOML file and structured environment
variables. Runtime filesystems are read-only except bounded `/tmp` mounts;
the initializer alone writes the cache.

## Kubernetes

Use Kubernetes 1.33 or newer, an amd64 node and a default StorageClass that
supports a `ReadWriteOnce` PVC and the configured `fsGroup`. The deployment has
one replica and uses `Recreate`, matching the shared writable cache initialization.
The 1-GiB PVC survives Pod replacement. Model acquisition needs HTTPS access to
the pinned model host; an already verified cache can be reused without a download.

The regular init container finishes cache initialization first. Native OCR is
then an init container with `restartPolicy: Always` and a startup probe, so it
warms before the main Tika/gateway container starts. Kubernetes terminates this
native sidecar after the main container finishes. The Pod grace period is 450
seconds, covering the ordered configured 346-second main and 100-second native
allowances with four seconds of margin. These are configured timeout budgets,
not a hard real-time guarantee under arbitrary host stalls. Native
sidecars are [stable since Kubernetes 1.33](https://kubernetes.io/docs/concepts/workloads/pods/sidecar-containers/);
their [termination order](https://kubernetes.io/docs/concepts/workloads/pods/pod-lifecycle/#pod-shutdown-and-sidecar-containers)
is part of this lifecycle design.

Load the two locally built images into your development cluster, or set both
image names to verified registry digests using a site overlay:

```yaml
apiVersion: kustomize.config.k8s.io/v1beta1
kind: Kustomization
resources:
  - ../../deploy/overlays/1cpu
images:
  - name: tika-indra
    newName: ghcr.io/YOUR_OWNER/tika-indra-tika
    digest: sha256:YOUR_VERIFIED_TIKA_DIGEST
  - name: indra-native
    newName: ghcr.io/YOUR_OWNER/tika-indra-native
    digest: sha256:YOUR_VERIFIED_NATIVE_DIGEST
```

The example resource path assumes your site overlay lives two directories below
the repository root. Choose `2cpu` for the larger profile. Kubernetes
[Kustomize image substitutions](https://kubernetes.io/docs/tasks/manage-kubernetes-objects/kustomization/)
update both the model initializer and the native sidecar.

For images already loaded into a local cluster:

```sh
kubectl apply -k deploy/overlays/1cpu
kubectl rollout status deployment/tika-indra --timeout=600s
kubectl port-forward service/tika-indra 9998:9998
```

Use `/startupz`, `/readyz` and `/healthz` for their separate purposes. The gateway
probes use HTTP on the public container port; loopback-only OCR probes execute
the Rust binary directly. The Service exposes only Tika port 9998. There are no
sleep-based lifecycle hooks. Keep the grace period intact during Pod deletion.

## Verify signed images before deploying

Every release signs the six exact image digests with keyless Cosign under the
GitHub OIDC identity of `release.yml`, attaches an SPDX SBOM (the Rust images
embed their dependency tree with cargo-auditable, so the SBOM lists crates) and
a release-evidence attestation. Verify a digest before putting it in a site
overlay; a mutable tag proves nothing:

```sh
identity='^https://github.com/somehowchris/tika-indra-ocr-client/\.github/workflows/release\.yml@refs/(heads/main|tags/v[^/]+)$'
issuer=https://token.actions.githubusercontent.com
for role in native gateway bridge tika tesseract backend; do
  ref="ghcr.io/somehowchris/tika-indra-$role@sha256:DIGEST_FROM_THE_RELEASE_INDEX"
  cosign verify "$ref" --certificate-oidc-issuer "$issuer" --certificate-identity-regexp "$identity"
  cosign verify-attestation --type spdxjson "$ref" --certificate-oidc-issuer "$issuer" --certificate-identity-regexp "$identity" \
    | jq -r '.payload | @base64d | fromjson | .predicate.packages | length'
done
```

The release run's `ocr-candidates-*` artifact holds `candidates.json` with every
role's digest and `candidates.sigstore.json`, the signed index; `cosign verify-blob
--bundle candidates.sigstore.json candidates.json` with the same identity flags
proves the index itself.

## Enable OTLP and Sentry

The native service reads `INDRA__TELEMETRY__*`, the gateway `TIKA__TELEMETRY__*`;
the Java child never inherits the gateway's variables. For Compose:

```yaml
services:
  indra:
    environment:
      INDRA__TELEMETRY__OTLP_ENDPOINT: http://otel-collector:4318
      INDRA__TELEMETRY__SENTRY_DSN: ${INDRA_SENTRY_DSN}
  tika:
    environment:
      TIKA__TELEMETRY__OTLP_ENDPOINT: http://otel-collector:4318
      TIKA__TELEMETRY__SENTRY_DSN: ${TIKA_SENTRY_DSN}
```

For Kubernetes, add the endpoint as a plain variable and the DSN or an OTLP
authorization header from a Secret in a site overlay patch:

```yaml
- op: add
  path: /spec/template/spec/containers/0/env/-
  value:
    name: TIKA__TELEMETRY__SENTRY_DSN
    valueFrom:
      secretKeyRef: {name: tika-telemetry, key: sentry-dsn}
```

An authorization value is accepted over plaintext `http` only for a loopback
collector; use `https` for a remote one. [The service guide](runtime-service.md#tracing-metrics-and-sentry)
lists every setting, sampling defaults and what is never exported.

## Tesseract comparison and rollback

```sh
docker buildx build --load --platform linux/amd64 \
  -f docker/tesseract.Dockerfile -t tika-tesseract:dev .
docker compose -f compose.tesseract.yaml up --detach --wait --wait-timeout 600
```

Stop the native project first when both use local port 9998. This image uses the
same gateway and Tika JAR, with native Tesseract and the pinned English
`tessdata_best` smoke model under `/opt/tessdata`. It is a local comparison path.
The multilingual study supplies its separately hashed fast/best language models
and matched [evaluation XML](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/deploy/evaluation/) through the evaluation runner.
It does not use the single-language smoke model as a multilingual benchmark.

## Verify a real whole-stack shutdown

Build the `gateway-acceptance` target to obtain the unit/process harnesses and
`/indra-stack-acceptance` in one image. The production `gateway` target contains
the runtime binary and source/relink material; test executables stay in the
acceptance target. The process harness honors
`INDRA_TEST_GATEWAY_BIN=/indra-gateway` when moved into scratch.

To export just the static host acceptance tool:

```sh
docker buildx build --platform linux/amd64 -f docker/gateway.Dockerfile \
  --target acceptance-tools --output type=local,dest=./acceptance-tools .
docker compose --project-name indra-shutdown-1 up --detach --wait --wait-timeout 600
./acceptance-tools/indra-stack-acceptance \
  --endpoint 127.0.0.1:9998 --output shutdown-1cpu.json --cpus 1 \
  compose --project indra-shutdown-1 --file compose.yaml
```

The tool stops that exact project. It verifies Tika 3.3.2 and both pages before
shutdown, holds a newly admitted upload open, observes draining and rejection of
new work, finishes the upload, and requires the complete text to match the
baseline byte for byte. The host supplies Docker Compose or kubectl. See the
[acceptance guide](tika-gateway.md#real-stack-shutdown-acceptance) for the two-CPU
and direct-Pod Kubernetes modes, evidence fields and limitations.

Native OCR and gateway images carry `/relink` with verified source and static
link inputs. Tika carries the two component bundles at `/relink/bridge` and
`/relink/gateway`. Follow [runtime-relinking.md](runtime-relinking.md) to extract
and verify those materials.
