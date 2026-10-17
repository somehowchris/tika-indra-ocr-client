# Tika 3.3.2 with Indra OCR

A small Tesseract CLI adapter for Apache Tika, backed by a persistent Indra OCR service. Tika keeps document parsing and PDF rendering. Indra performs OCR on the rendered images. The adapter is dependency-free Rust, statically linked with GNU/glibc. Final images are `scratch` or Debian distroless; builders are Debian-based.

**Recommended starting point:** the 1-CPU profile, FP32 recognition, upstream line batching, 300-DPI grayscale PDF rendering, and caller concurrency one. Select the 2-CPU and math profiles using the measurements for your own documents. There is no demonstrated setting that maximizes accuracy, latency and throughput simultaneously.

This repository was empty when inspected. The implementation and evidence here were created for the requested investigation. Read [the measured results](docs/results.md) before treating the deployment as production-ready: native execution, image-root execution, container execution and production accuracy are different verification gates.

## What is pinned

| Component | Pin |
|---|---|
| Apache Tika | **3.3.2**, official standard-server JAR verified with SHA-512 |
| Indra package | **0.3.2**, source `3d773aeedc5d79d1d0a2a04007a9075cd71a59f4` |
| Indra models | **v3.1.1**, immutable model revision `ed345c7f158eddbc7dee0c6a7269afb75a94f7e8`; every file SHA-256 checked |
| Rust | **1.99.1**, `x86_64-unknown-linux-gnu`, `+crt-static` |
| Runtime bases | Java 21 / Python 3.13, Debian 13 distroless, immutable image digests |
| Packaged Tesseract baseline | Same Tika/rendering policy, English `tessdata_best` at a pinned commit and SHA-256; native experiments also compare the hashed distribution model |

