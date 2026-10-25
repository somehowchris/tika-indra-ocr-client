# Trace an actual Tika parse

The Tika image contains Apache Tika **3.3.2** plus the explicit
`3.3.2-indra-trace.1` source overlay. This is a modified Apache distribution.
Its manifest preserves the base version used by the gateway's compatibility
probe and separately records the overlay version, exact upstream commit,
original SHA-512 and extension/patch-set SHA-256. It is not a Java agent.

Enable the gateway and native Rust exporters with `--otlp-endpoint
http://127.0.0.1:4318 --trace-sample-ratio 0.1`, or the corresponding
`telemetry` config/environment keys documented in [runtime-service.md](runtime-service.md).
For the Java parser container set:

```text
INDRA_TIKA_OTLP_ENDPOINT=http://127.0.0.1:4318/v1/traces
INDRA_TIKA_TRACE_SAMPLE_RATIO=0.1
```

Use HTTPS with an explicit port for a remote collector. Java accepts HTTP only
for literal loopback addresses. Optional `INDRA_TIKA_OTLP_AUTHORIZATION` supplies
one bounded authorization header; inject it through your orchestrator's secret
mechanism. Never put credentials in the endpoint URL. Authorization without an
endpoint is a startup error. The base parser works with tracing disabled.

The public gateway validates W3C version-00 `traceparent` and bounded
`tracestate`, starts its request span, and forwards that span to Java. Tika's
stateless `ParseContextConfig` provider attaches it to the individual request's
`ParseContext`. `TracedParser` brackets actual parser calls and restores the
previous context even on failure. The OCR process hook writes only the current
carrier into that child's `ProcessBuilder` environment. The Rust bridge forwards
it to native OCR, whose HTTP and inference spans retain exact parent edges.
The shipped base Tika XML enables these parser decorators. When supplying your
own parser XML, wrap each configured parser in `io.indra.tika.TracedParser` using
a nested `<parser>` element, as in `deploy/base/tika-config.xml`.
Nested parsers, successive OCR pages and concurrent requests share no ambient
thread-local scope or mutable global environment. Invalid/duplicate parents
start independent roots; invalid state is discarded while retaining a valid
parent. Baggage is discarded.

