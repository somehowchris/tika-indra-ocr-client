#!/usr/bin/env bash
# Component SBOM writer regressions. Fake cc and dpkg-query stand in for the
# pinned builder; the native evidence is a fixture. Only the image builds show
# what the real builder resolves.
set -euo pipefail
unset SOURCE_DATE_EPOCH
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT
mkdir -p "$temporary/bin" "$temporary/gcc/lib/x86_64-linux-gnu" "$temporary/gcc/lib/gcc/14" "$temporary/evidence"
export STATIC_TEST_ROOT
STATIC_TEST_ROOT=$(realpath -- "$temporary/gcc")
touch "$temporary/gcc/lib/x86_64-linux-gnu/libc.a" "$temporary/gcc/lib/gcc/14/libgcc_eh.a" "$temporary/gcc/lib/gcc/14/libstdc++.a"
cat > "$temporary/bin/cc" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
case $1 in
  -print-file-name=libc.a) printf '%s\n' "$STATIC_TEST_ROOT/lib/gcc/14/../../x86_64-linux-gnu/libc.a";;
  -print-file-name=libgcc_eh.a|-print-file-name=libstdc++.a) printf '%s\n' "$STATIC_TEST_ROOT/lib/gcc/14/${1#*=}";;
  # gcc echoes the bare name of an archive it cannot find.
  *) printf '%s\n' "${1#*=}";;
esac
SH
cat > "$temporary/bin/dpkg-query" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
if [[ $1 = -S ]]; then
  # Like dpkg, match canonical paths only, not the driver's form with "..".
  case $2 in
    "$STATIC_TEST_ROOT/lib/x86_64-linux-gnu/libc.a") printf 'diversion by libc6 from: %s\n' "$2"; printf 'libc6-dev:amd64: %s\n' "$2";;
    "$STATIC_TEST_ROOT/lib/gcc/14/libgcc_eh.a") printf 'libgcc-14-dev:amd64, libgcc-14-dev:i386: %s\n' "$2";;
    "$STATIC_TEST_ROOT/lib/gcc/14/libstdc++.a") printf 'libstdc++-14-dev:amd64: %s\n' "$2";;
    *) exit 1;;
  esac
  exit 0
fi
[[ $1 = -W && $2 = -f=* ]]
case $3 in
  libc6-dev:amd64) printf 'libc6-dev\t2.41-12+deb13u4\tglibc\t2.41-12+deb13u4\tamd64\n';;
  # A binNMU: the binary version differs from its source version.
  libgcc-14-dev:amd64) printf 'libgcc-14-dev\t14.2.0-19+b1\tgcc-14\t14.2.0-19\tamd64\n';;
  libstdc++-14-dev:amd64) printf 'libstdc++-14-dev\t14.2.0-19\tgcc-14\t14.2.0-19\tamd64\n';;
  *) exit 1;;
esac
SH
chmod +x "$temporary/bin/"*
export PATH="$temporary/bin:$PATH"
cp "$root/scripts/native-build-tools.lock" "$temporary/evidence/native-build-tools.lock"
printf 'component\tversion\tversion_header\n%s\n%s\n%s\n%s\n' \
  $'libjpeg-turbo\t3.1.2\tbuild/opencv/3rdparty/libjpeg-turbo/jconfig.h' \
  $'libpng\t1.6.57\tsrc/opencv/3rdparty/libpng/png.h' \
  $'libtiff\t4.7.1\tbuild/opencv/3rdparty/libtiff/tiffvers.h' \
  $'zlib\t1.3.2\tsrc/opencv/3rdparty/zlib/zlib.h' > "$temporary/evidence/vendored-components.tsv"
printf 'static executable fixture\n' > "$temporary/binary"
binary_sha256=$(sha256sum "$temporary/binary" | cut -d' ' -f1)
# shellcheck source=../scripts/native-build-tools.lock
source "$root/scripts/native-build-tools.lock"
distro=$(awk -F= '$1 == "ID" { gsub(/"/, "", $2); print $2 }' /etc/os-release)
distro_version=$(awk -F= '$1 == "VERSION_ID" { gsub(/"/, "", $2); print $2 }' /etc/os-release)
qualifiers="arch=amd64&distro=$distro-$distro_version"

