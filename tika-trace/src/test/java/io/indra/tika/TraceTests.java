/* SPDX-License-Identifier: Apache-2.0 */
package io.indra.tika;

import jakarta.ws.rs.core.MultivaluedHashMap;
import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.time.Duration;
import java.util.List;
import java.util.Set;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;
import org.apache.tika.exception.TikaException;
import org.apache.tika.metadata.Metadata;
import org.apache.tika.mime.MediaType;
import org.apache.tika.parser.Parser;
import org.apache.tika.parser.ParseContext;
import org.apache.tika.server.standard.config.TesseractServerConfig;
import org.xml.sax.ContentHandler;
import org.xml.sax.helpers.DefaultHandler;

/** Executable tests use the real Tika SPI, parser interfaces and JVM processes. */
public final class TraceTests {
    private static final String FIRST = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
    private static final String SECOND = "00-abcd2f3577b34da6a3ce929d0e0e4736-11f067aa0ba902b7-01";
    private TraceTests() { }

    public static void main(String[] args) throws Exception {
        if (args.length == 3 && args[0].equals("child")) {
            child(Path.of(args[1]), Integer.parseInt(args[2]));
            return;
        }
        check(System.getenv("INDRA_TIKA_OTLP_ENDPOINT") == null, "unit mode has exporter disabled");
        vectors(Path.of(args[0]));
        providerOrder();
        processEnvironment();
        concurrentContexts();
        failureRestoresContext();
        shutdownProcesses();
        System.out.println("Tika trace tests: shared vectors, SPI order, environment, concurrent contexts, failure restoration and real child shutdown passed");
    }

    private static String decoded(String value) {
        return value.equals("-") ? null : value.replace("\\t", "\t").replace("\\r", "\r").replace("\\n", "\n");
    }

    private static void vectors(Path source) throws IOException {
        check(Files.size(source) < 65536, "bounded shared vectors");
        int count = 0;
        for (String line : Files.readAllLines(source, StandardCharsets.UTF_8)) {
            if (line.isEmpty() || line.startsWith("#")) { continue; }
            String[] fields = line.split("\t", -1);
            check(fields.length == 5, "vector shape");
            var actual = TraceCarrier.from(decoded(fields[1]), decoded(fields[2]));
            check(java.util.Objects.equals(actual.parent, decoded(fields[3])), fields[0] + " parent");
            check(java.util.Objects.equals(actual.state, decoded(fields[4])), fields[0] + " state");
            ++count;
        }
        check(count == 23, "all frozen shared vectors");
        var duplicate = new MultivaluedHashMap<String, String>();
        duplicate.put("traceparent", List.of(FIRST, SECOND));
        check(TraceCarrier.headers(duplicate).parent == null, "duplicate parents rejected");
        duplicate.put("traceparent", List.of(FIRST));
        duplicate.put("tracestate", List.of("one=1", "two=2"));
        check("one=1,two=2".equals(TraceCarrier.headers(duplicate).state), "state fields keep wire order");
        duplicate.put("TraceState", List.of("three=3"));
        var ambiguous = TraceCarrier.headers(duplicate);
        check(FIRST.equals(ambiguous.parent) && ambiguous.state == null, "case-variant map entries have no provable wire order");
        check(TraceCarrier.from(FIRST, "a=" + "x".repeat(257)).state == null, "value cap");
        check(TraceCarrier.from(FIRST, "a=x,".repeat(33)).state == null, "member cap");
        check(TraceCarrier.from(FIRST, "x".repeat(513)).state == null, "state byte cap");
    }

    private static void providerOrder() {
        var headers = new MultivaluedHashMap<String, String>();
        headers.putSingle("traceparent", FIRST);
        headers.putSingle("tracestate", "vendor=opaque");
        headers.putSingle("X-Tika-OCRPreserveInterwordSpacing", "false");
        var provider = new TraceParseContextConfig();
        var standard = new TesseractServerConfig();
        for (int order = 0; order < 2; ++order) {
            var context = new ParseContext();
            var metadata = new Metadata();
            if (order == 0) { provider.configure(headers, metadata, context); }
            standard.configure(headers, metadata, context);
            if (order == 1) { provider.configure(headers, metadata, context); }
            check(FIRST.equals(context.get(RequestTrace.class).carrier().parent), "provider order parent");
            check("vendor=opaque".equals(context.get(RequestTrace.class).carrier().state), "provider order state");
        }
    }

    private static void processEnvironment() {
        var context = new ParseContext();
        context.set(RequestTrace.class, RequestTrace.from(TraceCarrier.from(FIRST, "vendor=opaque")));
        var builder = new ProcessBuilder("original-command", "original-argument");
        builder.environment().put("INDRA_TRACEPARENT", SECOND);
        builder.environment().put("INDRA_TRACESTATE", "stale=deployment");
        builder.environment().put("UNRELATED", "preserved");
        var first = OcrCall.start(context, builder);
        try {
            check(FIRST.equals(builder.environment().get("INDRA_TRACEPARENT")), "request parent replaces stale env");
            check("vendor=opaque".equals(builder.environment().get("INDRA_TRACESTATE")), "request state replaces stale env");
            check(builder.command().equals(List.of("original-command", "original-argument")), "OCR argv unchanged");
            check("preserved".equals(builder.environment().get("UNRELATED")), "unrelated env unchanged");
        } finally { first.close(); }
        context.set(RequestTrace.class, RequestTrace.EMPTY);
        var empty = OcrCall.start(context, builder);
        try {
            check(!builder.environment().containsKey("INDRA_TRACEPARENT"), "missing parent removes stale env");
            check(!builder.environment().containsKey("INDRA_TRACESTATE"), "missing parent removes stale state");
        } finally { empty.close(); }
    }

