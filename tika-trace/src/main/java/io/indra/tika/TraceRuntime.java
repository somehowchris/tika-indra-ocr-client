/* SPDX-License-Identifier: Apache-2.0 */
package io.indra.tika;

import io.opentelemetry.api.common.AttributeKey;
import io.opentelemetry.api.common.Attributes;
import io.opentelemetry.api.metrics.MeterProvider;
import io.opentelemetry.api.trace.Span;
import io.opentelemetry.api.trace.SpanKind;
import io.opentelemetry.api.trace.StatusCode;
import io.opentelemetry.api.trace.Tracer;
import io.opentelemetry.api.trace.TracerProvider;
import io.opentelemetry.exporter.otlp.http.trace.OtlpHttpSpanExporter;
import io.opentelemetry.sdk.resources.Resource;
import io.opentelemetry.sdk.trace.SdkTracerProvider;
import io.opentelemetry.sdk.trace.SpanLimits;
import io.opentelemetry.sdk.trace.export.BatchSpanProcessor;
import io.opentelemetry.sdk.trace.samplers.Sampler;
import java.net.URI;
import java.net.URISyntaxException;
import java.time.Duration;
import java.util.concurrent.ArrayBlockingQueue;
import java.util.concurrent.ThreadPoolExecutor;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicLong;
import java.util.logging.Handler;
import java.util.logging.Level;
import java.util.logging.LogRecord;
import java.util.logging.Logger;

/** Explicit SDK ownership. Disabled by default; no global SDK or ambient request scope. */
final class TraceRuntime {
    private static final String VERSION = "3.3.2-indra-trace.1";
    private static final AttributeKey<String> EXCEPTION_TYPE = AttributeKey.stringKey("exception.type");
    private static final AtomicLong SDK_WARNINGS = new AtomicLong();
    private static final Logger SDK_LOGGER = Logger.getLogger("io.opentelemetry");

    private static final class Owner {
        static final Tracer TRACER = create();
    }

    static void initialize() { Owner.TRACER.getClass(); }

    private static Tracer create() {
        String endpoint = System.getenv("INDRA_TIKA_OTLP_ENDPOINT");
        if (endpoint == null) {
            if (System.getenv("INDRA_TIKA_OTLP_AUTHORIZATION") != null) {
                throw new IllegalArgumentException("INDRA_TIKA_OTLP_AUTHORIZATION requires INDRA_TIKA_OTLP_ENDPOINT");
            }
            return TracerProvider.noop().get("indra-tika", VERSION);
        }
        validateEndpoint(endpoint);
        int timeout = integer("INDRA_TIKA_EXPORT_TIMEOUT_MS", 1000, 100, 5000);
        int queue = integer("INDRA_TIKA_EXPORT_QUEUE", 256, 16, 4096);
        int batch = integer("INDRA_TIKA_EXPORT_BATCH", Math.min(64, queue), 1, Math.min(512, queue));
        int interval = integer("INDRA_TIKA_EXPORT_INTERVAL_MS", 1000, 100, 60000);
        double ratio = ratio();
        // SDK diagnostics may contain collector URLs or response bodies. Keep
        // only a bounded count and severity; never forward their text/Throwable.
        SDK_LOGGER.setUseParentHandlers(false);
        SDK_LOGGER.setLevel(Level.WARNING);
        SDK_LOGGER.addHandler(new Handler() {
            @Override public void publish(LogRecord record) {
                if (record.getLevel().intValue() >= Level.WARNING.intValue()) {
                    long count = SDK_WARNINGS.incrementAndGet();
                    if (count == 1 || (count & (count - 1)) == 0) {
                        System.err.println("{\"level\":\"WARN\",\"event\":\"tika_trace_sdk_warning\",\"count\":" + count + "}");
                    }
                }
            }
            @Override public void flush() { }
            @Override public void close() { }
        });
        AtomicInteger thread = new AtomicInteger();
        var executor = new ThreadPoolExecutor(2, 2, 30, TimeUnit.SECONDS,
                new ArrayBlockingQueue<>(32), task -> {
                    Thread worker = new Thread(task, "indra-tika-export-" + thread.incrementAndGet());
                    worker.setDaemon(true);
                    return worker;
                }, new ThreadPoolExecutor.AbortPolicy());
        executor.allowCoreThreadTimeOut(true);
        try {
            var exporterBuilder = OtlpHttpSpanExporter.builder()
                    .setEndpoint(endpoint)
                    .setTimeout(Duration.ofMillis(timeout))
                    .setConnectTimeout(Duration.ofMillis(timeout))
                    .setMaxRequestBodySize(262144)
                    .setRetryPolicy(null)
                    .setMeterProvider(MeterProvider.noop())
                    .setExecutorService(executor);
            String authorization = System.getenv("INDRA_TIKA_OTLP_AUTHORIZATION");
            if (authorization != null) {
                if (authorization.isEmpty() || authorization.length() > 2048
                        || authorization.chars().anyMatch(c -> c < 0x20 || c > 0x7e)) {
                    throw new IllegalArgumentException("invalid trace authorization header");
                }
                exporterBuilder.addHeader("authorization", authorization);
            }
            var exporter = exporterBuilder.build();
            var processor = BatchSpanProcessor.builder(exporter)
                    .setMaxQueueSize(queue).setMaxExportBatchSize(batch)
                    .setScheduleDelay(Duration.ofMillis(interval))
                    .setExporterTimeout(Duration.ofMillis(timeout + 100L))
                    .build();
            var provider = SdkTracerProvider.builder()
                    .setResource(Resource.create(Attributes.builder()
                            .put("service.name", "tika-parser")
                            .put("service.version", VERSION).build()))
                    .setSampler(Sampler.parentBased(Sampler.traceIdRatioBased(ratio)))
                    .setSpanLimits(SpanLimits.builder().setMaxNumberOfAttributes(16)
                            .setMaxNumberOfEvents(2).setMaxNumberOfLinks(0)
                            .setMaxAttributeValueLength(128).build())
                    .addSpanProcessor(processor).build();
            Runtime.getRuntime().addShutdownHook(new Thread(() -> {
                var result = provider.shutdown().join(4, TimeUnit.SECONDS);
                executor.shutdownNow();
                if (!result.isSuccess()) {
                    System.err.println("{\"level\":\"WARN\",\"event\":\"tika_trace_shutdown_incomplete\"}");
                }
            }, "indra-tika-trace-shutdown"));
            return provider.get("indra-tika", VERSION);
        } catch (RuntimeException failure) {
            executor.shutdownNow();
            // Config startup failure is explicit without exposing URI/credentials
            // from third-party exception messages or nested causes.
            throw new IllegalStateException("Indra Tika trace exporter initialization failed");
        }
    }