run() {
  local status=0
  bash "$root/scripts/runtime-static-components.sh" "$@" > "$temporary/stdout" 2> "$temporary/stderr" || status=$?
  return "$status"
}
purls() {
  jq -r '.packages[].externalRefs[] | select(.referenceType == "purl") | .referenceLocator' "$1"
}

run gateway "$temporary/binary" "$temporary/gateway.json"
jq -e --arg sha "$binary_sha256" '
  .spdxVersion == "SPDX-2.3" and .dataLicense == "CC0-1.0" and .SPDXID == "SPDXRef-DOCUMENT" and
  .name == "static-components-gateway" and (.documentNamespace | endswith("/gateway/" + $sha)) and
  .creationInfo.created == "1970-01-01T00:00:00Z" and
  ([.packages[].SPDXID] == ["SPDXRef-Static-gateway-glibc", "SPDXRef-Static-gateway-libgcc"]) and
  ([.relationships[] | select(.spdxElementId == "SPDXRef-DOCUMENT" and .relationshipType == "DESCRIBES")
    | .relatedSpdxElement] == [.packages[].SPDXID]) and
  all(.packages[]; .filesAnalyzed == false and .downloadLocation == "NOASSERTION")
' "$temporary/gateway.json" > /dev/null
diff <(purls "$temporary/gateway.json") - <<EOF
pkg:deb/$distro/libc6-dev@2.41-12%2Bdeb13u4?$qualifiers&upstream=glibc
pkg:deb/$distro/libgcc-14-dev@14.2.0-19%2Bb1?$qualifiers&upstream=gcc-14%4014.2.0-19
EOF
run bridge "$temporary/binary" "$temporary/bridge.json"
jq -e '.name == "static-components-bridge" and
  [.packages[].SPDXID] == ["SPDXRef-Static-bridge-glibc", "SPDXRef-Static-bridge-libgcc"]' "$temporary/bridge.json" > /dev/null

SOURCE_DATE_EPOCH=1792000000 run native "$temporary/binary" "$temporary/native.json" "$temporary/evidence"
diff <(purls "$temporary/native.json") - <<EOF
pkg:deb/$distro/libc6-dev@2.41-12%2Bdeb13u4?$qualifiers&upstream=glibc
pkg:deb/$distro/libgcc-14-dev@14.2.0-19%2Bb1?$qualifiers&upstream=gcc-14%4014.2.0-19
pkg:deb/$distro/libstdc%2B%2B-14-dev@14.2.0-19?$qualifiers&upstream=gcc-14
pkg:github/microsoft/onnxruntime@$ONNXRUNTIME_REVISION
pkg:github/opencv/opencv@$OPENCV_REVISION
pkg:generic/ippicv@2026.0.0?checksum=sha256:$IPP_SHA256&download_url=https://raw.githubusercontent.com/opencv/opencv_3rdparty/$IPP_REVISION/ippicv/$IPP_FILENAME
pkg:generic/libjpeg-turbo@3.1.2
pkg:generic/libpng@1.6.57
pkg:generic/libtiff@4.7.1
pkg:generic/zlib@1.3.2
EOF
jq -e --arg opencv "git+$OPENCV_REPOSITORY@$OPENCV_REVISION" --arg ipp "$IPP_SHA256" '
  .creationInfo.created == "2026-10-14T17:46:40Z" and (.relationships | length) == 10 and
  all(.packages[]; .SPDXID | startswith("SPDXRef-Static-native-")) and
  ([.packages[] | select(.name == "onnxruntime" or .name == "opencv") | .versionInfo | test("^[0-9a-f]{40}$")] == [true, true]) and
  (.packages[] | select(.name == "opencv") | .downloadLocation) == $opencv and
  (.packages[] | select(.name == "ippicv") | .checksums) == [{algorithm: "SHA256", checksumValue: $ipp}] and
  ([.packages[] | select(.name | IN("libjpeg-turbo", "libpng", "libtiff", "zlib")) |
    [.downloadLocation, (.externalRefs[] | select(.referenceType == "cpe23Type") | .referenceLocator)]] == [
    [$opencv + "#3rdparty/libjpeg-turbo", "cpe:2.3:a:libjpeg-turbo:libjpeg-turbo:3.1.2:*:*:*:*:*:*:*"],
    [$opencv + "#3rdparty/libpng", "cpe:2.3:a:libpng:libpng:1.6.57:*:*:*:*:*:*:*"],
    [$opencv + "#3rdparty/libtiff", "cpe:2.3:a:libtiff:libtiff:4.7.1:*:*:*:*:*:*:*"],
    [$opencv + "#3rdparty/zlib", "cpe:2.3:a:zlib:zlib:1.3.2:*:*:*:*:*:*:*"]])
