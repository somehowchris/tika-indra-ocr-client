#!/usr/bin/env bash
# Run in the pinned GNU builder after linking. Never execute this in a service.
set -euo pipefail
export LC_ALL=C
script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
fail() { printf '%s\n' "$*" >&2; exit 1; }

# Required notices for the enabled CPU archive/header closure. The native
# recorder also preserves a broader acquired-source inventory; this explicit
# list fails the release if a required notice is absent or unexpectedly large.
collect_native_notices() {
    local native=$1 target=$2 component relative source destination bytes digest
    local count=0 total=0
    [[ -d "$native" && ! -e "$target" ]] || fail 'native notice source/destination is invalid'
    mkdir -p "$target"
    printf 'component\tsource_relative_path\tnotice_relative_path\tbytes\tsha256\n' > "$target/inventory.tsv"
    while IFS='|' read -r component relative; do
        source="$native/$relative"
        destination="$component/${relative##*/}"
        [[ -s "$source" && -f "$source" && ! -L "$source" ]] || fail "required native notice is missing: $relative"
        bytes=$(stat --format=%s "$source")
        (( bytes <= 2097152 )) || fail "native notice exceeds 2 MiB: $relative"
        total=$((total + bytes))
        (( total <= 8388608 )) || fail 'required native notices exceed 8 MiB'
        [[ ! -e "$target/$destination" ]] || fail 'duplicate required native notice destination'
        mkdir -p "$target/$component"
        cp -- "$source" "$target/$destination"
        digest=$(sha256sum "$target/$destination" | cut -d' ' -f1)
        printf '%s\t%s\t%s\t%s\t%s\n' "$component" "$relative" "$destination" "$bytes" "$digest" >> "$target/inventory.tsv"
        count=$((count + 1))
    done <<'NATIVE_NOTICES'
onnxruntime|src/onnxruntime/LICENSE
onnxruntime|src/onnxruntime/ThirdPartyNotices.txt
opencv|src/opencv/LICENSE
abseil|build/onnxruntime/Release/_deps/abseil_cpp-src/LICENSE
date|build/onnxruntime/Release/_deps/date-src/LICENSE.txt
eigen|build/onnxruntime/Release/_deps/eigen3-src/LICENSE
eigen|build/onnxruntime/Release/_deps/eigen3-src/COPYING.APACHE
eigen|build/onnxruntime/Release/_deps/eigen3-src/COPYING.BSD
eigen|build/onnxruntime/Release/_deps/eigen3-src/COPYING.MINPACK
eigen|build/onnxruntime/Release/_deps/eigen3-src/COPYING.MPL2
eigen|build/onnxruntime/Release/_deps/eigen3-src/COPYING.README
flatbuffers|build/onnxruntime/Release/_deps/flatbuffers-src/LICENSE
gsl|build/onnxruntime/Release/_deps/gsl-src/LICENSE
gsl|build/onnxruntime/Release/_deps/gsl-src/ThirdPartyNotices.txt
nlohmann-json|build/onnxruntime/Release/_deps/nlohmann_json-src/LICENSE.MIT
onnx|build/onnxruntime/Release/_deps/onnx-src/LICENSE
protobuf|build/onnxruntime/Release/_deps/protobuf-src/LICENSE
utf8-range|build/onnxruntime/Release/_deps/protobuf-src/third_party/utf8_range/LICENSE
cpuinfo|build/onnxruntime/Release/_deps/pytorch_cpuinfo-src/LICENSE
clog|build/onnxruntime/Release/_deps/pytorch_cpuinfo-src/deps/clog/LICENSE
re2|build/onnxruntime/Release/_deps/re2-src/LICENSE
safeint|build/onnxruntime/Release/_deps/safeint-src/LICENSE
libjpeg-turbo|src/opencv/3rdparty/libjpeg-turbo/LICENSE.md
libjpeg-turbo|src/opencv/3rdparty/libjpeg-turbo/README.ijg
libpng|src/opencv/3rdparty/libpng/LICENSE
libtiff|src/opencv/3rdparty/libtiff/LICENSE.md
zlib|src/opencv/3rdparty/zlib/LICENSE
softfloat|src/opencv/modules/core/3rdparty/SoftFloat/COPYING.txt
flann|src/opencv/modules/flann/include/opencv2/flann/defines.h
ipp|build/opencv/3rdparty/ippicv/ippicv_lnx/EULA.txt
ipp|build/opencv/3rdparty/ippicv/ippicv_lnx/third-party-programs.txt
NATIVE_NOTICES
    [[ "$count" = 31 ]] || fail 'required native notice inventory is incomplete'
    cat > "$target/README.txt" <<'NATIVE_NOTICE_SCOPE'
Required notices for the enabled ONNX Runtime/OpenCV CPU components and their
archive/header dependencies. inventory.tsv records exact source paths, byte
lengths and SHA-256 values. The broader acquired-source notices remain under
../../native-evidence/notices; not every acquired source is linked at runtime.

This software is based in part on the work of the Independent JPEG Group.

Intel IPP is supplied as an unmodified binary component under its included EULA
and third-party terms. It is not an open-source component. The relink bundle
preserves its static archives; instructions for modifying glibc do not grant
permission to modify Intel IPP or replace its license.

The full build/archive inventory remains in ../../native-evidence. Individual
executables use a subset of archive members; this notice inventory is not a
runtime function or performance profile.
NATIVE_NOTICE_SCOPE
}

