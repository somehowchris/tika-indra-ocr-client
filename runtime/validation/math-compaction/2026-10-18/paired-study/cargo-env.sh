# Source from an isolated pinned Rust/Clang builder; no runtime library search.
unset DOCS_RS
export CARGO_BUILD_JOBS=2
export ORT_LIB_PATH='/native/build/onnxruntime/Release'
export ORT_CXX_STDLIB='static:-bundle=stdc++'
export ORT_SKIP_DOWNLOAD=1
export OPENCV_INCLUDE_PATHS='/native/opencv/include/opencv5'
export OPENCV_LINK_PATHS='/native/opencv/lib,/native/opencv/lib/opencv5/3rdparty'
export OPENCV_LINK_LIBS='static=libopencv_imgcodecs.a,static=libopencv_imgproc.a,static=libopencv_geometry.a,static=libopencv_flann.a,static=libopencv_core.a,static=liblibjpeg-turbo.a,static=liblibpng.a,static=liblibtiff.a,static=libipphal.a,static=libzlib.a,static=libippiw.a,static=libippicv.a'
export CXXSTDLIB='static:-bundle=stdc++'
export RUSTFLAGS='-Ctarget-feature=+crt-static'
