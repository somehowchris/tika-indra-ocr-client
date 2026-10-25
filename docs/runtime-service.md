# Rust OCR service: operation and development

`indra-server` is the persistent Axum/Tokio service behind the Tika executable adapter. It verifies a pinned local model cache, warms each native worker, accepts bounded image requests, and returns UTF-8 text. Tika 3.3.2 remains responsible for document parsing and PDF rasterization. The serving path is Rust, ONNX Runtime and OpenCV; Python is only a separately measured reference implementation.

The [native engine](native-engine.md) defines the supported OCR algorithm, image limits and parity gates. The [native build](native-build.md) defines the GNU/glibc static build and final-image verification. Successful service unit tests do not prove native model parity or whole-stack performance.

Whole-stack graceful shutdown also requires the shipped Tika admission gateway. Tika 3.3.2's watchdog forcibly destroys parser JVMs on its own shutdown; keep OCR ready while the gateway finishes accepted Tika parses, then terminate OCR afterward. Draining only the OCR process cannot preserve a multi-page parse that has not requested all its pages yet.

## Local development

Run the native-free Rust checks in [development.md](development.md#native-free-checks), then use the [Docker quickstart](../README.md#local-docker) to build and start the actual service with Tika. Native compilation happens in the pinned GNU builder; the serving image contains the static executable and no shell. The builder's `/native/cargo-env.sh` is not a host installation.

Once the Compose stack is Ready, run the shipped executable directly:

```sh
docker compose exec -T indra /indra-server models verify
docker compose exec -T indra /indra-server check-config
docker compose exec -T indra /indra-server probe ready
docker compose exec -T indra /indra-server openapi > openapi.json
docker compose exec -T indra /indra-server completions bash > indra-server.bash

curl --fail-with-body --silent --show-error \
  -X PUT -H 'Accept: text/plain' -H 'Content-Type: image/png' \
  --data-binary @backend/tests/data/smoke.png http://127.0.0.1:9998/tika
```

These commands see the running container's environment and model path, but not flags given only on its `serve` command line, such as Compose's `--startup-timeout-seconds 300`. They do not expose the private OCR port or require a host-linked executable. For Rust changes, stop the stack, rebuild the affected images using the quickstart and start it again; keep the named model volume.

Debug builds read `.env` from the current directory before creating any runtime or worker. Existing environment variables take precedence over `.env`; a missing file is accepted and a malformed file is an error. Release builds never read it. The `dotenvy` call is excluded with `cfg(debug_assertions)`.

## Configuration

Copy [indra.example.toml](../runtime/indra.example.toml) and run `indra-server --config PATH check-config`. Precedence is **defaults → explicit TOML file → environment → flags**, resolved before typed decoding and cross-field validation. An explicitly named file must exist. Unknown fields are errors. Config errors retain safe field paths, expected types or TOML byte offsets while removing raw values and source snippets that could contain credentials. See [configuration.rs](../runtime/crates/indra-server/src/configuration.rs) for the complete schema and validation.

Every setting in the example has a corresponding global CLI flag. Flags use hyphens; environment variables use the section name and double underscores. Flags can appear before or after a subcommand. Boolean flags accept `--math`, `--math=true` and `--math=false`.

| TOML field | Environment | Flag |
|---|---|---|
| `server.listen` | `INDRA__SERVER__LISTEN` | `--listen` |
| `server.workers` | `INDRA__SERVER__WORKERS` | `--workers` |
| `server.queue_capacity` | `INDRA__SERVER__QUEUE_CAPACITY` | `--queue-capacity` |
| `engine.threads` | `INDRA__ENGINE__THREADS` | `--threads` |
| `engine.cv_threads` | `INDRA__ENGINE__CV_THREADS` | `--cv-threads` |
| `engine.batch_size` | `INDRA__ENGINE__BATCH_SIZE` | `--batch-size` |
| `engine.math` | `INDRA__ENGINE__MATH` | `--math` |
| `models.directory` | `INDRA__MODELS__DIRECTORY` | `--model-dir` |
| `models.connect_timeout_seconds` | `INDRA__MODELS__CONNECT_TIMEOUT_SECONDS` | `--model-connect-timeout-seconds` |
| `models.request_timeout_seconds` | `INDRA__MODELS__REQUEST_TIMEOUT_SECONDS` | `--model-request-timeout-seconds` |
| `telemetry.otlp_endpoint` | `INDRA__TELEMETRY__OTLP_ENDPOINT` | `--otlp-endpoint` |
| `telemetry.otlp_authorization` | `INDRA__TELEMETRY__OTLP_AUTHORIZATION` | `--otlp-authorization` |
| `telemetry.sentry_dsn` | `INDRA__TELEMETRY__SENTRY_DSN` | `--sentry-dsn` |
| `telemetry.environment` | `INDRA__TELEMETRY__ENVIRONMENT` | `--environment` |

The remaining server, engine and telemetry flags are their field names with underscores replaced by hyphens. `indra-server --help` lists all of them. Prefer injected environment variables or a mounted secret file for credentials; command-line arguments are visible to process inspection. Configuration is immutable after startup.

## Model cache and startup

`models fetch` is an explicit initialization command. It acquires only the immutable selected snapshot, verifies sizes and SHA-256 hashes, reuses already valid files, and publishes each completed file atomically. `models verify` is a read-only integrity gate. Serving verifies the cache again and never downloads missing or damaged artifacts. The selected revision is `ed345c7f158eddbc7dee0c6a7269afb75a94f7e8`.

In Docker, initialize a named model volume once and mount it read-only in the service. In Kubernetes, use an init container to populate the same volume, then mount it read-only in the OCR container. Pass the same `--math` setting to fetch, verify and serve so the selected model set agrees. A revision-specific directory allows a new deployment to acquire its snapshot without overwriting a running one. Model metadata and licensing are part of the verified set; follow the [cache implementation](../runtime/crates/indra-models/src/lib.rs) and model licence before commercial use.

The listener starts while models load, so startup and liveness probes remain responsive. Readiness becomes successful only after **every configured worker has verified models, opened sessions and completed warmup**. Failed loading or warmup marks the process failed and exits with an error; it never becomes ready with a partial model set.

## Admission, CPU and batching

One current-thread Tokio runtime handles network I/O. One dedicated native thread is the default; no native OCR, Rayon pool or blocking model initialization runs inside an async task. Worker count is at most two. Each persistent native worker explicitly reserves an 8 MiB stack: actual service acceptance found that engine initialization overflowed Rust's default 2 MiB worker stack. This reservation occurs once per worker, contributes to the process memory budget, and does not add per-request heap churn. The configuration rejects `workers × max(threads, cv_threads) > 2`, which prevents the most obvious oversubscription. This is a thread policy, not a CPU quota: enforce the total Tika, bridge and OCR budget with the measurement/deployment cgroup and record actual CPU throttling.

Use one worker with `--threads 1 --cv-threads 1` for the one-CPU baseline; compare one worker with `--threads 2 --cv-threads 2` under the two-CPU total budget. Two workers with one thread each are an alternative to measure, with duplicated sessions and working memory. Neither option is asserted faster without paired whole-stack results. ORT spinning switches and the batch row cap are explicit settings. [Engine batching](native-engine.md#batching-and-memory) groups similarly sized lines dynamically within a document; it does not insert new requests into an active decoder cache.

Admission covers uploads, queued requests, native work **and retained response buffers**. Capacity is `workers + queue_capacity`, limited to eight, with a fixed FIFO. The default is three slots. An admitted upload has one reusable input buffer and one reusable output string. The default pool reserves capacity for `3 × (20 + 4) = 72 MiB`; this is buffer capacity, not measured RSS and not the full native working set. Native sessions and temporary matrices have separate memory costs.

When every slot is occupied, `/ocr` immediately returns `503 queue_full` with `Retry-After: 1`. Readiness stays successful: brief saturation is not a deployment state change. A payload is copied once from received frames into the preallocated input buffer. Completed text moves into a `Bytes::from_owner` response; its owner returns both buffers and releases admission only when the final response-byte reference is dropped. The implementation does not claim that Axum, Bytes, tracing or the native libraries allocate nothing. The `service_allocations` gate measures that churn on the warmed service instead: it counts the Rust allocator calls of one `/healthz`, one `/readyz` and one `503 queue_full` loopback exchange, from accept until every allocated byte is freed again, and fails when repeats differ or a count exceeds the ceiling committed in [service_allocations.rs](../runtime/crates/indra-server/tests/service_allocations.rs). No tracing subscriber is installed there, so JSON logging and OTLP export are not included.

The default and shipped native listeners use loopback; the Kubernetes probes execute the service's own client inside its container. The admission bound covers OCR payloads; the native HTTP transport has no global TCP connection quota or first-header deadline, and a client that stalls while reading its response keeps that admission slot until it closes or the shutdown deadline expires. Keep the native listener private and expose document ingestion through the [Tika gateway](tika-gateway.md), which bounds connections and header completion. Binding native OCR to `0.0.0.0:8081` makes OCR and probes reachable by its network peers and needs equivalent ingress bounds.

## API contract

`POST /ocr` accepts one raw encoded PNG, JPEG or classic TIFF image, with one positive `Content-Length`. It accepts `application/octet-stream`, `image/png`, `image/jpeg` and `image/tiff`. Chunked transfers, trailers, repeated lengths and body-length mismatches are rejected. Tika sends its rasterized images through the static executable adapter using this protocol. The configured input bound cannot exceed 20 MiB; the output bound cannot exceed 4 MiB. Decoded image and page bounds are documented by the engine.

Success returns `text/plain; charset=utf-8`; empty text is a valid success. Every OCR response has a generated `X-Request-ID`, `Cache-Control: no-store`, `X-Content-Type-Options: nosniff` and `Connection: close`. Errors have JSON fields `code`, `message` and `request_id`; no native error string or document content is returned.

| Status | Meaning |
|---:|---|
| 400 | Invalid framing, trailer or body-length mismatch |
| 411 | Missing or repeated Content-Length |
| 413 | Encoded, decoded, line or output resource bound exceeded |
| 415 | Unsupported content type |
| 422 | Invalid or unsupported encoded image |
| 503 | Admission full, starting, draining or unavailable; retry advice is one second |
| 504 | Upload or end-to-end request deadline expired |
| 500 | Terminal native or internal ownership failure |

Utoipa generates OpenAPI from the handlers, including binary request media, required headers, error schemas and lifecycle responses. Fetch `/openapi.json` or run `indra-server openapi` without opening models or a listening socket. No external documentation CDN is required.

## Probes and graceful shutdown

| Endpoint / CLI | Success condition |
|---|---|
| `/startupz` / `probe startup` | Every worker completed verification and warmup; no terminal failure |
| `/healthz` / `probe health` | Supervisor is healthy, including normal startup and draining |
| `/readyz` / `probe ready` | All workers ready and new admission enabled; brief saturation is allowed |
| `POST /drain` / `drain` | Remove readiness and begin finishing accepted work; real loopback peer required |

The CLI commands perform bounded TCP exchanges directly, so scratch or distroless final images need no shell, curl or wget. `/drain` examines the socket peer; forwarded headers cannot grant access. The supplied listen address is reused by CLI probes, with wildcard addresses mapped to loopback.

SIGTERM, Ctrl-C and `drain` first change the lifecycle atomically: readiness fails and new OCR admission stops. Accepted uploads can still join the FIFO. Queued and running work retain their original deadlines, and response ownership is retained until the transport releases its bytes. When accepted work reaches zero, workers exit and HTTP stops. No arbitrary sleep substitutes for ownership tracking.

One absolute request deadline includes upload, queue and inference; upload additionally has its shorter timeout. A disconnected or expired caller cannot release native capacity while ONNX Runtime still accesses a buffer. Work canceled before execution is skipped when the worker reaches it. During execution, deadlines are cooperative between native calls. If a synchronous call exceeds its deadline plus the configured grace, the supervisor removes readiness, reports a finite watchdog failure and terminates the process after bounded cleanup. It cannot safely force-unwind native code or promise completion of a hung call.

With defaults, allow **at least 90 seconds**, preferably 100, for container termination: 75 seconds for draining, up to two seconds for HTTP cleanup, one second for runtime cleanup, separate two-second trace/metric/Sentry flush allowances, and the logging worker's bounded shutdown. If increasing export or shutdown limits, increase the deployment grace accordingly. The supplied Kubernetes deployment uses a native sidecar: it keeps OCR available until the main Tika/gateway container has drained, then sends OCR SIGTERM. Do not add an OCR preStop drain hook that removes this dependency before accepted Tika parses finish. For a separately deployed OCR service, the executable `drain` command can remove readiness before SIGTERM; any preStop time counts inside its termination grace period. See [Kubernetes Pod termination](https://kubernetes.io/docs/concepts/workloads/pods/pod-lifecycle/#pod-termination).

## Tracing, metrics and Sentry

For the request-scoped Java hooks, complete cross-process trace graph, build materials and real native acceptance command, see [distributed tracing](distributed-tracing.md).

JSON logs are always available on stderr. They have finite event/outcome names, request IDs, nested span context, worker identity, model revision, stage times, line/frame counts and batch utilization. The bounded asynchronous log queue is lossy under pressure so a blocked log sink does not stop OCR; a shutdown diagnostic reports dropped lines. `--log-filter` controls the JSON log targets and levels only: trace export has its own INFO floor and the SDK sampler, so a quiet log filter never disables tracing. Exporter failures and dropped spans are reported by the SDK itself under `opentelemetry*` targets in the same JSON stream (`--log-filter info,opentelemetry=warn` shows them). The first terminal transition logs one `service_terminal` event with its finite reason, phase and worker and produces at most one terminal-failure report per process; a panic or a failed `models` command reports on its own. A panic under the abort policy writes one synchronous `panic` line with thread, file and line, reports the `panic` failure class and flushes the bounded Sentry queue within the export timeout before the process aborts. The hook replaces std's plain-text panic report, so the panic payload, which may quote document data, reaches neither stderr nor Sentry. `models fetch` and `models verify` run with the same logging and reporting, so an init container failure carries a finite `model_cache_*` class.

Runtime events, including the synchronous lines written outside the log queue, have the envelope `{"timestamp":"<RFC 3339 UTC>","level":…,"fields":{"event":…},"target":…}`; an event inside a span also carries `span` and `spans`. SDK diagnostics under `opentelemetry*` targets carry their name in `fields.name` instead of `fields.event`. The table lists every WARN and ERROR event and the two per-request INFO events:

| `fields.event` | Level | Writer | Meaning |
|---|---|---|---|
| `service_terminal` | ERROR | indra-server | First terminal transition: finite reason, phase and worker |
| `service_failure` | ERROR | indra-server, indra-gateway | Reported failure with finite `failure_class` and `phase`; Sentry receives the same class |
| `tika_exchange_failed` | ERROR | indra-gateway | A retained Tika exchange failed; `error.type` is its finite class |
| `inference_rejected` | WARN | indra-server | The engine rejected one request without a terminal failure (invalid or unsupported image, resource limit, deadline); finite `error_code` |
| `accept_failed` | WARN | indra-gateway | Accepting a TCP connection failed; `kind` is the I/O error kind, and the listener retries after 100 ms |
| `socket_option` | WARN | indra-gateway | Setting `TCP_NODELAY` on an accepted connection failed; `kind` is the I/O error kind |
| `panic` | ERROR | indra-server, indra-gateway (synchronous) | Panic hook: thread, file and line; the process aborts afterwards |
| `log_queue_overflow` | WARN | indra-server, indra-gateway (synchronous, at shutdown) | `dropped_lines` lost by the bounded JSON log queue |
| `tika_operation_failed` | ERROR | Tika overlay (synchronous) | A traced parse or OCR process failed: `operation`, `error.type`, trace and span IDs |
| `tika_trace_sdk_warning` | WARN | Tika overlay (synchronous) | Java OTLP SDK warnings; `count` is logged at 1, 2, 4, 8 … |
| `tika_trace_shutdown_incomplete` | WARN | Tika overlay (synchronous) | The Java span flush did not complete successfully within its four-second shutdown budget |
| `tika_log` | Upstream log4j2 level | Apache Tika in the Tika image | One upstream Tika log event: `message`, `thread`, the logger as `target`, and `exception` when present |
| `http_request_finished` | INFO | indra-server | One OCR handler finished: request ID, finite outcome, elapsed time |
| `inference_finished` | INFO | indra-server | Statistics of one successful native call |

All other events are INFO or DEBUG; the INFO ones mark lifecycle steps (`listener_bound`, `worker_warmed`, `service_ready`, `drain_started`, `drain_complete`, `models_verified`, `gateway_ready`, `gateway_drain`, `child_reaped` and others). The Tika image writes upstream Tika's log4j2 events in the same envelope as `tika_log`; see [distributed tracing](distributed-tracing.md).

Set `telemetry.otlp_endpoint` to an OTLP/HTTP base URL, such as `http://otel-collector:4318`, to export **actual protobuf traces and metrics** to `/v1/traces` and `/v1/metrics`. The optional authorization value is sent only as the exporter header and is accepted over plaintext `http` only for a loopback collector; a remote collector needs `https`. Traces include the server request, queued/native execution, startup and native model stages. The `POST /ocr` server span records `http.response.status_code` and, for an error, the finite `error.type`; a 5xx response (`queue_full`, `unavailable`, `deadline`, `native_failure`, `internal`) also sets its span status to Error, while 4xx responses leave it Unset. Async request spans are instrumented per poll; entered span guards are confined to synchronous native work. The parser validates W3C version-00 `traceparent`, bounded `tracestate`, nonzero IDs and lowercase hexadecimal syntax. Invalid context starts a new trace. Baggage is ignored. Request JSON logs and Sentry failures carry trace/span IDs when OTLP is enabled.

Every exported resource has exactly `service.name`, `service.version`, `service.instance.id` and `deployment.environment.name`. The instance ID is a random 128-bit hex value generated once per process and shared by traces and metrics; it contains no host or pod data, so replicas and restarted processes export separate metric streams. `OTEL_RESOURCE_ATTRIBUTES` is not read: add pod or node identity in the collector, for example with its `k8sattributes` processor. `telemetry.environment` defaults to `local`; set it per deployment (the gateway reads `TIKA__TELEMETRY__ENVIRONMENT`), because it is also the Sentry environment. Release images bake in the source commit as SemVer build metadata: `service.version` is the crate version plus the commit, such as `0.2.0+<commit>`, and the Sentry release is `<service.name>@<service.version>`, such as `indra-ocr@0.2.0+<commit>`. Local builds without the `SOURCE_REVISION` build argument report the crate version alone.

The local sampling ratio controls new root traces. Existing parent sampling decisions are preserved across gateway, Java and native process hops. Default trace sampling is 10%; exporters use bounded queues/batches, explicit connection/request deadlines and dedicated transport threads. Slow exporters do not enlarge admission. See the [W3C Trace Context specification](https://www.w3.org/TR/trace-context/), [SDK batch processor](https://docs.rs/opentelemetry_sdk/0.32.0/opentelemetry_sdk/trace/struct.BatchSpanProcessor.html), and [concrete exporter wiring](../runtime/crates/indra-telemetry/src/otlp.rs).

| Instrument | Use |
|---|---|
| `indra.ocr.requests`, `indra.ocr.request.duration` | Handler outcomes and latency; finite outcome label |
| `indra.ocr.active` | Upload/queue/native/response ownership currently admitted |
| `indra.ocr.queue.duration` | Time waiting for the native worker |
| `indra.ocr.stage.duration` | Decode, orientation, detection, recognition, math, order and assembly |
| `indra.ocr.recognition.batches` | Actual text batches by row count, 1–64 |
| `indra.ocr.recognition.columns` | Valid versus padded columns; quantify padding efficiency |
| `indra.ocr.frames`, `indra.ocr.lines` | Actual processed page and line counts |
| `indra.ocr.input.bytes`, `indra.ocr.output.bytes` | Successful native request/response byte totals |

All duration histograms, including the gateway's `indra.gateway.request.duration` and `indra.gateway.backend.duration`, record seconds with the same explicit bucket boundaries: 1 ms to 300 s, with 0.5 s steps between 1 and 2.5 s (`SECONDS` in [otlp.rs](../runtime/crates/indra-telemetry/src/otlp.rs)). Longer observations fall into the overflow bucket. Handler latency ends when a response is constructed, before the client finishes receiving it. Active ownership includes retained response bytes, so it can remain positive afterward. Stage/batch metrics are emitted for successful native calls; failed calls still produce handler outcomes and contextual failure events. IDs and text never appear as metric labels.

Set `telemetry.sentry_dsn` to send contextual error envelopes for terminal model/native/watchdog failures. Events contain a finite failure class, phase, worker, elapsed time, request/trace IDs when available (as tags and as Sentry's native trace context, so the event links to the OTLP trace), and phase-specific breadcrumbs. Invalid images and ordinary queue overload are request errors, not Sentry incidents. Default Sentry integrations, request bodies, PII, hostname and automatic stack traces are disabled; native exception strings can contain input fragments and are never stringified for telemetry. A concrete reqwest transport preserves normal `NO_PROXY` handling, enforces the configured timeout, disables redirects and retains TLS verification. The bounded Sentry queue flushes during orderly exit. See [Sentry ClientOptions](https://docs.rs/sentry/0.49.3/sentry/struct.ClientOptions.html) and [event construction](../runtime/crates/indra-telemetry/src/errors.rs).

The shipped Tika source overlay creates request-scoped Java parse and OCR-process spans, then passes validated context to the Rust adapter through `INDRA_TRACEPARENT` and `INDRA_TRACESTATE`. With the exporters enabled, these spans connect gateway and Java activity to native inference across the adapter boundary, as described and tested in [distributed tracing](distributed-tracing.md).

Only validated trace context crosses the telemetry boundary from inbound headers. Do not put document content or credentials in `tracestate`. Images, OCR text, filenames, authorization headers and baggage are absent from logs/events. Dedicated tensor parity dumps are an explicit engine diagnostic facility and must be handled as document data; they are not production telemetry.

OTLP and Sentry are independently compiled features, enabled in the normal server build but dormant until configured. `--no-default-features` retains JSON logging and rejects any setting that requests omitted telemetry support. TLS uses Rustls with the final image's CA trust store. The service introduces no application trait objects, growable request queues, payload clones or blanket `anyhow` errors. External HTTP/telemetry libraries retain their own typed ownership and internal allocation mechanisms.

## Verification and measurement

```sh
cargo test --locked --manifest-path runtime/Cargo.toml \
  -p indra-server --all-features --lib
cargo test --locked --manifest-path runtime/Cargo.toml \
  -p indra-telemetry --all-features
cargo clippy --locked --manifest-path runtime/Cargo.toml \
  -p indra-server -p indra-telemetry --all-features --all-targets -- -D warnings
cargo bench --locked --manifest-path runtime/Cargo.toml \
  -p indra-telemetry --bench propagation

INDRA_TEST_MODEL_DIR=/models/ed345c7f158eddbc7dee0c6a7269afb75a94f7e8 \
  cargo test --locked --manifest-path runtime/Cargo.toml \
  -p indra-server --test lifecycle_native -- --ignored --test-threads=1
INDRA_TEST_MODEL_DIR=/models/ed345c7f158eddbc7dee0c6a7269afb75a94f7e8 \
  cargo test --locked --release --manifest-path runtime/Cargo.toml \
  -p indra-server --test service_allocations
```

Service tests exercise saturation without readiness churn, framing, source precedence, safe diagnostics, remote drain rejection, accepted-work draining, timeout/disconnect ownership and final response-byte lifetime. A deterministic supervisor watchdog test holds a reservation in a native thread and proves timeout cannot release it. The separate ignored native process test warms real models, holds an accepted upload across CLI drain and SIGTERM, then verifies exact native OCR (`INVOICE 12345`) and successful process exit. It needs real native linkage and the verified model cache. `INDRA_TEST_SERVER_BIN=/indra-server` overrides the build-time executable path when the harness is copied into scratch; `INDRA_TEST_THREADS=1` or `2` controls both ORT and OpenCV, and the harness records those settings. SIGTERM uses the pinned rustix process API directly, without a shell or external `kill` executable. Run both thread settings under their respective whole-container CPU limits; a thread flag does not establish a CPU quota. The standalone `service_allocations` executable serves the same snapshot in-process and prints one JSON line of six allocator counters per case; its ceilings were observed on linux/amd64 at opt-level 3 without LTO, with `indra-engine` replaced by a stub that none of the measured requests calls; aarch64 with the release profile's thin LTO gave the same counts with 8 fewer bytes. The native service workflow runs it as `/gates/server-allocations` from the acceptance image under both CPU limits.

The telemetry wire test runs real SDK exporters against a local HTTP collector, decodes OTLP protobuf and Sentry envelopes, checks W3C parent IDs/child spans, span status, histogram buckets and the actual instruments, and rejects request/PII/hostname fields. It runs the exporters in a re-executed child process and requires that child's request JSON log line on stderr to carry the request ID, the incoming trace ID and the exported server span ID. The panic test re-executes itself, panics on a thread whose name needs JSON escaping and aborts without an orderly shutdown; it requires a single JSON `panic` line on stderr, a Sentry envelope tagged `failure.class=panic` and `ocr.phase=process`, and the payload in neither. A separate allocation test proves that 10,000 strict traceparent parses allocate, reallocate and free nothing. The Criterion case measures that parser alone.

Native parity, whole-stack speed and OCR quality remain separate gates. Measure matched Tika 3.3.2 rasterization, exact fixtures/model hashes and all comparison arms under the same total one-CPU and two-CPU limits. Record memory, affinity, throttling, warmup, threads, spinning, batch histograms, padding and failures. Disable external exports for baseline timing, then measure the overhead of chosen production instrumentation as a separate paired arm. Use the profiling build and flame graphs to investigate observed bottlenecks; a parser microbenchmark cannot establish an OCR throughput claim.
