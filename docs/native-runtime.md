# Native CPU runtime

## Recovery status

The first native engine and its local build artifacts were lost when the workspace reset after disk exhaustion. The files in this branch are reconstructed source. The historical summary in `runtime/crates/indra-engine/tests/evidence/recovery/historical-provenance.json` records previously reported results, including artifact identifiers, but the underlying executable, logs and trace arrays are absent. **Those results do not validate reconstructed code.** A fresh hosted static build and stage parity run are required.

The recovered remote checkpoint contains the original core and model cache. Reconstruction commits preserve source incrementally before any expensive new build. Do not start another local native build: local workspace use is capped at 20 GiB. Hosted builds must upload source, configuration, notices and evidence before advancing to another phase; cache native dependencies separately from the Rust target directory.

## Pinned boundary

| Component | Exact source |
|---|---|
| Indra reference | [0.3.2, `3d773aeedc5d79d1d0a2a04007a9075cd71a59f4`](https://github.com/datalab-to/indra/tree/3d773aeedc5d79d1d0a2a04007a9075cd71a59f4) |
| FP32 model snapshot | [`ed345c7f158eddbc7dee0c6a7269afb75a94f7e8`](https://huggingface.co/datalab-to/indra-ocr/tree/ed345c7f158eddbc7dee0c6a7269afb75a94f7e8), verified by `indra-models` |
| ONNX Runtime | [1.31.0, `402979ac1bef89dcd55add41eaa7277c407599eb`](https://github.com/microsoft/onnxruntime/tree/402979ac1bef89dcd55add41eaa7277c407599eb) |
| Rust ONNX bindings | [`ort` 2.0.0-rc.14, `1ee1f842e8a381153ede7a506f01734e545854e0`](https://github.com/pykeio/ort/tree/1ee1f842e8a381153ede7a506f01734e545854e0) |
| OpenCV | [5.0.0, `40738fb16ceddb5fb3fea747585f7ce6abb0605b`](https://github.com/opencv/opencv/tree/40738fb16ceddb5fb3fea747585f7ce6abb0605b) |
| Rust OpenCV bindings | [0.101.1, `901d7a221b6172846f77d38c2beb868fab18ef2c`](https://github.com/twistedfall/opencv-rust/tree/901d7a221b6172846f77d38c2beb868fab18ef2c) |
| Intel IPP asset | [OpenCV third-party commit `406d398c436d0465c8e53dd432d9ecd9301d5f4a`](https://github.com/opencv/opencv_3rdparty/tree/406d398c436d0465c8e53dd432d9ecd9301d5f4a), SHA-256 in the build lock |

The serving path performs image decoding, orientation, deskew, adaptive line detection, line recognition, Chinese routing, optional math, reading order and `Page.text` assembly in Rust with native ONNX Runtime/OpenCV. Tika owns PDF extraction and rasterization. PDFium, upstream markdown/table rendering and the optional absent mask/INT8 models are outside this selected path. `table_heads=False` and adaptive detection remain explicit reference settings.

## Reconstructing the static native closure

Use a pinned GNU/glibc Debian builder with GCC/G++, GNU binutils, Git, curl, NASM, CMake and Ninja. No Alpine or musl builder is permitted. The script invokes standard build tools and does not install packages globally. It requires the historical CMake 4.4.4, Ninja 1.13.2 and NASM 2.16.03 versions; the lock retains the exact historical tool-wheel hashes for the builder's acquisition step. Clang/libclang are builder-only OpenCV binding generators. The final executable does not dynamically load them.

```sh
scripts/build-native-runtime.sh /opt/indra-native
. /opt/indra-native/cargo-env.sh
cargo build --manifest-path runtime/Cargo.toml --locked \
  --target x86_64-unknown-linux-gnu --release -p indra-engine --example ocr
```

The example command becomes an acceptance gate once its reconstruction lands. It is not a claim that this intermediate checkpoint includes a complete engine.

The native script builds sequentially with two compilation jobs, pins both Git revisions and each upstream's pinned dependency declarations, verifies IPP with an additional SHA-256 check, and uses full ONNX-format CPU support. It disables the Python binding, training, telemetry, GPU providers and ONNX Runtime's own mimalloc override. The Rust serving binary installs the selected allocator separately.

OpenCV includes core, flann, geometry, imgproc and imgcodecs with bundled JPEG, PNG and TIFF. JPEG NASM SIMD, Eigen and IPP must be enabled; the audit fails on missing configuration. OpenCL, OpenMP, TBB, LAPACK, protobuf and tracing/ITT are disabled. SSE3 is the portable baseline; OpenCV retains its explicit SSE4.1/SSE4.2/AVX/FP16/AVX2/AVX512_SKX runtime dispatch. No `-march=native` flag is accepted. Do not describe these compiled kernels as the kernel selected for every inference; selection requires real instrumentation and hardware evidence.

ONNX Runtime's default target can omit the RE2 archive required by `ort-sys`' full static closure. The script explicitly builds target `re2`. The OpenCV binding configuration uses exact archive filenames: the parser removes one leading `lib`, and `liblibjpeg-turbo.a` must resolve to the installed file of that name.

Use `scripts/record-native-runtime.sh PREFIX EVIDENCE` after compilation. It records source state, dependency declarations, CMake caches, configuration headers, tool versions, every native archive hash and third-party notices. `--configuration-only` records a distinct incomplete state. An inventory and successful link are separate gates; neither proves OCR correctness.

## Fresh verification gates

1. Save source and build configuration as hosted artifacts before compilation. Maintain enough free space for the native archives; stop before exhaustion.
2. Build the real GNU target and inspect ELF headers. A fully static claim requires no `PT_INTERP` and no `DT_NEEDED`; save the executable hash and byte count.
3. Run all native unit tests with the actual linked libraries, then Clippy and formatting checks. `DOCS_RS` checks only type correctness and cannot establish linkage or execution.
4. Recreate malformed/truncated PNG, JPEG and TIFF fixtures, duplicate TIFF dimension tags and duplicate/late APNG controls. Verify predecode limits and reject truncated later frames without partial success.
5. Execute the exact Python reference and Rust port on the same inputs/settings. Compare the actual preprocessing tensors, detector output, script decisions, recognition shapes/results, math steps and reading-order decisions before final UTF-8 output. Save per-stage tolerances, every observed difference, model/fixture hashes and compiler/runtime versions.
6. Only then measure through Tika under whole-stack one-CPU and two-CPU limits. Trace instrumentation is off during timing. Preserve batch histograms, aligned/valid columns, spinning switches, OpenCV/ORT threads, CPU affinity/quota and memory limits for every arm.

Historical testing covered ten cases with 1,767 stages, 1,735 bit-identical stages and ten equal text outputs. Its floating comparison used `atol=1e-5`, `rtol=1e-5`; maximum absolute difference was `2.384185791015625e-5`, which passed the combined absolute/relative rule. These retained figures are a recovery target, not new proof. The old reference was Python 3.13.16, NumPy 2.5.4, ORT 1.31.0 and OpenCV 5.0.0.93. Its wheel enabled Intel ITT tracing; the source build disabled tracing, so any future comparison must describe the entire native configuration rather than attribute differences solely to Rust versus Python.

## Redistribution and interpretation

The closure includes a prebuilt Intel IPP asset; it is not an entirely source-built native dependency tree. Preserve its Intel Simplified Software License and third-party notice, alongside ONNX Runtime, OpenCV, JPEG IJG and other bundled dependency notices, including the Rubik font OFL where distributed. The recorder collects notices from dependency sources and the extracted IPP package. This collection supplements, rather than replaces, the image's complete redistribution materials.

Model files and generated model vocabulary material retain their separate model license. Benchmark predictions must not be silently licensed as Apache adapter code. See `THIRD_PARTY_NOTICES.md` and the checked model license. GNU/glibc static redistribution requires the accompanying relinking materials described in `docs/relinking.md`.

Parity establishes implementation agreement. It does not turn a shared wrong formula, unsupported-script result, or weak handwriting result into an accuracy success.
