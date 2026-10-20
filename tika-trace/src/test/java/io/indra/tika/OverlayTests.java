/* SPDX-License-Identifier: Apache-2.0 */
package io.indra.tika;

import java.io.ByteArrayInputStream;
import java.net.HttpURLConnection;
import java.net.InetSocketAddress;
import java.net.ServerSocket;
import java.net.URI;
import java.nio.channels.FileChannel;
import java.nio.channels.FileLock;
import java.nio.channels.OverlappingFileLockException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.util.List;
import java.util.ServiceLoader;
import java.util.Set;
import java.util.concurrent.Callable;
import java.util.concurrent.Executors;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;
import java.util.jar.JarFile;
import com.sun.net.httpserver.HttpServer;
import org.apache.tika.Tika;
import org.apache.tika.config.TikaConfig;
import org.apache.tika.metadata.Metadata;
import org.apache.tika.parser.ParseContext;
import org.apache.tika.server.core.ParseContextConfig;
import org.apache.tika.server.core.TikaServerConfig;
import org.apache.tika.server.core.TikaServerWatchDog;
import org.apache.tika.sax.BodyContentHandler;

/** Runs against the finished jar via its manifest classpath, not the build classpath. */
public final class OverlayTests {
    private OverlayTests() { }

    public static void main(String[] args) throws Exception {
        if (args[0].equals("watchdog")) {
            watchdog(Path.of(args[1]), Boolean.parseBoolean(args[2]));
            return;
        }
        if (args[0].equals("recovery")) { watchdogShutdown(false); return; }
        check("Apache Tika 3.3.2".equals(new Tika().toString()), "preserve exact gateway version contract");
        try (var jar = new JarFile(Path.of(args[0]).toFile())) {
            var attributes = jar.getManifest().getMainAttributes();
            check("3.3.2-indra-trace.1".equals(attributes.getValue("Indra-Overlay-Version")), "identify modified distribution");
            check(attributes.getValue("Indra-Patchset-SHA256").matches("[0-9a-f]{64}"), "bind overlay source hash");
        }
        boolean found = false;
        for (var provider : ServiceLoader.load(ParseContextConfig.class)) {
            found |= provider instanceof TraceParseContextConfig;
        }
        check(found, "discover actual request-context SPI through manifest classpath");
        hook("org/apache/tika/parser/ocr/TesseractOCRParser.class", "io/indra/tika/OcrCall");
        hook("org/apache/tika/server/core/TikaServerWatchDog.class", "io/indra/tika/ChildShutdown");
        var xml = """
                <properties><parsers><parser class="io.indra.tika.TracedParser">
                <parser class="org.apache.tika.parser.txt.TXTParser"/>
                </parser></parsers></properties>
                """;
        var config = new TikaConfig(new ByteArrayInputStream(xml.getBytes(StandardCharsets.UTF_8)));
        var output = new BodyContentHandler();
        var metadata = new Metadata();
        metadata.set("Content-Type", "text/plain");
        config.getParser().parse(new ByteArrayInputStream("trace overlay".getBytes(StandardCharsets.UTF_8)),
                output, metadata, new ParseContext());
        check(output.toString().contains("trace overlay"), "actual Tika reflection constructs and executes decorator");
        watchdogShutdown(true);
        watchdogShutdown(false);
        System.out.println("Tika overlay tests: base version, overlay identity, SPI, patched hooks, parser execution, terminal OTLP flush and forced recovery passed");
    }

