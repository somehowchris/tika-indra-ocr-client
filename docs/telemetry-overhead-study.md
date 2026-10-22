# Whole-stack local telemetry experiment

The prospective [protocol](../runtime/validation/telemetry-http-protocol.json) compares the same ordinary OCR worker with SDK export disabled and enabled. Gateway, Tika 3.3.2, bridge, native worker, HTTP client and both local receiver threads share one Docker cgroup: 1 or 2 CPUs, 3 GiB total memory and no swap. Docker networking is disabled; exporters use loopback. No vendor receives data.

This is a synthetic implementation experiment, not multilingual OCR quality evidence. All three fixtures run with `math=true` under the [always-math amendment](../evaluation/audits/always-math-20261019/amendment.json), and the native worker must serve the Indra 1.1 model set bound by the [Indra 1.1 amendment](../evaluation/audits/indra-1-1-20261020/amendment.json); the controller's `control/models.sha256` inventory must list exactly that file set. This changes the prospective overhead study; retained mixed-mode correctness packets keep their original protocols and verifier identities. Twelve successful requests per cohort support a descriptive median and wall throughput; they do not support a p99 or a population claim. The actual classified Sentry event occurs in a separate terminal fault phase. Success-cohort timing does not measure Sentry error throughput.

## Run it on your host from a signed release

A release dispatched on a `v*` tag with `run_evaluation=true` retains the artifact `telemetry-study-kit-SHA-RUN-ATTEMPT`. It holds the static driver from the gateway acceptance build, the release's trace-acceptance image, `identity.json` with its native-equivalence receipt, and a correctness packet already produced on the hosted runner. Nothing needs to be built or hashed on the host. Model weights are not redistributed: the image's own native server fetches and verifies them. [`scripts/telemetry-study-kit.sh`](../scripts/telemetry-study-kit.sh) shows how the release assembles it; the sections below describe each binding.

The host needs Linux x86-64 with cgroup v2, Docker Engine 29 or newer with the containerd image store (`docker info` reports `driver-type: io.containerd.snapshotter.v1`), `git`, `gh`, `jq`, coreutils, util-linux and binutils. Timing pins the protocol's affinities `0` and `0,1`, so logical CPUs 0 and 1 must share a last-level cache and stay idle: no builds, other containers or interactive load during the timed run.

```sh
git clone https://github.com/somehowchris/tika-indra-ocr-client.git && cd tika-indra-ocr-client
git checkout TAG                        # the tag the release ran on
gh run download RUN -D "$HOME/telemetry" -n telemetry-study-kit-SHA-RUN-ATTEMPT
kit=$HOME/telemetry/telemetry-kit
(cd "$kit" && sha256sum --check SHA256SUMS)
docker load --input "$kit/trace-acceptance.tar"
image=$(jq -r .assembled_image_root "$kit/identity.json")
docker image inspect --format '{{.Id}}' "$image"   # must print the same ID
chmod 0755 "$kit/telemetry_http"

mkdir "$HOME/telemetry/fetch"                     # fetch and verify the locked models
docker run --rm --user "$(id -u):$(id -g)" --entrypoint /indra-server \
  --mount "type=bind,src=$HOME/telemetry/fetch,dst=/models" "$image" --math true models fetch
models=$(scripts/stage-locked-models.sh \
  "$HOME/telemetry/fetch/$(jq -r .revision backend/models.lock.json)" "$HOME/telemetry/models")

TELEMETRY_CORRECTNESS_CPUSET_1=0 TELEMETRY_CORRECTNESS_CPUSET_2=0,1 \
  scripts/benchmark-telemetry-http.sh correctness \
  "$kit/telemetry_http" "$kit/identity.json" "$image" "$models" "$HOME/telemetry/correctness"
tar -C "$HOME/telemetry" -czf telemetry-correctness.tar.gz correctness
```

Send `telemetry-correctness.tar.gz` back. After its review you receive a timing clearance JSON bound to this source, driver, identity, image and the `control/models.sha256` inventory of your correctness run. Then run and analyze the timed study on the same checkout:

```sh
TELEMETRY_TIMING_CLEARANCE=$HOME/telemetry/clearance.json \
  scripts/benchmark-telemetry-http.sh run \
  "$kit/telemetry_http" "$kit/identity.json" "$image" "$models" "$HOME/telemetry/timing"
scripts/analyze-telemetry-http.sh "$HOME/telemetry/timing"
tar -C "$HOME/telemetry" -czf telemetry-timing.tar.gz timing
```

Send `telemetry-timing.tar.gz` back unedited, including any failed cell. A failure stops the study; rerunning starts a new output directory and the failed one is kept.

## Build and admit

The driver is the `telemetry_http` example of `indra-gateway`. The gateway builder compiles it with the shipped gateway's pinned toolchain and static checks, and the `gateway-acceptance` image carries it at `/indra-telemetry-http`, listed in `/build-evidence/binaries.sha256`:

```sh
docker buildx build --load --platform linux/amd64 -f docker/gateway.Dockerfile \
  --target gateway-acceptance -t indra-gateway-acceptance:local .
id=$(docker create indra-gateway-acceptance:local)
docker cp "$id:/indra-telemetry-http" ./telemetry_http && docker rm "$id"
```

