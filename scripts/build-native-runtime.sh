#!/usr/bin/env bash
# CPU-only GNU native dependencies. Run in an isolated builder, not the host.
set -euo pipefail
export LC_ALL=C

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
# shellcheck source=native-build-tools.lock
source "$script_dir/native-build-tools.lock"

fail() { printf '%s\n' "$*" >&2; exit 1; }
[[ $# -ge 1 && $# -le 2 ]] || fail "usage: $0 ABSOLUTE_PREFIX [--configure-only]"
prefix=$1
mode=${2:-build}
[[ "$prefix" = /* && "$prefix" != / ]] || fail 'prefix must be a dedicated absolute directory'
[[ "$prefix" != *[[:space:]]* && "$prefix" != *"'"* ]] || fail 'prefix must contain no whitespace or single quote'
[[ "$mode" = build || "$mode" = --configure-only ]] || fail 'unknown mode'
[[ $(uname -s) = Linux && $(uname -m) = x86_64 ]] || fail 'this native lock targets GNU Linux x86-64'
getconf GNU_LIBC_VERSION > /dev/null || fail 'a GNU/glibc builder is required'
[[ ! -f /etc/alpine-release ]] || fail 'Alpine builders are excluded'

for tool in git cmake ninja nasm curl sha256sum awk sort find g++ gcc ar; do
    command -v "$tool" > /dev/null || fail "missing builder tool: $tool"
done
[[ $(cmake --version | head -n 1) = "cmake version $CMAKE_VERSION" ]] || fail "requires CMake $CMAKE_VERSION"
[[ $(ninja --version) = "$NINJA_EXECUTABLE_VERSION" ]] || fail "requires Ninja executable $NINJA_EXECUTABLE_VERSION from package $NINJA_VERSION"
printf '%s  %s\n' "$NINJA_EXECUTABLE_SHA256" "$(command -v ninja)" | sha256sum --check --status || fail 'Ninja executable SHA-256 differs'
# Debian's reproducible build omits the optional "compiled on" suffix.
# Parse the version field, accepting either representation without weakening the pin.
[[ $(nasm -v | awk '$1 == "NASM" && $2 == "version" {print $3}') = "$NASM_VERSION" ]] || fail "requires NASM $NASM_VERSION"
case "${CFLAGS:-} ${CXXFLAGS:-} ${LDFLAGS:-}" in
    *-march=native*|*-mcpu=native*) fail 'host-specific native compiler flags are excluded' ;;
esac

mkdir -p "$prefix/src" "$prefix/downloads" "$prefix/evidence"
checkout() {
    local name=$1 repository=$2 revision=$3 directory="$prefix/src/$1"
    if [[ ! -d "$directory/.git" ]]; then
        [[ ! -e "$directory" ]] || fail "refusing non-git source directory: $directory"
        git init --quiet "$directory"
        git -C "$directory" remote add origin "$repository"
    fi
    [[ $(git -C "$directory" remote get-url origin) = "$repository" ]] || fail "source remote differs: $name"
    if ! git -C "$directory" cat-file -e "$revision^{commit}" 2>/dev/null; then
        git -C "$directory" fetch --depth=1 origin "$revision"
    fi
    [[ -z $(git -C "$directory" status --porcelain --untracked-files=no) ]] || fail "modified tracked source: $name"
    git -C "$directory" checkout --quiet --detach "$revision"
    [[ $(git -C "$directory" rev-parse HEAD) = "$revision" ]] || fail "wrong source revision: $name"
    git -C "$directory" submodule update --init --recursive --depth=1
}
checkout onnxruntime "$ONNXRUNTIME_REPOSITORY" "$ONNXRUNTIME_REVISION"
checkout opencv "$OPENCV_REPOSITORY" "$OPENCV_REVISION"

# Upstream checks this IPP asset with MD5. Add the retained immutable SHA-256
# before configuration, and make OpenCV obtain it from this verified local copy.
ipp="$prefix/downloads/$IPP_FILENAME"
if [[ ! -f "$ipp" ]]; then
    curl --fail --location --proto '=https' --tlsv1.2 --retry 3 \
        "https://raw.githubusercontent.com/opencv/opencv_3rdparty/$IPP_REVISION/ippicv/$IPP_FILENAME" \
        --output "$ipp.partial"
    printf '%s  %s\n' "$IPP_SHA256" "$ipp.partial" | sha256sum --check --status || fail 'IPP SHA-256 mismatch'
    mv -- "$ipp.partial" "$ipp"
fi
printf '%s  %s\n' "$IPP_SHA256" "$ipp" | sha256sum --check --status || fail 'cached IPP SHA-256 mismatch'

ort_source="$prefix/src/onnxruntime"
ort_build="$prefix/build/onnxruntime/Release"
opencv_build="$prefix/build/opencv"
cmake -S "$ort_source/cmake" -B "$ort_build" -G Ninja \
    -DCMAKE_BUILD_TYPE=Release -DCMAKE_POSITION_INDEPENDENT_CODE=ON \
    -DCMAKE_C_COMPILER=gcc -DCMAKE_CXX_COMPILER=g++ \
    -DFETCHCONTENT_BASE_DIR="$ort_build/_deps" \
    -DFETCHCONTENT_TRY_FIND_PACKAGE_MODE=NEVER \
    -Donnxruntime_BUILD_SHARED_LIB=OFF -Donnxruntime_BUILD_UNIT_TESTS=OFF \
    -Donnxruntime_ENABLE_PYTHON=OFF -Donnxruntime_ENABLE_TRAINING=OFF \
    -Donnxruntime_ENABLE_TRAINING_APIS=OFF -Donnxruntime_ENABLE_TRAINING_OPS=OFF \
    -Donnxruntime_USE_TELEMETRY=OFF -Donnxruntime_USE_MIMALLOC=OFF \
    -Donnxruntime_BUILD_FOR_NATIVE_MACHINE=OFF -Donnxruntime_ENABLE_LTO=OFF \
    -Donnxruntime_USE_CUDA=OFF -Donnxruntime_USE_OPENVINO=OFF \
    -Donnxruntime_USE_DNNL=OFF -Donnxruntime_USE_TENSORRT=OFF \
    -Donnxruntime_MINIMAL_BUILD=OFF -Donnxruntime_REDUCED_OPS_BUILD=OFF \
    -Donnxruntime_DISABLE_CONTRIB_OPS=OFF -Donnxruntime_USE_FULL_PROTOBUF=OFF \
    -Donnxruntime_BUILD_BENCHMARKS=OFF

cmake -S "$prefix/src/opencv" -B "$opencv_build" -G Ninja \
    -DCMAKE_BUILD_TYPE=Release -DCMAKE_POSITION_INDEPENDENT_CODE=ON \
    -DCMAKE_C_COMPILER=gcc -DCMAKE_CXX_COMPILER=g++ \
    -DCMAKE_INSTALL_PREFIX="$prefix/opencv" -DBUILD_SHARED_LIBS=OFF \
    -DBUILD_LIST=core,flann,geometry,imgproc,imgcodecs \
    -DBUILD_TESTS=OFF -DBUILD_PERF_TESTS=OFF -DBUILD_EXAMPLES=OFF \
    -DBUILD_opencv_apps=OFF -DBUILD_opencv_python2=OFF -DBUILD_opencv_python3=OFF \
    -DBUILD_JAVA=OFF -DWITH_FFMPEG=OFF -DWITH_GSTREAMER=OFF \
    -DWITH_GTK=OFF -DWITH_QT=OFF -DWITH_OPENCL=OFF \
    -DWITH_TBB=OFF -DWITH_OPENMP=OFF -DWITH_LAPACK=OFF \
    -DCV_TRACE=OFF -DWITH_ITT=OFF -DWITH_PROTOBUF=OFF \
    -DWITH_EIGEN=ON -DEigen3_DIR="$ort_build/_deps/eigen3-build" \
    -DEIGEN_INCLUDE_PATH="$ort_build/_deps/eigen3-src" \
    -DWITH_IPP=ON -DOPENCV_IPPICV_URL="file://$prefix/downloads/" \
    -DWITH_JPEG=ON -DBUILD_JPEG=ON -DENABLE_LIBJPEG_TURBO_SIMD=ON \
    -DWITH_PNG=ON -DBUILD_PNG=ON -DWITH_TIFF=ON -DBUILD_TIFF=ON \
    -DBUILD_ZLIB=ON -DWITH_WEBP=OFF -DWITH_OPENJPEG=OFF \
    -DWITH_JASPER=OFF -DWITH_OPENEXR=OFF -DWITH_AVIF=OFF \
    -DCPU_BASELINE=SSE3 -DCPU_DISPATCH=SSE4_1,SSE4_2,AVX,FP16,AVX2,AVX512_SKX

"$script_dir/record-native-runtime.sh" "$prefix" "$prefix/evidence" --configuration-only
if [[ "$mode" = --configure-only ]]; then
    exit 0
fi

# The CPU budget applies to each sequential native build. Do not start both at once.
cmake --build "$ort_build" --parallel 2
# ort-sys' full static closure requires RE2 even when the default target omits it.
cmake --build "$ort_build" --target re2 --parallel 2
cmake --build "$opencv_build" --parallel 2
cmake --install "$opencv_build"
"$script_dir/record-native-runtime.sh" "$prefix" "$prefix/evidence"

cat > "$prefix/cargo-env.sh" <<EOF
# Source from an isolated pinned Rust/Clang builder; no runtime library search.
unset DOCS_RS
export CARGO_BUILD_JOBS=2
export ORT_LIB_PATH='$ort_build'
export ORT_CXX_STDLIB='static:-bundle=stdc++'
export ORT_SKIP_DOWNLOAD=1
export OPENCV_INCLUDE_PATHS='$prefix/opencv/include/opencv5'
export OPENCV_LINK_PATHS='$prefix/opencv/lib,$prefix/opencv/lib/opencv5/3rdparty'
export OPENCV_LINK_LIBS='static=libopencv_imgcodecs.a,static=libopencv_imgproc.a,static=libopencv_geometry.a,static=libopencv_flann.a,static=libopencv_core.a,static=liblibjpeg-turbo.a,static=liblibpng.a,static=liblibtiff.a,static=libipphal.a,static=libzlib.a,static=libippiw.a,static=libippicv.a'
export CXXSTDLIB='static:-bundle=stdc++'
export RUSTFLAGS='-Ctarget-feature=+crt-static'
EOF
printf 'Native archives and evidence saved under %s\n' "$prefix"