    private static void concurrentContexts() throws Exception {
        var entered = new CountDownLatch(2);
        var failure = new AtomicReference<Throwable>();
        var parser = new TracedParser(new ProbeParser(entered));
        Thread[] threads = new Thread[2];
        String[] parents = {FIRST, SECOND};
        for (int i = 0; i < threads.length; ++i) {
            String parent = parents[i];
            threads[i] = new Thread(() -> {
                try {
                    var context = new ParseContext();
                    var incoming = RequestTrace.from(TraceCarrier.from(parent, "vendor=opaque"));
                    context.set(RequestTrace.class, incoming);
                    var metadata = new Metadata();
                    metadata.set("expected", parent);
                    parser.parse(InputStream.nullInputStream(), new DefaultHandler(), metadata, context);
                    check(context.get(RequestTrace.class) == incoming, "decorator restores exact previous context");
                } catch (Throwable error) { failure.compareAndSet(null, error); }
            });
            threads[i].start();
        }
        for (Thread thread : threads) { thread.join(4000); check(!thread.isAlive(), "finite concurrent test"); }
        if (failure.get() != null) { throw new AssertionError("concurrent requests", failure.get()); }
    }

    private static final class ProbeParser implements Parser {
        private static final long serialVersionUID = 1L;
        private final transient CountDownLatch entered;
        ProbeParser(CountDownLatch entered) { this.entered = entered; }
        @Override public Set<MediaType> getSupportedTypes(ParseContext context) { return Set.of(MediaType.OCTET_STREAM); }
        @Override public void parse(InputStream stream, ContentHandler handler, Metadata metadata,
                                    ParseContext context) throws IOException, TikaException {
            entered.countDown();
            try {
                check(entered.await(2, TimeUnit.SECONDS), "requests overlap");
            } catch (InterruptedException failure) {
                Thread.currentThread().interrupt();
                throw new IOException("interrupted test");
            }
            var builder = new ProcessBuilder("unused");
            var call = OcrCall.start(context, builder);
            try {
                check(metadata.get("expected").equals(builder.environment().get("INDRA_TRACEPARENT")), "no request trace cross-talk");
            } finally { call.close(); }
        }
    }

    private static void failureRestoresContext() throws Exception {
        var expected = RequestTrace.from(TraceCarrier.from(FIRST, null));
        var context = new ParseContext();
        context.set(RequestTrace.class, expected);
        var parser = new TracedParser(new BrokenParser());
        try {
            parser.parse(InputStream.nullInputStream(), new DefaultHandler(), new Metadata(), context);
            throw new AssertionError("parser failure must propagate");
        } catch (IOException failure) {
            check("DOCUMENT_SECRET".equals(failure.getMessage()), "original exception behavior");
            check(context.get(RequestTrace.class) == expected, "exception restores previous trace");
        }
    }

    private static final class BrokenParser implements Parser {
        private static final long serialVersionUID = 1L;
        @Override public Set<MediaType> getSupportedTypes(ParseContext context) { return Set.of(MediaType.OCTET_STREAM); }
        @Override public void parse(InputStream stream, ContentHandler handler, Metadata metadata,
                                   ParseContext context) throws IOException { throw new IOException("DOCUMENT_SECRET"); }
    }

    private static void child(Path marker, int delay) throws IOException, InterruptedException {
        Runtime.getRuntime().addShutdownHook(new Thread(() -> {
            try {
                Thread.sleep(delay);
                Files.writeString(marker.resolve("flushed"), "complete", StandardOpenOption.CREATE_NEW);
            } catch (IOException | InterruptedException failure) { throw new AssertionError(failure); }
        }));
        Files.writeString(marker.resolve("ready"), "ready", StandardOpenOption.CREATE_NEW);
        new CountDownLatch(1).await();
    }

    private static void shutdownProcesses() throws Exception {
        Path work = Files.createTempDirectory("indra-trace-shutdown-");
        Process[] children = new Process[2];
        try {
            for (int i = 0; i < 2; ++i) {
                Path marker = Files.createDirectory(work.resolve(Integer.toString(i)));
                children[i] = new ProcessBuilder(Path.of(System.getProperty("java.home"), "bin/java").toString(),
                        "-Xmx32m", "-XX:ActiveProcessorCount=1", "-cp", System.getProperty("java.class.path"),
                        TraceTests.class.getName(), "child", marker.toString(), i == 0 ? "50" : "5000")
                        .inheritIO().start();
            }
            long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(3);
            while (!(Files.exists(work.resolve("0/ready")) && Files.exists(work.resolve("1/ready")))) {
                check(System.nanoTime() < deadline, "child readiness deadline");
                Thread.sleep(2);
            }
            long start = System.nanoTime();
            ChildShutdown.finish(Set.of(children), Duration.ofMillis(300), Duration.ofSeconds(1));
            check(System.nanoTime() - start < TimeUnit.SECONDS.toNanos(2), "one aggregate shutdown budget");
            check(!children[0].isAlive() && !children[1].isAlive(), "all child processes reaped");
            check(Files.exists(work.resolve("0/flushed")), "cooperative child can flush");
            check(!Files.exists(work.resolve("1/flushed")), "stalled child is forced after grace");
        } finally {
            for (Process process : children) {
                if (process != null) { process.destroyForcibly(); check(process.waitFor(2, TimeUnit.SECONDS), "cleanup reap"); }
            }
            for (int i = 0; i < 2; ++i) {
                Files.deleteIfExists(work.resolve(i + "/ready"));
                Files.deleteIfExists(work.resolve(i + "/flushed"));
                Files.deleteIfExists(work.resolve(Integer.toString(i)));
            }
            Files.delete(work);
        }
    }

    private static void check(boolean ok, String label) {
        if (!ok) { throw new AssertionError(label); }
    }
}