Java spans contain fixed operation names and bounded parser/exception class
names. The hook omits document bytes, filenames, URLs, exception messages and
stacks. Failure JSON includes the operation, finite class and trace/span IDs.
The overlay's `tika_operation_failed`, `tika_trace_sdk_warning` and
`tika_trace_shutdown_incomplete` lines use the same JSON envelope as the Rust
services (event table in the [runtime guide](runtime-service.md#tracing-metrics-and-sentry)).
Upstream Tika's log4j2 events use that envelope too. The image sets
`LOG4J_CONFIGURATION_FILE=/opt/tika/log4j2.xml` ([source](../tika-trace/log4j2.xml)),
which keeps Tika 3.3.2's bundled console configuration (root level INFO, one
stderr appender) and writes each event as one line with `fields.event`
`tika_log`, the message, the thread, the logger as `target` and, only when the
event has one, the stack trace in `fields.exception`. The forked parser JVM
inherits the variable, and so does the Tesseract comparison image built on this
one. These messages and stack traces are upstream Tika's own and are not
filtered: they can contain exception messages and paths, so apply your normal
restricted log storage/access policy to them. Anything a JVM writes to stderr
outside log4j stays plain text; parse a line as JSON only when it starts with
`{`. Rust Sentry integration reports finite
failure classes and correlation IDs as described in the runtime guide; Java does
not send a second Sentry event for the same parser failure.

The Java batch exporter defaults to 256 queued spans, batches of 64, a 1000 ms
interval and a 1000 ms export timeout. Tune with `INDRA_TIKA_EXPORT_QUEUE` (16–4096),
`INDRA_TIKA_EXPORT_BATCH` (1–512 and no larger than the queue),
`INDRA_TIKA_EXPORT_INTERVAL_MS` (100–60000) and
`INDRA_TIKA_EXPORT_TIMEOUT_MS` (100–5000). It uses two bounded export threads,
the SDK retry policy disabled, a 256 KiB request-body cap, bounded span fields and a four-second
shutdown budget. The pinned OkHttp sender may transparently recover a failed
connection within that same call timeout. SDK diagnostics become a bounded warning counter without
collector URLs, response bodies or exception text; this counter counts warning
records, not exact lost spans. Sampling honors existing parent decisions; the
configured ratio applies to new roots. A timeout/overflow can lose telemetry and
does not extend the parse deadline. Alert on the `tika_trace_sdk_warning` and
`tika_trace_shutdown_incomplete` events together with collector-side
rejected/dropped-span metrics.

## Reproduce acceptance

`docker/tika.Dockerfile` always builds the pinned overlay and runs executable Java
tests before constructing its distroless runtime. The builder fetches only locked
Maven artifacts and two exact Tika source files; verifies hashes and sizes;
applies patches with zero fuzz; compiles for Java 21 without annotation
processors; and creates jars with fixed entry timestamps. Original/patched
sources, source jars, POMs, build metadata, test logs and SHA-256 inventory remain
under `/opt/tika/materials`, `/opt/tika/evidence` and `/opt/tika/SHA256SUMS`.
The `trace-evidence` Buildx target exports that same complete bundle.

The Java tests cover shared transport vectors, real SPI provider ordering,
stale inherited environment removal, overlapping request contexts, exception
restoration, bounded real child-process shutdown, finished-JAR hook presence,
manifest classpath discovery, base version and configured parser execution. The
watchdog tests select the image's log4j2 configuration through
`LOG4J_CONFIGURATION_FILE` and require every upstream line from the real
watchdog and forked Tika JVMs in the JSON envelope.

Build `docker/gateway.Dockerfile --target gateway-acceptance` for the real trace
gate executable. After building the actual Tika, bridge and native images, use
immutable image references to assemble its isolated test image:

```sh
bash scripts/prepare-trace-helper.sh /absolute/gateway-acceptance-resolution \
  /absolute/new-trace-helper-evidence
docker build -f docker/trace-acceptance.Dockerfile \
  --build-arg TIKA_IMAGE="$TIKA_REF" \
  --build-arg NATIVE_IMAGE="$NATIVE_REF" \
  --build-context trace-helper=/absolute/new-trace-helper-evidence/context \
  -t indra-trace-acceptance:local .
TRACE_CPUSET_1=0 TRACE_CPUSET_2=0,1 \
  bash scripts/verify-trace-native.sh indra-trace-acceptance:local \
  /absolute/verified-model-cache/ed345c7f158eddbc7dee0c6a7269afb75a94f7e8 /absolute/new-trace-evidence
```

Pass the revision directory containing the verified model files. The trace gate
mounts that directory directly at /models and supplies it as --model-dir; the
fetch command creates the revision leaf below its cache parent.

The preparation script binds an owned, unstarted container to its verified daemon
root and extracts the static helper. It compares its SHA256 with the same
container’s build manifest and retains the root, container, ELF and cleanup
records. The named context contains only that helper. A daemon image ID is not
a registry reference for a Dockerfile FROM instruction.

Select CPU IDs from distinct physical cores on the measurement host. The script
runs 1-CPU and 2-CPU correctness gates serially, with the entire gateway, forked
Java, CLI bridge, native OCR, test driver and local OTLP sink in one 3 GiB cgroup
with zero swap and no network access. It binds daemon OCI identity to each actual
container and verifies configured limits, normal exit and absence of OOM kills.
These are correctness runs, not performance measurements.

The gate performs actual two-page OCR, overlaps independent requests, tests
missing/invalid/duplicate parents and malformed state, and rejects an unsupported
CLI output mode. It decodes real OTLP protobuf and checks exact gateway → Java
parse → OCR process → native HTTP → native inference ancestry, state isolation,
unique IDs, output bytes, errors and forbidden payloads. The gate uses 60-second export intervals, 512-span batches and an aggregate
capture cap of 512 spans. It requires both services to finish shutdown before the first export
interval; periodic/full-batch export cannot explain the completed graph, so
termination must flush the SDKs; ordinary child termination and
orphan reaping must complete. Raw protobuf, response hashes, process logs,
executable/JAR/dependency identities and the graph report are retained even when
the gate fails. Passing Java unit/build checks alone does not establish this
native end-to-end gate.

The overlay is deliberately limited to [the pinned Tika release sources](https://github.com/apache/tika/tree/b8a6916eab70ccdb5d4551c69be1a46af29c2cff).
Its transport follows [W3C Trace Context](https://www.w3.org/TR/trace-context/)
and its exporter uses the [OpenTelemetry Java SDK](https://opentelemetry.io/docs/languages/java/sdk/).

The [local packaging regression receipt](evidence/trace-helper-context-20261019/receipt.json)
retains the bare-ID failure, named-context success, unchanged helper bytes and
synthetic caller-path controls. This preparation proof does not establish the
real OTLP gate or signing acceptance.