    /** Hold the real SDK's OTLP acknowledgement until the competing finally path waits. */
    private static void watchdogShutdown(boolean terminal) throws Exception {
        Path work = Files.createTempDirectory("indra-watchdog-");
        var failure = new AtomicReference<Throwable>();
        var exporterBody = new AtomicReference<byte[]>();
        var owner = new AtomicReference<Process>();
        var executor = Executors.newSingleThreadExecutor();
        var collector = HttpServer.create(new InetSocketAddress("127.0.0.1", 0), 1);
        collector.setExecutor(executor);
        collector.createContext("/v1/traces", exchange -> {
            try {
                check(terminal, "normal watchdog recovery must force the child before SDK shutdown");
                byte[] body = exchange.getRequestBody().readNBytes(262145);
                check(body.length > 0 && body.length <= 262144, "bounded complete OTLP request");
                exporterBody.set(body);
                long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(3);
                while (!Files.exists(work.resolve("terminal-grace-entered"))) {
                    check(owner.get().isAlive(), "watchdog forced the child before terminal grace");
                    check(System.nanoTime() < deadline, "terminal grace barrier deadline");
                    Thread.sleep(2);
                }
                exchange.sendResponseHeaders(200, -1);
            } catch (Throwable error) {
                failure.compareAndSet(null, error);
            } finally {
                exchange.close();
            }
        });
        collector.start();
        Process process = null;
        try {
            var command = new ProcessBuilder(java(), "-Xmx96m", "-XX:ActiveProcessorCount=1",
                    "-cp", System.getProperty("java.class.path"), OverlayTests.class.getName(),
                    "watchdog", work.toString(), Boolean.toString(terminal));
            command.environment().put("INDRA_TIKA_OTLP_ENDPOINT",
                    "http://127.0.0.1:" + collector.getAddress().getPort() + "/v1/traces");
            command.environment().put("INDRA_TIKA_EXPORT_INTERVAL_MS", "60000");
            command.environment().put("INDRA_TIKA_EXPORT_TIMEOUT_MS", "5000");
            command.environment().put("INDRA_TIKA_EXPORT_BATCH", "512");
            command.environment().put("INDRA_TIKA_EXPORT_QUEUE", "1024");
            command.environment().put("INDRA_TIKA_TRACE_SAMPLE_RATIO", "1.0");
            process = command.redirectErrorStream(true).redirectOutput(work.resolve("watchdog.log").toFile()).start();
            owner.set(process);
            check(process.waitFor(12, TimeUnit.SECONDS), "bounded actual watchdog regression");
            String log = Files.readString(work.resolve("watchdog.log"));
            check(process.exitValue() == 0, "actual watchdog regression failed: " + log);
            check(failure.get() == null, "collector failed: " + failure.get());
            if (terminal) {
                byte[] body = exporterBody.get();
                check(body != null, "terminal shutdown must export the queued Java span");
                String wire = new String(body, StandardCharsets.ISO_8859_1);
                check(wire.contains("tika.parse") && wire.contains("tika-parser"), "real Java SDK OTLP span and resource");
                check(!log.contains("tika_trace_shutdown_incomplete") && !log.contains("tika_trace_sdk_warning"),
                        "child SDK shutdown completed after OTLP acknowledgement");
            } else {
                check(exporterBody.get() == null && log.contains("failed to read from status file. Restarting now."),
                        "normal failed-communication recovery still uses immediate force");
            }
            System.out.println("Watchdog regression: " + (terminal ? "terminal SDK flush" : "normal forced recovery") + " passed");
        } finally {
            if (process != null && process.isAlive()) {
                process.descendants().forEach(ProcessHandle::destroyForcibly);
                process.destroyForcibly();
                check(process.waitFor(2, TimeUnit.SECONDS), "watchdog cleanup reap");
            }
            collector.stop(0);
            executor.shutdownNow();
            try (var paths = Files.walk(work)) {
                for (Path path : paths.sorted(java.util.Comparator.reverseOrder()).toList()) { Files.deleteIfExists(path); }
            }
        }
    }