' "$temporary/native.json" > /dev/null

# Rejections name their cause and write no document.
reject() {
  local message=$1
  shift
  if run "$@"; then printf 'accepted: %s\n' "$*" >&2; exit 1; fi
  grep -Fq -- "$message" "$temporary/stderr" || { cat "$temporary/stderr" >&2; exit 1; }
}
cp "$temporary/native.json" "$temporary/native.before"
reject 'refusing to replace' native "$temporary/binary" "$temporary/native.json" "$temporary/evidence"
cmp "$temporary/native.json" "$temporary/native.before"
reject 'links no native SDK' gateway "$temporary/binary" "$temporary/extra.json" "$temporary/evidence"
reject 'requires its native evidence' native "$temporary/binary" "$temporary/missing.json"
reject 'unknown runtime kind' tika "$temporary/binary" "$temporary/tika.json"
cp -a "$temporary/evidence" "$temporary/good-evidence"
mutate() {
  rm -rf "$temporary/evidence"
  cp -a "$temporary/good-evidence" "$temporary/evidence"
  "$@"
}
mutate sed -i '$d' "$temporary/evidence/vendored-components.tsv"
reject 'inventory is incomplete' native "$temporary/binary" "$temporary/short.json" "$temporary/evidence"
mutate sed -i '2d' "$temporary/evidence/vendored-components.tsv"
reject 'unexpected vendored component record: libpng' native "$temporary/binary" "$temporary/order.json" "$temporary/evidence"
mutate sed -i 's/^libtiff\t4.7.1/libtiff\t"4.7.1"/' "$temporary/evidence/vendored-components.tsv"
reject 'unexpected vendored component record: libtiff' native "$temporary/binary" "$temporary/quoted.json" "$temporary/evidence"
mutate sed -i '1s/version_header/header/' "$temporary/evidence/vendored-components.tsv"
reject 'unknown format' native "$temporary/binary" "$temporary/format.json" "$temporary/evidence"
mutate rm "$temporary/evidence/vendored-components.tsv"
reject 'lacks vendored-components.tsv' native "$temporary/binary" "$temporary/absent.json" "$temporary/evidence"
mutate sed -i 's/^OPENCV_REVISION=.*/OPENCV_REVISION=main/' "$temporary/evidence/native-build-tools.lock"
reject 'invalid locked opencv source' native "$temporary/binary" "$temporary/floating.json" "$temporary/evidence"
mutate true
rm "$temporary/gcc/lib/gcc/14/libstdc++.a"
reject 'finds no libstdc++.a' native "$temporary/binary" "$temporary/nostdcxx.json" "$temporary/evidence"
[[ $(cd "$temporary" && printf '%s ' ./*.json) = './bridge.json ./gateway.json ./native.json ' ]]
printf 'Static component SBOM writer: gateway, bridge and native documents, Debian purl encoding, dpkg diversion records, locked revisions, IPP checksum, codec CPEs and 11 rejections passed. Fake builder tools only.\n'
