# Tika shutdown that preserves accepted parses

Run `indra-gateway` as the Tika container entrypoint. It owns public HTTP port
9998 and starts the pinned Tika 3.3.2 JVM privately on `127.0.0.1:9999`. It stops
new admission on SIGTERM, completes accepted parses and response transmission,
then terminates and reaps Java and its descendants. The Rust OCR service remains
ready throughout that Tika drain because a multi-page parse can request another
OCR page after shutdown begins. The same gateway belongs in every measured arm,
including Tesseract and the Python reference.

## Why Tika needs a supervisor

In the exact Tika 3.3.2 commit
[`b8a6916eab70ccdb5d4551c69be1a46af29c2cff`](https://github.com/apache/tika/tree/b8a6916eab70ccdb5d4551c69be1a46af29c2cff),
the watchdog's JVM shutdown hook forcibly destroys its forked parser processes.
Giving that JVM SIGTERM first can therefore abort an HTTP parse while its OCR
sidecar is still draining correctly. The apparent forked-process shutdown grace
setting is inside an unimplemented TODO comment. These are source-level facts,
not assumptions about a future Tika release: see
[`TikaServerWatchDog.java` lines 59–75](https://github.com/apache/tika/blob/b8a6916eab70ccdb5d4551c69be1a46af29c2cff/tika-server/tika-server-core/src/main/java/org/apache/tika/server/core/TikaServerWatchDog.java#L59-L75)
and [`TikaServerConfig.java` lines 91–105](https://github.com/apache/tika/blob/b8a6916eab70ccdb5d4551c69be1a46af29c2cff/tika-server/tika-server-core/src/main/java/org/apache/tika/server/core/TikaServerConfig.java#L91-L105).

The gateway retains Tika's forked parser mode and its existing parser limits. It
does not enable the unsafe status/async/pipes endpoints. Forked-child recycling
is disabled (`maxFiles` is `-1` in every shipped Tika XML): Tika would otherwise
restart its parser child after 10,000 files, and the gateway must treat the
resulting mid-parse EOF as a terminal upstream failure because it cannot be told
apart from a crash. Process hygiene is the orchestrator's restart policy, not a
scheduled child restart that drops accepted work. Its Java watchdog starts
with explicit bounds:

```text
/usr/bin/java -Xms32m -Xmx128m -XX:ActiveProcessorCount=1
  -XX:+UseSerialGC -Djava.awt.headless=true -Djava.io.tmpdir=/tmp
  -jar /opt/tika/tika-server.jar --host 127.0.0.1 --port 9999
  --config /etc/tika/tika-config.xml
```

These are separate arguments passed directly to `Command`, without a shell. The
forked parser JVM has its own limits in the Tika XML. Container CPU and memory
limits must include gateway, watchdog JVM, parser JVM, and OCR processes. A
JVM/ORT thread setting alone does not enforce the requested total CPU budget.

## Development and commands

The gateway has no ONNX Runtime, OpenCV or model-cache dependency. It can be built
and tested before the native OCR engine is available:

```sh
cargo build --locked --manifest-path runtime/Cargo.toml -p indra-gateway
cargo test --locked --manifest-path runtime/Cargo.toml -p indra-gateway

runtime/target/debug/indra-gateway \
  --java /usr/bin/java \
  --jar /absolute/path/tika-server-standard-3.3.2.jar \
  --tika-config /absolute/path/tika-config.xml serve

runtime/target/debug/indra-gateway probe startup
runtime/target/debug/indra-gateway probe ready
runtime/target/debug/indra-gateway probe health
runtime/target/debug/indra-gateway drain
runtime/target/debug/indra-gateway openapi > gateway-openapi.json
runtime/target/debug/indra-gateway completions bash > indra-gateway.bash
```

`serve` is also the default command. `probe` and `drain` use the executable's own
bounded HTTP client; final images need no shell, curl, or interpreter for health
checks. A wildcard listen address maps to IPv4 loopback when probing. For example,
`/indra-gateway --listen 127.0.0.1:8081 probe ready` can test the OCR service's
compatible readiness endpoint. The command needs a successful status, without
assuming a particular response body format.

Debug builds load `.env` before starting threads. Release builds never read it.
Missing `.env` is accepted; malformed `.env` is an explicit startup error.

## Configuration

The complete schema is in [gateway.example.toml](../runtime/gateway.example.toml).
Use `indra-gateway --config /etc/tika/gateway.toml check-config`. Precedence is
**defaults → explicit TOML file → `TIKA__SECTION__FIELD` environment → flags**,
applied before typed decoding. Unknown fields and missing explicitly supplied
files fail startup. Diagnostics preserve safe key/type information and TOML byte
positions; raw values and source snippets are redacted. Every example setting has
a global CLI flag using its final field name with hyphens.

| Setting | Default | Purpose |
| --- | --- | --- |
| `gateway.listen` | `0.0.0.0:9998` | Public IPv4 HTTP listener |
| `gateway.backend` | `127.0.0.1:9999` | Private IPv4 loopback Tika endpoint |
| `gateway.ocr_ready_endpoint` | Unset | Optional native OCR IPv4 loopback readiness dependency; set to `127.0.0.1:8081` for the native stack |
| `gateway.max_active` | 2 | Uploads, retained backend parses and response owners combined; maximum 8 |
| `gateway.max_connections` | 16 | Includes idle/header/response sockets; maximum 64 |
| `gateway.max_request_bytes` | 64 MiB | One preallocated upload buffer per active slot; maximum 128 MiB |
| `gateway.max_response_bytes` | 16 MiB | Counted streaming response limit; maximum 64 MiB |
| `gateway.header_timeout_seconds` | 5 | Complete the first HTTP header within this absolute bound |
| `gateway.upload_timeout_seconds` | 15 | Complete a validated upload before starting Java |
| `gateway.request_timeout_seconds` | 300 | One absolute deadline across upload and backend response |
| `gateway.client_stall_seconds` | 5 | Stop waiting on a full public response channel and drain backend to a sink |
| `gateway.startup_timeout_seconds` | 180 | Verify the private endpoint reports exactly Tika 3.3.2 |
| `gateway.shutdown_timeout_seconds` | 315 | Final drain watchdog; must cover request deadline plus client stall |
| `gateway.child_shutdown_seconds` | 8 | Each of the TERM and KILL/reap allowances |

For example, `TIKA__GATEWAY__MAX_ACTIVE=1` and `--max-active 1` select the same
field. `TIKA__TELEMETRY__OTLP_ENDPOINT` maps to `telemetry.otlp_endpoint` and
`--otlp-endpoint`. Keep credentials in injected environment or a mounted config
secret rather than process arguments. Config is immutable while serving.

Native deployments set `--ocr-ready-endpoint 127.0.0.1:8081` or
`TIKA__GATEWAY__OCR_READY_ENDPOINT=127.0.0.1:8081`. The supervisor probes the
configured `/readyz` and Tika's private version endpoint together, at most once
per second, each under a 500 ms absolute deadline and a 1 KiB body limit. OCR must
return complete JSON with `phase: "ready"` and HTTP 200. This includes a warmed,
verified model service and remains true during transient OCR queue saturation.
An unavailable dependency closes admission and readiness, while gateway liveness
continues to describe the supervisor. A dependency that answered stays ready
through two consecutive probe timeouts, so one slow answer under the shared CPU
quota does not withdraw readiness; the third consecutive timeout does. A refused
connection, a non-200 status or an OCR phase other than `ready` withdraws it
with the probe that sees it, because work admitted to a Tika that refuses
connections would fail terminally. Each Tika or OCR readiness change logs
`tika_dependency_changed` or `ocr_dependency_changed`. Accepted work retains its
original ownership and deadlines. No background task accumulates between probe
cycles.
The setting is absent by default for the Tesseract arm; the supplied native
Compose and Kubernetes manifests explicitly enable it.

## HTTP and ownership contract

`GET /readyz`, `/healthz`, `/startupz`, `POST /drain`, and
`GET /openapi.json` are gateway operations. Readiness requires successful pinned
Tika startup, a recent successful private version probe, a ready OCR dependency
when configured, and open admission. The JSON `ocr_ready` is `null` when this
dependency is not configured and otherwise reports its last probe result. A
momentarily full parse capacity does not remove readiness. Normal drain removes
readiness immediately while health stays successful. A terminal error removes
both readiness and health. Only an actual IPv4 loopback socket peer can call
`/drain`; forwarded headers cannot grant access.

Other enabled Tika paths and methods are forwarded under the same admission
rules. Keep the backend private. The gateway accepts HTTP/1 uploads with one exact
decimal `Content-Length`; GET, HEAD and OPTIONS can omit it for an empty body.
Chunked uploads, trailers, upgrades, ambiguous lengths, and oversized headers are
rejected. Tika request headers such as content type, Accept and supported parser
configuration headers are preserved. Standard hop-by-hop headers and fields
named by `Connection` are removed; the upstream Host is the configured loopback
endpoint. Every public response closes its HTTP connection.

The admission CAS and drain transition share the core lifecycle word. Each
accepted parse has three independent owners: its pooled input buffer, retained
backend operation, and frontend response. Capacity is released only after all
three are gone. An upload accepted before drain can still finish and start its
backend parse afterward. A client that disconnects during an incomplete upload
has started no Java parse; a disconnect after Java starts turns its response into
a bounded sink until backend EOF or the original deadline.

Uploads are staged completely into the slot's existing buffer, then exposed to
Hyper through `Bytes::from_owner`; the original storage returns to the pool only
after the final library reference drops. Default persistent input capacity is
2 × 64 MiB. Responses use 16 KiB slices, one queued channel frame, and one frame of
lookbehind. That lookbehind identifies the final chunk, which carries the
frontend owner through Hyper's final transport `Bytes` reference. Dropping the
HTTP handler/body alone cannot prematurely release that final response. Internal
Hyper buffers, OS socket buffers, channel metadata and telemetry have their own
bounded or library-owned storage; this is not an allocation-free process claim.

The response stream requires an explicit backend completion marker. EOF after a
partial upstream response becomes a body error, including when the public
response uses chunked framing. No successful final chunk is synthesized for a
truncated document. If a status or size error is found before response headers,
the gateway returns JSON; after headers, it aborts the stream. Tika's own complete
HTTP error responses are preserved. They do not by themselves imply the JVM is
unhealthy. Saturated/unavailable admission returns 503 with `Retry-After: 1`; the
refusal reason (`tika_unready`, `ocr_unready`, `saturated`, `draining`, or the
terminal `admission_invariant`) is the request's outcome, and every
OCR-readiness transition logs `ocr_dependency_changed`. The Java child never
inherits the gateway's own `TIKA__*` variables (collector credentials, Sentry
DSN).

Every proxied request, refused or admitted, returns `X-Request-ID` and ends with
one INFO JSON log event `gateway_request_finished` with that `request_id`, its
finite `outcome` and `elapsed_ms`. The same outcome labels the
`indra.gateway.requests` metric. An admitted request writes the event when its
frontend owner is released, normally with the final response bytes; outcomes
are `complete`, `tika_status_error` (Tika's own error status passed through),
`client_cancelled`, `response_incomplete`, or the error codes of the JSON
rejections (`invalid_headers`, `content_length_required`, `invalid_upload`,
`request_too_large`, `deadline_exceeded`, `tika_failure`, `response_too_large`).

An ambiguous upstream connection failure or deadline is terminal: a closed socket
does not prove Java stopped parsing. The gateway stops admission and retains the
backend lease while its supervisor terminates and reaps the owned group. It
exits unsuccessfully for the orchestrator to restart. This conservative rule can
restart the container after an upstream failure that Tika might otherwise recover
from internally; it prevents invisible abandoned work from silently defeating
the active-parse limit. Ordinary frontend disconnects do not trigger that restart.
See [proxy.rs](../runtime/crates/indra-gateway/src/proxy.rs) and
[service.rs](../runtime/crates/indra-gateway/src/service.rs).

### Tika OCR request contract

Tika 3.3.2's `TesseractOCRParser` builds the OCR command line from its XML
parameters, which a request can override with `X-Tika-OCR<Field>` headers.
The bridge implements only the command line of the shipped XML and exits with
status 1 for anything else; Indra selects its script models and layout itself,
so the other values have no equivalent ([`parse_args`](../src/main.rs)).

| XML parameter / request header | Accepted | Rejected by the bridge |
| --- | --- | --- |
| `language` / `X-Tika-OCRLanguage` | empty | any language (`-l`) |
| `pageSegMode` / `X-Tika-OCRPageSegMode` | `3` | every other mode |
| `outputType` / `X-Tika-OCROutputType` | `txt` | `hocr` |
| `preserveInterwordSpacing` / `X-Tika-OCRPreserveInterwordSpacing` | `false` | `true` |
| `pageSeparator` / `X-Tika-OCRPageSeparator` | empty | any separator |
| `otherTesseractSettings` (XML only) | none | extra `-c` settings |

Settings that Tika applies itself, such as `X-Tika-OCRSkipOcr`,
`X-Tika-OCRTimeoutSeconds` and the minimum and maximum file size to OCR, do not
reach the bridge and keep their Tika meaning. Values Tika itself rejects, such
as an unknown `X-Tika-OCR*` header or a page segmentation mode outside 0–13,
return HTTP 400 before parsing starts.

A setting the bridge rejects fails the parse. For an image, Tika raises
`TesseractOCRParser bad exit value 1 err msg: <bridge message>`, logs it as
`Text extraction failed`, and answers HTTP 422 with an empty body (the shipped
XML leaves `returnStackTrace` off). For a PDF, the first page that needs OCR
fails the whole parse: Tika wraps the bridge error in
`Unable to extract PDF content`, logs and answers the same way, and returns no
text for the other pages. The real-stack trace gate sends a TIFF to
`PUT /tika` with `X-Tika-OCROutputType: hocr` and requires the 422
([`trace_acceptance.rs`](../runtime/crates/indra-gateway/examples/trace_acceptance.rs));
the bridge's unit tests reject `--psm 0`, `hocr`, `-l` and
`preserve_interword_spaces=1`.

## Shutdown ordering and budgets

The gateway creates a separate process group for Java and enables Linux child
subreaping, including when it is not PID 1 during local development. Its event
loop reaps exited children, detects an unexpected watchdog exit, and polls Tika
availability without spawning an unbounded stream of tasks. After normal drain,
it sends TERM to its own Java group, waits/reaps, and escalates to KILL only if the
configured child allowance expires. It never signals an unrelated process group.
See [process.rs](../runtime/crates/indra-gateway/src/process.rs),
[Rust's process-group API](https://doc.rust-lang.org/std/os/unix/process/trait.CommandExt.html#tymethod.process_group),
and [Linux subreaping](https://man7.org/linux/man-pages/man2/PR_SET_CHILD_SUBREAPER.2const.html).

Kubernetes must terminate the main Tika/gateway container before the OCR native
sidecar. The native sidecar continues accepting later page requests while Tika
drains. Compose must stop the dependent Tika/gateway service before its OCR
dependency. Use the repository's deployment manifests, including their shared
CPU/memory budgets. Do not add a preStop sleep as a substitute for retained work.

With defaults, budget up to 315 seconds for gateway drain, 16 seconds for Java
TERM/KILL/reaping, 6 seconds for final HTTP shutdown, one second for Tokio shutdown,
and up to eight seconds for trace/metric/Sentry/log flushing. This is a conservative
346-second main-container allowance. Compose grants 350 seconds; Kubernetes
grants 450 seconds overall, covering the main allowance followed by 100 seconds
for the OCR sidecar, with four seconds of margin. These configured timeout
budgets are not a hard real-time guarantee under arbitrary host stalls. Most drains finish as soon as accepted
work completes. Reduce related request, parser and shutdown settings together if
your product has a shorter timeout contract. Kubernetes and Compose termination
semantics are documented in their
[pod termination](https://kubernetes.io/docs/concepts/workloads/pods/pod-lifecycle/#pod-termination)
and [startup/shutdown order](https://docs.docker.com/compose/how-tos/startup-order/)
references.

## Telemetry and verification

The gateway shares the verified telemetry implementation described in
[runtime-service.md](runtime-service.md). It emits JSON tracing context, consumes
validated W3C parents, exports real OTLP spans/metrics when configured, and reports
contextual Sentry failures with bounded queues and flush deadlines. Service name
defaults to `tika-gateway`. Raw URLs, query strings, document names, bodies and text
are not log/trace fields; trace routes and outcomes use finite labels. Baggage is
not propagated.

`indra.gateway.requests` and `indra.gateway.request.duration` describe public
request outcomes, including cancellation. `indra.gateway.backend.requests` and
`indra.gateway.backend.duration` separately describe retained Tika work, including
work completed after client disconnect. `indra.gateway.active` includes all three
owners. OCR-stage metrics remain under `indra.ocr.*`; a Tika metadata request is
not counted as an OCR inference. The first terminal request failure reaches Sentry
inside its request span with the generated request ID; supervisor failures without
a request use their finite process/startup class. Each failure is reported once.
That ID, also returned as `X-Request-ID`, is 16 lowercase hex digits counting up
from the gateway's startup time in nanoseconds since the UNIX epoch, so a
restarted gateway does not repeat earlier IDs. It is a correlation value, not a
secret.

The shipped Tika 3.3.2 source overlay propagates the gateway span through
request-scoped Java parser and OCR-process spans to the Rust bridge and native
inference. It never mutates the global process environment. Java export is
configured separately and remains disabled by default. See
[distributed tracing](distributed-tracing.md) for the exact overlay identity,
bounded exporter settings, custom parser XML and actual full-stack OTLP gate.

The automatically run Rust process harness tests a controlled HTTP child, its
grandchild and real gateway signals: held-upload SIGTERM/CLI drain, readiness
under saturation, disconnect retention, truncated post-header responses, header
deadlines, request deadlines, version mismatch, and orphan reaping. Unit tests
cover final transport-byte lifetime, a deliberately stalled channel, framing,
configuration precedence/redaction and loopback-only drain. No model or external
daemon is required. These are process/protocol gates; actual Tika and OCR quality,
native parity, static linkage and whole-stack performance have separate gates.

```sh
cargo test --locked --manifest-path runtime/Cargo.toml -p indra-gateway
cargo clippy --locked --manifest-path runtime/Cargo.toml \
  -p indra-gateway --all-features --all-targets -- -D warnings
cargo test --locked --manifest-path runtime/Cargo.toml \
  -p indra-gateway --no-default-features
```

## Real stack shutdown acceptance

[`stack_acceptance.rs`](../runtime/crates/indra-gateway/examples/stack_acceptance.rs)
tests the caller's already started, otherwise idle **native** stack. It uses the
existing two-page TIFF, whose pinned SHA-256 is
`303ebb45259b96fecc0b200edf8e0a68ed50e2b43f441bceef8694e43699c329`.
The two page texts are exactly `INVOICE 12345` and `SECOND PAGE 67890`.

The gate verifies public Tika 3.3.2, configured OCR readiness, and a complete
initial TIFF parse. Its initial text assertion permits only CR, LF and form-feed
delimiters around the two exact ordered lines; missing, duplicate, reordered or
additional text fails. It then starts the same upload again and withholds all but
the first byte. Only after `/readyz` reports this accepted request does it invoke
the explicitly selected orchestration command. It requires an actual HTTP 503
with `phase: "draining"` and the request still active; a broken connection does
not count as proof. It also requires a new document request to receive 503.
The held upload then completes. The resulting HTTP body must contain both exact
pages and match the initial Tika body **byte for byte**, including every delimiter.
Finally the orchestration command must finish successfully before its deadline.

Start the pinned native Compose stack using a unique test project, then run:

```sh
cargo run --locked --manifest-path runtime/Cargo.toml \
  -p indra-gateway --example stack_acceptance -- \
  --endpoint 127.0.0.1:9998 --output shutdown-1cpu.json --cpus 1 \
  compose --project indra-shutdown-1 --file compose.yaml

# Start a fresh two-CPU project with the same files supplied to this command.
cargo run --locked --manifest-path runtime/Cargo.toml \
  -p indra-gateway --example stack_acceptance -- \
  --endpoint 127.0.0.1:9998 --output shutdown-2cpu.json --cpus 2 \
  compose --project indra-shutdown-2 --file compose.yaml \
  --override-file compose.2cpu.yaml
```

The Compose mode invokes `docker compose --project-name NAME --file FILE ... stop`.
It preserves each service's deployed `stop_grace_period` and reverse dependency
ordering and leaves stopped containers and volumes available for inspection.
An optional `--second-override-file` and `--docker-context` select additional
explicit inputs. See the official
[`docker compose stop` reference](https://docs.docker.com/reference/cli/docker/compose/stop/).

For Kubernetes, use a dedicated test pod and an address that continues to reach
that **same** pod throughout termination. A Service that removes the terminating
endpoint can hide the gateway's explicit drain response, and a replacement pod
must not answer these probes. Run from a host that can reach the direct Pod IP,
or establish and independently validate a connection path that remains attached
to the original pod:

```sh
cargo run --locked --manifest-path runtime/Cargo.toml \
  -p indra-gateway --example stack_acceptance -- \
  --endpoint 10.244.0.12:9998 --output shutdown-kubernetes.json --cpus 1 \
  kubernetes --context acceptance-cluster --namespace ocr-test --pod tika-test
```

This mode invokes `kubectl --context CONTEXT --namespace NAMESPACE delete pod POD
--wait=true --timeout=600s`, with a bounded API request timeout. It leaves the
deployed pod termination grace unchanged and never uses force deletion. The
main-container/native-sidecar order is therefore the deployed order being
tested. See the official
[`kubectl delete` reference](https://kubernetes.io/docs/reference/kubectl/generated/kubectl_delete/).

Both modes use direct argument arrays, without a shell or arbitrary command
string. The gateway must allow its upload to remain open for the observation
period: defaults are eight seconds for drain observation and fifteen seconds
for the gateway upload deadline. Full HTTP exchanges default to 300 seconds;
command completion defaults to 600 seconds. The new JSON report file is reserved
before any shutdown action, never overwrites existing evidence, and records
passed/failed stage, complete text, SHA-256s, readiness delay and command status.
HTTP response storage is fixed at 8 KiB, including rejection and version replies.
On harness failure, the harness terminates its own CLI process group and reaps
its direct CLI child within a bounded allowance. Docker/API actions already
requested can continue, and the caller retains
responsibility for inspecting and cleaning up that dedicated test stack.

The `--cpus` value records the caller's selected limit. The harness does not
pretend to audit CPU quota or identify a container digest from OCR text. Preserve
the actual native image digests, container/Pod configuration, cgroup evidence,
logs and this report together. Running the Rust example's unit tests proves its
fixture, held-body protocol and explicit command construction; only executing
it against the pinned real stack proves the deployment behavior:

```sh
cargo test --locked --manifest-path runtime/Cargo.toml \
  -p indra-gateway --example stack_acceptance
```
