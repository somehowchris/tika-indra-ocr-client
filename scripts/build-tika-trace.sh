#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Build a source-bound Tika 3.3.2 overlay, its SDK closure, and executable tests.
set -euo pipefail
export LC_ALL=C TZ=UTC
if [ "$#" -ne 2 ]; then
  printf 'usage: %s VERIFIED_BASE_JAR NEW_OUTPUT_DIRECTORY\n' "$0" >&2
  exit 2
fi
base=$(realpath "$1")
output=$2
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
mkdir "$output"
output=$(realpath "$output")
work=$(mktemp -d)
finish() {
  local status=$?
  trap - EXIT
  if [ "$status" -ne 0 ]; then
    for log in "$output"/evidence/*tests.log; do
      if [ -f "$log" ]; then cat "$log" >&2; fi
    done
  fi
  rm -rf "$work"
  exit "$status"
}
trap finish EXIT
mkdir -p "$work/classes" "$work/patches" "$work/test-classes" "$work/tools" "$output/trace" "$output/materials/upstream" "$output/materials/maven" "$output/evidence"
printf '%s  %s\n' fb1f2fe57ac458b09d44d41d816f582e1d2fc93488acff6275caf414d8d5ef94e42166edc0b488dc2fb6ef3aa21fab62b107c43b9060385ff6d675e393c2c9e9 "$base" | sha512sum --check --status
fetch() {
  local url=$1 destination=$2 digest=$3 bytes=$4
  curl --fail --show-error --silent --location --retry 3 --connect-timeout 20 --max-time 180 \
    --proto '=https' --proto-redir '=https' --max-filesize "$bytes" "$url" --output "$destination"
  test "$(stat --format=%s "$destination")" = "$bytes"
  printf '%s  %s\n' "$digest" "$destination" | sha256sum --check --status
}
while IFS=$'\t' read -r name digest bytes source; do
  [[ "$name" = \#* || -z "$name" ]] && continue
  fetch "$source" "$output/materials/upstream/$name" "$digest" "$bytes"
done < "$root/tika-trace/sources.lock.tsv"
while IFS=$'\t' read -r kind digest bytes path; do
  [[ "$kind" = \#* || -z "$kind" ]] && continue
  case "$kind" in runtime) destination="$output/trace/${path##*/}" ;; sources|pom) destination="$output/materials/maven/${path##*/}" ;; *) exit 1 ;; esac
  fetch "https://repo.maven.apache.org/maven2/$path" "$destination" "$digest" "$bytes"
done < "$root/tika-trace/dependencies.lock.tsv"
cp "$output/materials/upstream/"*.java "$work/"
patch --batch --fuzz=0 --directory "$work" --strip=1 < "$root/tika-trace/patches/0001-ocr-process-trace.patch"
patch --batch --fuzz=0 --directory "$work" --strip=1 < "$root/tika-trace/patches/0002-child-telemetry-shutdown.patch"
mapfile -t sources < <(find "$root/tika-trace/src/main/java" -type f -name '*.java' | sort)
javac -proc:none --release 21 -encoding UTF-8 -g:none -Xlint:all -Werror \
  -cp "$base:$output/trace/*" -d "$work/classes" "${sources[@]}"
cp -a "$root/tika-trace/src/main/resources/." "$work/classes/"
jar --create --file "$output/trace/indra-tika-trace.jar" --date=2026-10-17T00:00:00Z -C "$work/classes" .
javac -proc:none --release 21 -encoding UTF-8 -g:none \
  -cp "$base:$output/trace/*" -d "$work/patches" "$work/TesseractOCRParser.java" "$work/TikaServerWatchDog.java"
javac -proc:none --release 21 -encoding UTF-8 -g:none -Xlint:all -Werror \
  -cp "$base:$output/trace/*" -d "$work/test-classes" "$root/tika-trace/src/test/java/io/indra/tika/TraceTests.java"
env -u INDRA_TIKA_OTLP_ENDPOINT java -Xmx96m -XX:ActiveProcessorCount=1 \
  -cp "$work/test-classes:$base:$output/trace/*" io.indra.tika.TraceTests \
  "$root/shared/trace-context/cases.tsv" > "$output/evidence/java-tests.log" 2>&1
javac -proc:none --release 21 -encoding UTF-8 -g:none -Xlint:all -Werror -d "$work/tools" "$root/tika-trace/tools/BuildJar.java"
patchset=$(cd "$root" && find tika-trace -type f -print0 | sort -z | xargs -0 sha256sum | sha256sum | cut -d ' ' -f 1)
java -Xmx256m -XX:ActiveProcessorCount=1 -cp "$work/tools" BuildJar "$base" "$work/patches" "$output/trace" "$output/tika-server.jar" 1792195200 "$patchset"
javac -proc:none --release 21 -encoding UTF-8 -g:none -Xlint:all -Werror \
  -cp "$output/tika-server.jar" -d "$work/test-classes" "$root/tika-trace/src/test/java/io/indra/tika/OverlayTests.java"
# The image selects the JSON console layout the same way; the watchdog test
# proves that its forked Tika JVM inherits the setting.
cp "$root/tika-trace/log4j2.xml" "$output/log4j2.xml"
env -u INDRA_TIKA_OTLP_ENDPOINT LOG4J_CONFIGURATION_FILE="$output/log4j2.xml" java -Xmx128m -XX:ActiveProcessorCount=1 \
  -cp "$work/test-classes:$output/tika-server.jar" io.indra.tika.OverlayTests \
  "$output/tika-server.jar" > "$output/evidence/overlay-tests.log" 2>&1
# Keep the original extension, patches, exact baseline sources, source jars and
# POMs available in the distributed image. No build tool is needed at runtime.
cp -a "$root/tika-trace" "$output/materials/extension"
cp "$root/scripts/build-tika-trace.sh" "$output/materials/"
cp "$work/TesseractOCRParser.java" "$work/TikaServerWatchDog.java" "$output/materials/"
printf 'base_version=3.3.2\noverlay_version=3.3.2-indra-trace.1\nbase_commit=b8a6916eab70ccdb5d4551c69be1a46af29c2cff\npatchset_sha256=%s\nsource_date_epoch=1792195200\n' "$patchset" > "$output/evidence/build.txt"
java -version 2> "$output/evidence/java-version.txt"
javac -version > "$output/evidence/javac-version.txt"
(cd "$output" && find trace materials evidence -type f -print0 | sort -z | xargs -0 sha256sum; sha256sum tika-server.jar log4j2.xml) > "$output/SHA256SUMS"
(cd "$output" && sha256sum --check --status SHA256SUMS)