    @SuppressWarnings("unchecked") // Read the actual concurrent ownership set; do not replace Process objects.
    private static void watchdog(Path work, boolean terminal) throws Exception {
        int port;
        try (var socket = new ServerSocket(0, 1, java.net.InetAddress.getLoopbackAddress())) { port = socket.getLocalPort(); }
        Path config = work.resolve("tika.xml");
        Files.writeString(config, """
                <properties><server><taskPulseMillis>1000</taskPulseMillis><taskTimeoutMillis>120000</taskTimeoutMillis>
                <endpoints><endpoint>tika</endpoint><endpoint>version</endpoint></endpoints></server>
                <parsers><parser class="io.indra.tika.TracedParser"><parser class="org.apache.tika.parser.txt.TXTParser"/>
                </parser></parsers></properties>
                """);
        var settings = new TikaServerConfig();
        settings.setHost("127.0.0.1");
        settings.setJavaPath(java());
        settings.setConfigPath(config.toString());
        settings.setForkedJvmArgs(List.of("-Xmx96m", "-XX:ActiveProcessorCount=1", "-Djava.awt.headless=true"));
        settings.setMaxForkedStartupMillis(5000);
        settings.setTaskPulseMillis(10);
        settings.setTaskTimeoutMillis(200);
        var constructor = TikaServerWatchDog.class.getDeclaredConstructor(int.class, String.class, TikaServerConfig.class);
        constructor.setAccessible(true);
        var watcher = (Callable<?>) constructor.newInstance(port, "shutdown-regression", settings);
        var failed = new AtomicReference<Throwable>();
        var thread = new Thread(() -> {
            try { watcher.call(); } catch (Throwable error) { failed.set(error); }
        }, "actual-watchdog-regression");
        thread.start();
        var field = TikaServerWatchDog.class.getDeclaredField("PROCESSES");
        field.setAccessible(true);
        var children = (Set<Process>) field.get(null);
        Process child = null;
        Thread hook = null;
        try {
            long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(5);
            while (children.isEmpty()) { check(System.nanoTime() < deadline, "watchdog child startup"); Thread.sleep(2); }
            child = children.iterator().next();
            while (true) {
                try { request(port, false); break; } catch (java.io.IOException unavailable) {
                    check(child.isAlive() && System.nanoTime() < deadline, "actual Tika readiness"); Thread.sleep(5);
                }
            }
            request(port, true); // A real traced parser call remains queued until shutdown.
            if (terminal) {
                Process owned = child;
                hook = new Thread(() -> ChildShutdown.finish(Set.of(owned)), "concurrent-shutdown-hook");
                hook.start();
                deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(3);
                while (true) {
                    boolean waiting = java.util.Arrays.stream(thread.getStackTrace()).anyMatch(frame ->
                            frame.getClassName().equals(ChildShutdown.class.getName()) && frame.getMethodName().equals("finish"));
                    if (waiting) { break; }
                    check(child.isAlive(), "terminal finally forced the real JVM before SDK flush");
                    check(thread.isAlive() && System.nanoTime() < deadline, "terminal finally must enter bounded grace");
                    Thread.sleep(2);
                }
                Files.writeString(work.resolve("terminal-grace-entered"), "ready");
            } else {
                // Let this recovery finish without starting another parser JVM.
                ((TikaServerWatchDog) watcher).shutDown();
                String[] arguments = child.info().arguments().orElseThrow();
                Path status = null;
                for (int i = 0; i + 1 < arguments.length; ++i) {
                    if (arguments[i].equals("-forkedStatusFile")) { status = Path.of(arguments[i + 1]); }
                }
                check(status != null, "actual forked status file");
                // Keep the real file locked until readStatus returns FAILED_COMMUNICATION.
                try (var channel = FileChannel.open(status, StandardOpenOption.WRITE); var lock = recoveryLock(channel)) {
                    check(lock.isValid(), "hold normal recovery input");
                    thread.join(3000);
                    check(!thread.isAlive(), "normal recovery must force without waiting for shutdown grace");
                }
            }
            thread.join(3000);
            if (hook != null) { hook.join(3000); check(!hook.isAlive(), "concurrent hook completed"); }
            check(!thread.isAlive() && !child.isAlive(), "watchdog and child completed");
            check(terminal ? failed.get() instanceof java.nio.file.NoSuchFileException : failed.get() == null,
                    "exercise the exact terminal status-file race or normal recovery: " + failed.get());
            // EOF calls System.exit(PARENT_EXCEPTION=6), concurrently with SIGTERM's 143.
            int exit = child.exitValue();
            check(terminal ? exit == 6 || exit == 143 : exit == 137,
                    "graceful terminal exit versus normal SIGKILL: " + exit);
            System.out.println("Actual watchdog child exit=" + exit);
        } finally {
            if (child != null && child.isAlive()) { child.destroyForcibly(); check(child.waitFor(2, TimeUnit.SECONDS), "real child cleanup"); }
            thread.join(2000);
            if (hook != null) { hook.join(2000); }
        }
    }

    private static FileLock recoveryLock(FileChannel channel) throws Exception {
        long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(1);
        while (true) {
            check(System.nanoTime() < deadline, "bounded recovery status-file lock");
            try {
                FileLock lock = channel.tryLock();
                if (lock != null) { return lock; }
            } catch (OverlappingFileLockException busy) {
                // The watchdog can briefly hold its shared read lock in this JVM.
            }
            Thread.sleep(1);
        }
    }

    private static void request(int port, boolean parse) throws java.io.IOException {
        var connection = (HttpURLConnection) URI.create("http://127.0.0.1:" + port + (parse ? "/tika" : "/version")).toURL().openConnection();
        connection.setConnectTimeout(100);
        connection.setReadTimeout(2000);
        try {
            if (parse) {
                connection.setRequestMethod("PUT");
                connection.setRequestProperty("Content-Type", "text/plain");
                connection.setDoOutput(true);
                try (var output = connection.getOutputStream()) { output.write("watchdog regression".getBytes(StandardCharsets.UTF_8)); }
            }
            check(connection.getResponseCode() == 200, "actual Tika response");
            try (var input = connection.getInputStream()) { input.readAllBytes(); }
        } finally { connection.disconnect(); }
    }

    private static String java() { return Path.of(System.getProperty("java.home"), "bin/java").toString(); }

    private static void hook(String resource, String expected) throws Exception {
        try (var input = OverlayTests.class.getClassLoader().getResourceAsStream(resource)) {
            check(input != null && new String(input.readAllBytes(), StandardCharsets.ISO_8859_1).contains(expected),
                    "finished jar contains exact patched hook " + resource);
        }
    }

    private static void check(boolean ok, String label) {
        if (!ok) { throw new AssertionError(label); }
    }
}