Exact pins are in [deploy/dependencies.lock.json](deploy/dependencies.lock.json), [backend/models.lock.json](backend/models.lock.json), the Dockerfiles and the hashed Python lock. The [Apache release catalog](https://release-catalog.apache.org/tika/3.3.2/index.html) confirms the requested Tika release. The [Indra findings](research/indra-findings.md) link each upstream claim to its source.

**License:** the adapter is Apache-2.0. Indra's code and its weights have different licenses. The pinned weights include revenue/funding thresholds, competition restrictions, and output attribution/share-alike provisions. Check [the exact model license](https://huggingface.co/datalab-to/indra-ocr/blob/ed345c7f158eddbc7dee0c6a7269afb75a94f7e8/LICENSE) against your intended deployment; an Apache code license does not remove those terms. The GNU static bridge images include glibc notices and exact source/relinking materials in `/relink`; see [third-party notices](THIRD_PARTY_NOTICES.md) and [relinking](docs/relinking.md).

## Local Docker

Use a Linux/amd64 Docker host with the current Docker Compose plugin. The authored CI uses Docker 29.9.1 and the engine's native BuildKit driver. The deployment allocates 3 GiB to the application stack; the Docker VM/host also needs room for its OS and image builds. ARM emulation is outside this validation.

Build in dependency order from the repository root:

```sh
docker build --platform linux/amd64 -f docker/bridge.Dockerfile -t indra-bridge:dev .
docker build --platform linux/amd64 -f docker/tika.Dockerfile -t tika-indra:dev .
docker build --platform linux/amd64 -f backend/Dockerfile -t indra-backend:dev .

docker compose -f compose.yaml up -d --no-build --wait --wait-timeout 180
python3 scripts/smoke_stack.py
```

The first build downloads dependencies and the locked models. Startup and inference use those local models. The final-stage backend build executes real OCR with math disabled and enabled and fails if its interpreter, native libraries or models do not work.

To evaluate math recognition, change the Indra service's `INDRA_MATH` value from `"0"` to `"1"` in the selected Compose/Kubernetes configuration and recreate it. Both model sets are already in the image. This option needs equation-specific accuracy checks; the local ordinary-text test measured extra cost without an accuracy improvement.

Extract text:

```sh
curl --fail-with-body \
  -H 'Accept: text/plain' -H 'Content-Type: application/pdf' \
  --data-binary @document.pdf -X PUT http://127.0.0.1:9998/tika
```

Tika is published only on host loopback. Indra shares Tika's network namespace and listens only on `127.0.0.1:8081`. The smoke check covers an image, an image-only PDF and a native-text PDF through both `/tika` and `/rmeta/text`.

To switch to the total 2-CPU profile, replace the running stack:

```sh
docker compose -f compose.yaml down
docker compose -f compose.yaml -f compose.2cpu.yaml up -d --no-build --wait --wait-timeout 180
python3 scripts/smoke_stack.py
```

Inspect and stop:

```sh
docker compose -f compose.yaml logs --tail=100
docker compose -f compose.yaml down
```

| Whole application budget | Tika limit | Indra limit | Memory total |
|---|---:|---:|---:|
| 1 CPU | 0.25 CPU | 0.75 CPU | 3 GiB |
| 2 CPUs | 0.50 CPU | 1.50 CPUs | 3 GiB |

Limits include the JVM parent, forked parser, spawned Rust bridge and every Indra child. These conservative per-container partitions cannot borrow idle quota from the other container. Native benchmarks use a common eligible CPU set; they are not measurements of this quota partition. See [resource tradeoffs](docs/design.md#cpu-and-memory-budgets).

### Compare with Tesseract

```sh
docker compose -f compose.yaml down
docker build --platform linux/amd64 -f docker/tesseract.Dockerfile -t tika-tesseract:dev .
docker compose -f compose.tesseract.yaml up -d --no-build --wait --wait-timeout 180
python3 scripts/smoke_stack.py
```

For the 2-CPU baseline, use `-f compose.tesseract.yaml -f compose.tesseract.2cpu.yaml`. Run one stack at a time. Tesseract receives the same aggregate CPU and memory ceilings. For a fair OCR-only experiment, the Indra stack additionally uses `compose.benchmark.yaml`; the baseline already forces the same 300-DPI OCR-only policy. Production defaults use Tika AUTO.

## Simple Kubernetes deployment

The manifests create one Deployment with two containers in one Pod and a ClusterIP Service. They use non-root users, read-only roots, dropped capabilities, bounded temporary storage and separate startup/readiness/liveness checks. `Recreate` avoids temporarily doubling the application budget during an update, with a corresponding availability gap.

On a local cluster, load `tika-indra:dev` and `indra-backend:dev` into that cluster's image store using the cluster's normal image-import command. On a registry-backed cluster, set both images to your **verified release digests** in the selected Kustomize overlay:

```yaml
images:
  - name: tika-indra:dev
    newName: ghcr.io/YOUR_OWNER/tika-indra-tika
    digest: sha256:YOUR_VERIFIED_TIKA_DIGEST
  - name: indra-backend:dev
    newName: ghcr.io/YOUR_OWNER/tika-indra-backend
    digest: sha256:YOUR_VERIFIED_BACKEND_DIGEST
```

Then deploy one budget:

```sh
kubectl apply -k deploy/overlays/1cpu
kubectl rollout status deployment/tika-indra --timeout=300s
kubectl port-forward service/tika-indra 9998:9998
```

Use `deploy/overlays/2cpu` to replace that Deployment with the larger profile. Run `python3 scripts/smoke_stack.py` through the forwarded port. Kubernetes does not automatically verify Cosign signatures; verify before applying, or add a cluster admission policy if continuous enforcement is required.

## Signed releases and verification

The GitHub Actions workflows pin actions to full commit SHAs. Pull requests get read-only permissions. Pushes to `main` and version tags run the reusable CI gate, build candidates with commit-addressed tags, test the **published digests** in Compose and Kubernetes at both CPU budgets, and only then sign and attest them through GitHub OIDC. Host-installed, checksum-pinned Syft produces the SPDX SBOM; Cosign signs and verifies both the image and SBOM. BuildKit emits provenance. The digests are immutable; a commit-addressed tag can change on a rerun. New main pushes cancel superseded runs, so an interrupted candidate may be unsigned. Use a successful run's digest artifact and verify every digest before deployment.

Use the digest artifact from your completed release, not an assumed image name or a mutable tag:

```sh
IMAGE='ghcr.io/YOUR_OWNER/tika-indra-tika@sha256:YOUR_DIGEST'
IDENTITY='https://github.com/somehowchris/tika-indra-ocr-client-/.github/workflows/release.yml@refs/heads/main'
# For a version-tag run, use its exact @refs/tags/YOUR_TAG identity instead.

cosign verify --certificate-identity "$IDENTITY" \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com "$IMAGE"
cosign verify-attestation --type spdxjson --certificate-identity "$IDENTITY" \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com "$IMAGE"
```

No registry publication or successful signing run is implied by the presence of these workflows. Their execution status is recorded in [results](docs/results.md).

The release also runs [two 180-request signed-container comparisons](benchmarks/README.md#complete-signed-container-matrix): the shared CPU configuration and the exact deployment CPU partitions. Both enforce total CPU limits of one/two and 3 GiB memory through a verified parent cgroup. Their raw artifact and job summary must complete before the release digest artifact is published. Documentation and retained-evidence-only main pushes do not rebuild unchanged runtime images; version tags still run the release gates.

## Where to go next

- [Architecture, exact Tika contract and optimization decisions](docs/design.md)
- [Measured results and remaining acceptance gates](docs/results.md)
- [Reproduce the benchmarks and supply your own corpus](benchmarks/README.md)
- [Backend API, model validation and operational limits](backend/README.md)
- [Primary-source Indra research](research/indra-findings.md)
- [Benchmark methodology and interpretation](research/benchmark-methodology.md)

Start with the provided fixture suite, then a held-out sample of your own documents. Include per-script and per-document-type results, errors, reading order, memory peaks, startup, queueing and the exact deployed image digests in the acceptance decision.
