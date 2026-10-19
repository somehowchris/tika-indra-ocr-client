/* SPDX-License-Identifier: Apache-2.0 */
package io.indra.tika;

import java.io.ByteArrayInputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Path;
import java.util.ServiceLoader;
import java.util.jar.JarFile;
import org.apache.tika.Tika;
import org.apache.tika.config.TikaConfig;
import org.apache.tika.metadata.Metadata;
import org.apache.tika.parser.ParseContext;
import org.apache.tika.server.core.ParseContextConfig;
import org.apache.tika.sax.BodyContentHandler;

/** Runs against the finished jar via its manifest classpath, not the build classpath. */
public final class OverlayTests {
    private OverlayTests() { }

    public static void main(String[] args) throws Exception {
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
        System.out.println("Tika overlay tests: base version, overlay identity, SPI, patched class hooks, configured parser execution passed");
    }

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