    private static void validateEndpoint(String raw) {
        try {
            URI uri = new URI(raw);
            String host = uri.getHost();
            boolean loopback = "127.0.0.1".equals(host) || "[::1]".equals(host);
            if (raw.length() > 2048 || host == null || uri.getRawUserInfo() != null
                    || uri.getRawQuery() != null || uri.getRawFragment() != null
                    || !"/v1/traces".equals(uri.getRawPath())
                    || uri.getPort() < 1 || uri.getPort() > 65535
                    || !("https".equals(uri.getScheme()) || ("http".equals(uri.getScheme()) && loopback))) {
                throw new URISyntaxException("redacted", "invalid");
            }
        } catch (URISyntaxException failure) {
            throw new IllegalArgumentException("INDRA_TIKA_OTLP_ENDPOINT requires an explicit port and /v1/traces path; HTTPS or literal loopback HTTP; no userinfo/query/fragment");
        }
    }

    private static int integer(String key, int fallback, int minimum, int maximum) {
        String value = System.getenv(key);
        if (value == null) { return fallback; }
        try {
            int result = Integer.parseInt(value);
            if (result >= minimum && result <= maximum) { return result; }
        } catch (NumberFormatException ignored) {
            // Report the public key/bounds, never the supplied value.
        }
        throw new IllegalArgumentException(key + " must be an integer from " + minimum + " to " + maximum);
    }

    private static double ratio() {
        String value = System.getenv("INDRA_TIKA_TRACE_SAMPLE_RATIO");
        if (value == null) { return 1.0; }
        try {
            double result = Double.parseDouble(value);
            if (Double.isFinite(result) && result >= 0 && result <= 1) { return result; }
        } catch (NumberFormatException ignored) { }
        throw new IllegalArgumentException("INDRA_TIKA_TRACE_SAMPLE_RATIO must be finite and between zero and one");
    }

    static Call parse(RequestTrace parent, Class<?> parser) {
        return start(parent, "tika.parse", SpanKind.INTERNAL, safeClass(parser));
    }

    static Call ocr(RequestTrace parent) {
        return start(parent, "tika.ocr.process", SpanKind.CLIENT,
                "org.apache.tika.parser.ocr.TesseractOCRParser");
    }

    private static Call start(RequestTrace parent, String name, SpanKind kind, String parser) {
        RequestTrace incoming = parent == null ? RequestTrace.EMPTY : parent;
        var span = Owner.TRACER.spanBuilder(name).setParent(incoming.context())
                .setSpanKind(kind).setAttribute("code.namespace", parser).startSpan();
        return new Call(span, new RequestTrace(incoming.context().with(span)), name);
    }

    private static String safeClass(Class<?> type) {
        String name = type.getName();
        boolean packageAllowed = name.length() <= 128 && (name.startsWith("org.apache.tika.")
                || name.startsWith("java.io.") || name.startsWith("org.xml.sax.")
                || name.startsWith("java.lang."));
        if (!packageAllowed) { return "other"; }
        for (int i = 0; i < name.length(); ++i) {
            char c = name.charAt(i);
            if (!((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z')
                    || (c >= '0' && c <= '9') || c == '.' || c == '$' || c == '_')) {
                return "other";
            }
        }
        return name;
    }

    static final class Call implements AutoCloseable {
        private final Span span;
        private final RequestTrace context;
        private final String operation;
        private boolean closed;
        Call(Span span, RequestTrace context, String operation) {
            this.span = span;
            this.context = context;
            this.operation = operation;
        }
        RequestTrace context() { return context; }
        void failed(Throwable error) {
            String type = safeClass(error.getClass());
            span.setStatus(StatusCode.ERROR);
            span.setAttribute("error.type", type);
            span.addEvent("exception", Attributes.of(EXCEPTION_TYPE, type));
            var ids = span.getSpanContext();
            System.err.println("{\"level\":\"ERROR\",\"event\":\"tika_operation_failed\",\"operation\":\""
                    + operation + "\",\"error.type\":\"" + type + "\",\"trace_id\":\""
                    + ids.getTraceId() + "\",\"span_id\":\"" + ids.getSpanId() + "\"}");
        }
        @Override public void close() {
            if (!closed) { closed = true; span.end(); }
        }
    }
}