[[ $# = 3 ]] || fail 'usage: collect-runtime-materials.sh native|gateway|bridge OUTPUT BINARY'
kind=$1
output=$2
binary=$3
# The native service and gateway build from the runtime workspace under /app;
# the std-only Tika adapter builds from the root package under /build.
case "$kind" in
    native|gateway) project=/app manifest=runtime/Cargo.toml readme=docs/runtime-relinking.md;;
    bridge) project=/build manifest=Cargo.toml readme=docs/relinking.md;;
    *) fail 'unknown runtime kind';;
esac
[[ "$output" = /* && "$binary" = /* && -f "$binary" && ! -L "$binary" ]] || fail 'absolute output and regular binary required'
[[ ! -e "$output" ]] || fail 'refusing to replace a material bundle'
mkdir -p "$output/glibc-source" "$output/licenses/rust" "$project/.cargo"

# Signed APT metadata supplied these sources. Check the installed version again,
# then every archive size and SHA-256 recorded in its exact source-control file.
version=$(dpkg-query -W -f='${source:Version}' libc6-dev)
match_count=0
for dsc in /glibc-source/*.dsc; do
    [[ -f "$dsc" && ! -L "$dsc" ]] || fail 'glibc source control is missing'
    if grep -qx 'Source: glibc' "$dsc" && grep -Fxq "Version: $version" "$dsc"; then
        match_count=$((match_count + 1))
        cp -- "$dsc" "$output/glibc-source/"
        awk '/^Checksums-Sha256:/ { inside=1; next } inside && /^[^ ]/ { exit } inside { print }' "$dsc" > "$output/glibc-archives.txt"
    fi
done
[[ "$match_count" = 1 && -s "$output/glibc-archives.txt" ]] || fail 'glibc source version does not match installed libc6-dev'
archive_count=0
while read -r hash bytes name; do
    [[ "$hash" =~ ^[[:xdigit:]]{64}$ && "$bytes" =~ ^[0-9]+$ && "$name" =~ ^[A-Za-z0-9][A-Za-z0-9._+~:-]*$ ]] || fail 'invalid source archive record'
    source="/glibc-source/$name"
    [[ -f "$source" && ! -L "$source" ]] || fail 'source archive is not a regular file'
    [[ $(stat --format=%s "$source") = "$bytes" ]] || fail 'source archive length differs'
    printf '%s  %s\n' "$hash" "$source" | sha256sum --check --status
    cp -- "$source" "$output/glibc-source/"
    archive_count=$((archive_count + 1))
done < "$output/glibc-archives.txt"
[[ "$archive_count" -ge 2 ]] || fail 'upstream and Debian glibc sources are both required'

for notice in /usr/share/doc/libc6/copyright /usr/share/doc/libgcc-s1/copyright /usr/share/doc/libstdc++6/copyright; do
    [[ -f "$notice" ]] || fail "missing system notice: $notice"
    package=$(basename "$(dirname "$notice")")
    cp -L -- "$notice" "$output/licenses/$package-copyright"
done
for name in LGPL-2.1 GPL-2 GPL-3; do
    cp -- "/usr/share/common-licenses/$name" "$output/licenses/$name"
done
rust_docs="$(rustc --print sysroot)/share/doc/rust"
find "$rust_docs" -maxdepth 1 -type f \( -iname 'license*' -o -iname 'copyright*' -o -iname 'notice*' \) -exec cp -- {} "$output/licenses/rust/" \;
[[ -d "$rust_docs/licenses" ]] || fail 'Rust third-party notices are missing'
cp -a -- "$rust_docs/licenses" "$output/licenses/rust/"

# Cargo's vendored sources include both code and notices and work without registry
# access. Keep the configuration relative to the extracted source root.
cd "$project"
cargo vendor --locked --manifest-path "$manifest" vendor > .cargo/config.toml
if [[ "$kind" = bridge ]]; then
    sources=(Cargo.toml Cargo.lock rust-toolchain.toml src tests benches shared vendor .cargo scripts docker LICENSE THIRD_PARTY_NOTICES.md licenses docs)
else
    sources=(Cargo.toml Cargo.lock src tests benches runtime shared vendor .cargo backend/models.lock.json backend/tests/data scripts docker LICENSE THIRD_PARTY_NOTICES.md licenses docs)
fi
tar --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner \
    --exclude='*/target' --exclude='runtime/proofs' \
    -I 'gzip -n -6' -cf "$output/rust-source.tar.gz" "${sources[@]}"
cp -- "$readme" "$output/README.md"
[[ "$kind" = bridge ]] || cp -- docs/relinking.md "$output/relinking.md"
cp -- LICENSE THIRD_PARTY_NOTICES.md "$output/"
cp -- licenses/INDRA-MODEL-LICENSE.txt "$output/licenses/"
cp -- licenses/INDRA-CODE-LICENSE.txt "$output/licenses/"

if [[ "$kind" = native ]]; then
    cp -a -- /native/evidence "$output/native-evidence"
    collect_native_notices /native "$output/licenses/native"
    # Preserve archive paths expected by ort-sys, plus installed OpenCV headers.
    # They allow a recipient to rebuild Rust and relink against modified glibc
    # without first recompiling ONNX Runtime or OpenCV.
    cd /
    find native/build/onnxruntime/Release native/opencv/lib -type f -name '*.a' -print0 > /tmp/native-material-paths
    printf '%s\0' native/opencv/include native/cargo-env.sh >> /tmp/native-material-paths
    tar --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner \
        --null -I 'gzip -n -6' -cf "$output/native-link-inputs.tar.gz" -T /tmp/native-material-paths
fi

# Name the statically linked C/C++ components for the signed image SBOM.
if [[ "$kind" = native ]]; then
    bash "$script_dir/runtime-static-components.sh" native "$binary" "$output/static-components.spdx.json" /native/evidence
else
    bash "$script_dir/runtime-static-components.sh" "$kind" "$binary" "$output/static-components.spdx.json"
fi

{
    printf 'runtime_kind=%s\nglibc_source_version=%s\n' "$kind" "$version"
    printf 'scope=source-and-relink-inputs; not a claim of a modified-glibc rebuild\n'
    printf 'binary_sha256=%s\n' "$(sha256sum "$binary" | cut -d' ' -f1)"
    rustc -Vv
    gcc --version
    getconf GNU_LIBC_VERSION
} > "$output/build-identity.txt"
cd "$output"
# Its own output is explicitly excluded; nested component manifests are included.
# shellcheck disable=SC2094
find . -type f ! -path ./SHA256SUMS -print0 | sort -z | xargs -0 -r sha256sum > SHA256SUMS
sha256sum --check --status SHA256SUMS
