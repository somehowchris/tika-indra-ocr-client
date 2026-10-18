#!/usr/bin/env bash
# Record exact native inputs/configuration and, after build, every static archive.
set -euo pipefail
export LC_ALL=C
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
# shellcheck source=native-build-tools.lock
source "$script_dir/native-build-tools.lock"
fail() { printf '%s\n' "$*" >&2; exit 1; }
[[ $# -ge 2 && $# -le 3 ]] || fail "usage: $0 PREFIX EVIDENCE [--configuration-only]"
prefix=$1
evidence=$2
mode=${3:-complete}
[[ "$mode" = complete || "$mode" = --configuration-only ]] || fail 'unknown mode'
[[ "$prefix" = /* && "$evidence" = /* ]] || fail 'absolute paths required'
mkdir -p "$evidence/notices"

verify_source() {
    local name=$1 expected=$2
    [[ $(git -C "$prefix/src/$name" rev-parse HEAD) = "$expected" ]] || fail "$name source revision differs"
    [[ -z $(git -C "$prefix/src/$name" status --porcelain --untracked-files=no) ]] || fail "$name tracked source changed"
    git -C "$prefix/src/$name" submodule status --recursive > "$evidence/$name-submodules.txt"
    if grep -Eq '^[+-U]' "$evidence/$name-submodules.txt"; then
        fail "$name submodule is missing or differs from the recorded commit"
    fi
}
verify_source onnxruntime "$ONNXRUNTIME_REVISION"
verify_source opencv "$OPENCV_REVISION"
cp -- "$script_dir/native-build-tools.lock" "$evidence/native-build-tools.lock"
cp -- "$prefix/src/onnxruntime/cmake/deps.txt" "$evidence/onnxruntime-deps.txt"
cp -- "$prefix/build/onnxruntime/Release/CMakeCache.txt" "$evidence/onnxruntime-CMakeCache.txt"
cp -- "$prefix/build/opencv/CMakeCache.txt" "$evidence/opencv-CMakeCache.txt"

cache_is() {
    local path=$1 key=$2 expected=$3 actual
    actual=$(awk -F= -v key="$key" '$1 ~ ("^" key ":[^=]+$") { print substr($0,index($0,"=")+1) }' "$path")
    if [[ "$key" = CPU_DISPATCH ]]; then actual=${actual//,/;}; fi
    [[ "$actual" = "$expected" ]] || fail "native configuration mismatch: $key expected=$expected actual=$actual"
}
ort_cache="$evidence/onnxruntime-CMakeCache.txt"
cv_cache="$evidence/opencv-CMakeCache.txt"
for flag in BUILD_SHARED_LIB BUILD_UNIT_TESTS ENABLE_PYTHON ENABLE_TRAINING ENABLE_TRAINING_APIS ENABLE_TRAINING_OPS USE_TELEMETRY USE_MIMALLOC BUILD_FOR_NATIVE_MACHINE ENABLE_LTO MINIMAL_BUILD REDUCED_OPS_BUILD USE_CUDA USE_OPENVINO USE_DNNL USE_TENSORRT; do
    cache_is "$ort_cache" "onnxruntime_$flag" OFF
done
cache_is "$cv_cache" BUILD_SHARED_LIBS OFF
for flag in CV_TRACE WITH_ITT WITH_OPENCL WITH_OPENMP WITH_TBB WITH_LAPACK; do
    cache_is "$cv_cache" "$flag" OFF
done
for flag in WITH_IPP WITH_EIGEN WITH_JPEG WITH_PNG WITH_TIFF BUILD_JPEG BUILD_PNG BUILD_TIFF BUILD_ZLIB; do
    cache_is "$cv_cache" "$flag" ON
done
cache_is "$cv_cache" CPU_BASELINE SSE3
cache_is "$cv_cache" CPU_DISPATCH 'SSE4_1;SSE4_2;AVX;FP16;AVX2;AVX512_SKX'
if grep -Eq -- '-m(arch|cpu)=native' "$ort_cache" "$cv_cache"; then
    fail 'host-specific compiler flag appears in native configuration'
fi
[[ -f "$prefix/build/opencv/opencv2/cvconfig.h" ]] || fail 'OpenCV configuration header missing'
cv_header="$prefix/build/opencv/opencv2/cvconfig.h"
for feature in HAVE_IPP HAVE_EIGEN HAVE_JPEG HAVE_PNG HAVE_TIFF; do
    grep -Eq "^#define $feature([[:space:]]|$)" "$cv_header" || fail "OpenCV silently omitted $feature"
done
cp -- "$cv_header" "$evidence/opencv-cvconfig.h"
[[ -f "$prefix/build/opencv/3rdparty/libjpeg-turbo/jconfig.h" ]] || fail 'JPEG configuration header missing'
grep -Eq '^#define WITH_SIMD 1' "$prefix/build/opencv/3rdparty/libjpeg-turbo/jconfig.h" || fail 'JPEG SIMD was not enabled'
printf '%s  %s\n' "$IPP_SHA256" "$prefix/downloads/$IPP_FILENAME" | sha256sum --check --status || fail 'IPP SHA-256 differs'

{
    printf 'onnxruntime_revision=%s\nopencv_revision=%s\nipp_sha256=%s\n' "$ONNXRUNTIME_REVISION" "$OPENCV_REVISION" "$IPP_SHA256"
    printf 'configuration_only=%s\n' "$([[ "$mode" = --configuration-only ]] && printf true || printf false)"
    printf 'execution_environment_attestation=caller-supplied; this recorder does not attest Docker, CFS or OCR accuracy\n'
    printf 'cpu_dispatch_selection_claim=none; portable baseline plus upstream runtime dispatch\n'
    cmake --version
    ninja --version
    nasm -v
    gcc --version
    g++ --version
    getconf GNU_LIBC_VERSION
} > "$evidence/build-provenance.txt"

# Preserve native notices without collapsing third-party notices to top-level licenses.
for source_name in onnxruntime opencv; do
    root="$prefix/src/$source_name"
    for name in LICENSE LICENSE.txt NOTICE ThirdPartyNotices.txt; do
        if [[ -f "$root/$name" ]]; then
            cp -- "$root/$name" "$evidence/notices/$source_name-$name"
        fi
    done
done
find "$prefix/build/onnxruntime/Release/_deps" "$prefix/src/opencv/3rdparty" "$prefix/src/opencv/modules" "$prefix/build/opencv/3rdparty/ippicv" \
    -type f \( -iname 'license*' -o -iname 'copying*' -o -iname 'copyright*' -o -iname '*third*party*' -o -name 'README.ijg' -o -name 'OFL.txt' \) \
    -print0 | sort -z > "$evidence/notice-paths.nul"
while IFS= read -r -d '' notice; do
    relative=${notice#"$prefix/"}
    mkdir -p "$evidence/notices/$(dirname -- "$relative")"
    cp -- "$notice" "$evidence/notices/$relative"
done < "$evidence/notice-paths.nul"

if [[ "$mode" = complete ]]; then
    for archive in libopencv_imgcodecs.a libopencv_imgproc.a libopencv_geometry.a libopencv_flann.a libopencv_core.a; do
        [[ -s "$prefix/opencv/lib/$archive" ]] || fail "missing installed archive: $archive"
    done
    for archive in liblibjpeg-turbo.a liblibpng.a liblibtiff.a libipphal.a libzlib.a libippiw.a libippicv.a; do
        [[ -s "$prefix/opencv/lib/opencv5/3rdparty/$archive" ]] || fail "missing installed archive: $archive"
    done
    [[ -s "$prefix/build/onnxruntime/Release/_deps/re2-build/libre2.a" ]] || fail 'RE2 archive missing; explicitly build target re2'
    for archive in common flatbuffers framework graph lora mlas optimizer providers session util; do
        [[ -s "$prefix/build/onnxruntime/Release/libonnxruntime_$archive.a" ]] || fail "missing ONNX Runtime archive: $archive"
    done
    find "$prefix/build/onnxruntime/Release" "$prefix/opencv" \
        -type f -name '*.a' -print0 | sort -z | xargs -0 -r sha256sum > "$evidence/static-archives.sha256"
    [[ -s "$evidence/static-archives.sha256" ]] || fail 'no archive inventory'
fi
find "$evidence" -type f ! -name SHA256SUMS -print0 | sort -z | xargs -0 -r sha256sum > "$evidence/SHA256SUMS"
