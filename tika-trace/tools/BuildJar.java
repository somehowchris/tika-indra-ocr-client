/* SPDX-License-Identifier: Apache-2.0 */
import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.LinkOption;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.security.MessageDigest;
import java.time.Instant;
import java.util.HexFormat;
import java.util.jar.Attributes;
import java.util.jar.JarEntry;
import java.util.jar.JarFile;
import java.util.jar.JarOutputStream;

/** Build-only overlay of two exact Tika class families, retaining every other resource. */
public final class BuildJar {
    private static final String BASE_SHA512 = "fb1f2fe57ac458b09d44d41d816f582e1d2fc93488acff6275caf414d8d5ef94e42166edc0b488dc2fb6ef3aa21fab62b107c43b9060385ff6d675e393c2c9e9";
    private static final String[] PATCHED = {
        "org/apache/tika/parser/ocr/TesseractOCRParser.class",
        "org/apache/tika/parser/ocr/TesseractOCRParser$HOCRPassThroughHandler.class",
        "org/apache/tika/server/core/TikaServerWatchDog.class",
        "org/apache/tika/server/core/TikaServerWatchDog$DoNotRestartException.class",
        "org/apache/tika/server/core/TikaServerWatchDog$FORKED_STATUS.class",
        "org/apache/tika/server/core/TikaServerWatchDog$ForkedProcess.class",
        "org/apache/tika/server/core/TikaServerWatchDog$ForkedStatus.class"
    };
    private BuildJar() { }

    public static void main(String[] args) throws Exception {
        if (args.length != 6) { throw new IllegalArgumentException("BASE_JAR PATCH_CLASSES LIB_DIRECTORY NEW_JAR SOURCE_DATE_EPOCH PATCHSET_SHA256"); }
        Path base = Path.of(args[0]);
        Path patches = Path.of(args[1]);
        Path libraries = Path.of(args[2]);
        Path destination = Path.of(args[3]);
        long timestamp = Instant.ofEpochSecond(Long.parseLong(args[4])).toEpochMilli();
        if (!args[5].matches("[0-9a-f]{64}")) { throw new IllegalArgumentException("patch-set hash"); }
        var digest = MessageDigest.getInstance("SHA-512");
        byte[] buffer = new byte[65536];
        try (var input = Files.newInputStream(base)) {
            for (int n; (n = input.read(buffer)) >= 0;) { digest.update(buffer, 0, n); }
        }
        if (!BASE_SHA512.equals(HexFormat.of().formatHex(digest.digest()))) { throw new IllegalArgumentException("original Apache jar SHA-512 differs"); }
        try (var files = Files.walk(patches)) {
            if (files.filter(Files::isRegularFile).count() != PATCHED.length) { throw new IllegalArgumentException("unexpected compiled class family"); }
        }
        StringBuilder classpath = new StringBuilder(2048);
        try (var files = Files.list(libraries)) {
            Path[] jars = files.sorted().limit(33).toArray(Path[]::new);
            if (jars.length != 16) { throw new IllegalArgumentException("expected15pinnedSDKdependencies plus extension"); }
            for (Path jar : jars) {
                String name = jar.getFileName().toString();
                if (!Files.isRegularFile(jar, LinkOption.NOFOLLOW_LINKS) || !name.matches("[a-zA-Z0-9.-]+\\.jar")) {
                    throw new IllegalArgumentException("unexpected library path");
                }
                if (!classpath.isEmpty()) { classpath.append(' '); }
                classpath.append("trace/").append(name);
            }
        }
        try (var original = new JarFile(base.toFile())) {
            for (String name : PATCHED) {
                if (original.getJarEntry(name) == null || !Files.isRegularFile(patches.resolve(name), LinkOption.NOFOLLOW_LINKS)) {
                    throw new IllegalArgumentException("original/patched class family differs");
                }
            }
            var manifest = original.getManifest();
            var attributes = manifest.getMainAttributes();
            attributes.put(Attributes.Name.IMPLEMENTATION_VERSION, "3.3.2-indra-trace.1");
            attributes.put(Attributes.Name.CLASS_PATH, classpath.toString());
            attributes.putValue("Indra-Base-Version", "3.3.2");
            attributes.putValue("Indra-Base-Commit", "b8a6916eab70ccdb5d4551c69be1a46af29c2cff");
            attributes.putValue("Indra-Base-SHA512", BASE_SHA512);
            attributes.putValue("Indra-Patchset-SHA256", args[5]);
            try (var output = new JarOutputStream(Files.newOutputStream(destination, StandardOpenOption.CREATE_NEW))) {
                var manifestEntry = new JarEntry(JarFile.MANIFEST_NAME);
                manifestEntry.setTime(timestamp);
                output.putNextEntry(manifestEntry);
                manifest.write(output);
                output.closeEntry();
                var entries = original.entries();
                while (entries.hasMoreElements()) {
                    var entry = entries.nextElement();
                    String name = entry.getName();
                    if (name.equals(JarFile.MANIFEST_NAME)) { continue; }
                    if (name.matches("META-INF/[^/]+\\.(SF|RSA|DSA|EC)")) {
                        throw new IllegalArgumentException("cannot retain an invalidated upstream jar signature");
                    }
                    boolean replaced = false;
                    for (String selected : PATCHED) { if (selected.equals(name)) { replaced = true; break; } }
                    var written = new JarEntry(name);
                    written.setTime(timestamp);
                    output.putNextEntry(written);
                    if (!entry.isDirectory()) {
                        try (InputStream input = replaced ? Files.newInputStream(patches.resolve(name)) : original.getInputStream(entry)) {
                            for (int n; (n = input.read(buffer)) >= 0;) { output.write(buffer, 0, n); }
                        }
                    }
                    output.closeEntry();
                }
            }
        }
    }
}
