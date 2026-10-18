# Whole-stack local telemetry experiment

The prospective [protocol](../runtime/validation/telemetry-http-protocol.json) compares the same ordinary OCR worker with SDK export disabled and enabled. Gateway, Tika 3.3.2, bridge, native worker, HTTP client and both local receiver threads share one Docker cgroup: 1 or 2 CPUs, 3 GiB total memory and no swap. Docker networking is disabled; exporters use loopback. No vendor receives data.

This is a synthetic implementation experiment, not multilingual OCR quality evidence. The math-layout fixture runs with `math=false`. Twelve successful requests per cohort support a descriptive median and wall throughput; they do not support a p99 or a population claim. The actual classified Sentry event occurs in a separate terminal fault phase. Success-cohort timing does not measure Sentry error throughput.

## Build and admit

Build the example from the exact committed source, using the repository toolchain:

```sh
RUSTFLAGS='-Ctarget-feature=+crt-static' cargo build --locked --release \
  --manifest-path runtime/Cargo.toml --target x86_64-unknown-linux-gnu \
  -p indra-gateway --example telemetry_http
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