The driver is mounted read-only into an assembled diagnostic image containing `/indra-gateway`, `/indra-server`, `/opt/indra/tesseract`, `/usr/bin/java`, `/opt/tika/tika-server.jar` and the 16 verified JARs under `/opt/tika/trace`. The controller rejects a driver with a program interpreter or shared-library dependencies. Java and its runtime libraries belong to the image, separately from the static Rust executables.

Provide an identity JSON with these fields:

| Field | Required binding |
| --- | --- |
| `schema_version` | `1` |
| `source_sha` | Exact producing driver Git commit; relevant source must be clean |
| `binary_sha256`, `protocol_sha256` | Actual driver ELF and frozen protocol |
| `assembled_image_root` | Immutable daemon image root, independently walked to selected OCI manifest and config |
| `role_sha256` | Exactly `bridge`, `gateway`, `java`, `native`, `tika`, matched against files inside every executed cell |
| `trace_dependencies_sha256` | Exactly 16 JAR basenames and hashes, matched inside every cell |
| `native_materials_sha256` | Matched native material manifest hash |
| `native_equivalence_passed` | `true`; an assertion that requires supporting receipts below |
| `native_equivalence_receipt` | Simple sibling filename, not a path or symlink |
| `native_equivalence_receipt_sha256` | Actual supporting receipt hash |

A boolean and a file hash are not independent evidence of native equivalence. Review the receipt's producing command, source/material closure, exact native binary and retained exit/output receipts. A diagnostic assembled from different builds must remain labeled as such. Timing additionally requires explicit authorization of the exact identity metadata after its native gate receipts have been reviewed. Existing native/reference and full trace-graph gates remain required separately.

The model tree must contain no symlinks. The controller inventories all model files before starting containers. Each cell proves its exact owner label, image root, resource flags, live host PID and `docker-FULLCID.scope`, then walks host cgroup ancestors and verifies effective leaf/driver CPU affinity before allowing model work.

## Execute

```sh
scripts/benchmark-telemetry-http.sh plan /absolute/new-plan
scripts/benchmark-telemetry-http.sh correctness \
  /absolute/static-driver /absolute/identity.json IMAGE /absolute/models /absolute/new-correctness
```

Correctness uses CPU 12 and 12,13 by default and executes one pair per budget plus separate references. These cells provide no timing evidence. It requires ordinary local Docker administration; it uses no privileged container, host PID namespace, host control socket or added capability.

Timed execution is held until all compilation and other experiments finish. Set `TELEMETRY_TIMING_CLEARANCE` to a reviewed JSON that binds `source_sha`, `binary_sha256`, `protocol_sha256`, `identity_metadata_sha256`, `assembled_image_root`, and `models_inventory_sha256`, and includes `native_gate_receipts_reviewed:true`, `compilation_and_other_measurements_finished:true`, `affinity_1:"0"`, `affinity_2:"0,1"`. The models hash is the SHA-256 of the sorted relative-path `sha256sum` inventory emitted as `control/models.sha256` by correctness mode. Both CPUs must share their last-level cache.

```sh
TELEMETRY_TIMING_CLEARANCE=/absolute/reviewed-clearance.json \
  scripts/benchmark-telemetry-http.sh run \
  /absolute/static-driver /absolute/identity.json IMAGE /absolute/models /absolute/new-timing
scripts/analyze-telemetry-http.sh /absolute/new-timing
```

Ten paired repetitions per budget alternate off/enabled order. Each arm starts a fresh stack, performs three unmeasured fixture requests and requires exact same-cap reference bodies for every warmup and measured response. A failure stops the study; it is never silently replaced. Receiver threads run in both arms, including off mode.

Enabled acceptance requires the exact scheduled gateway → Java parse → Java OCR → native HTTP/inference graph, actual cumulative gateway/native request counters, a real sanitized gateway Sentry failure, no receiver/export errors or drop markers, and no synthetic client authorization/cookie/baggage canaries in exporter HTTP headers or complete golden document lines of at least 8 UTF-8 bytes in raw export bodies. This does not exhaustively exclude shorter fragments or arbitrary unrecognized OTLP attribute text. A distinct deliberately configured exporter Authorization credential is not confused with a client secret. The prospective protocol amendment retains the original collector source hash and adds an opt-in header validator; its original entry point delegates with a no-op, preserving the original trace gate behavior. Receiver size, count and time limits are finite. Off mode must export nothing.

Raw synthetic SDK payloads, request outcomes, summaries, resource snapshots, image/asset proofs and hashes remain in the new output directory. They contain synthetic inputs only. Each cell records its running/completed/failed state; interrupted or failed cells and cleanup errors remain visible. Cleanup verifies exact ownership before stopping or removing its container.

`analysis/paired.json` retains all 20 paired cohort observations and each enabled-minus-off difference and ratio. Cohort CPU and throttling cover the measured request interval, including exports that actually occur there. Startup-through-export-audit CPU and full-cell memory peak include startup, warmup, terminal fault, flush and local receivers; they are not attributed solely to successful requests. Ratios with a zero denominator are null. Request observations are not treated as independent paired repetitions.

No timings have been claimed merely because these controls or the static binary exist. Preserve failed attempts, report hardware and identities, and inspect the raw paired results before drawing a conclusion.
