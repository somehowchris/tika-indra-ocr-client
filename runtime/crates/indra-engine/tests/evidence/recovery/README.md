# Native-engine recovery checkpoint

The previous workspace was reset following disk exhaustion. The files in this directory preserve facts retained in the conversation and provide a starting point for source reconstruction. They are **not recovered historical binaries, source bytes, trace arrays or execution logs**.

`provenance.json` distinguishes historical results from fresh verification. Historical hashes identify artifacts that existed before the reset; they do not verify reconstructed source. All reconstructed implementation files require a new hosted static build, real model execution, stage parity and malformed-input checks before publication as a validated implementation.

The latest reported remote checkpoint is `d43b708fb3a50ecc65bf5d286036a687defd3af9` (core planner and model cache). The native engine was unpushed. The parent agent is restoring the repository. No local native rebuild is planned; the whole workspace must stay below 20 GiB.

## Retained architecture

- Actual ONNX Runtime CPU sessions and OpenCV preprocessing in Rust, not a Python inference client.
- Separate production runtime workspace; the small GNU-static Tika command-line adapter remains independent.
- Pinned FP32 models verified at startup by `indra-models`; no request-time model acquisition.
- Bounds: 20 MiB encoded input, 4 MiB UTF-8 output, 25 million decoded pixels, 16,384 pixels per side, 16 image frames, 1,024 text lines.
- ORT threads and OpenCV threads each configurable from one to two, both spinning switches exposed, maximum recognition batch count 64.
- Source-compatible in-page width grouping is separate from the core `BatchPlan` policy. Ordinary text uses source width-ratio 1.3 and proxy-area budget 40,960; math uses height 64, ratio 4 and area budget 655,360. Exact grouping details must be reread from the pinned upstream before reconstruction.
- Bounded reusable heap arrays per worker, borrowed positional ONNX inputs, typed errors, no owned `Vec`, `dyn`, `.clone()`, `.to_owned()` or `.to_vec()` in engine code.
- Ordinary corrupt/truncated image failures become `InvalidImage`; unrelated native failures remain fatal typed errors. Reject duplicate TIFF dimensions and duplicate/late APNG control metadata before allocating decoded images, with bounded native multipage decode as defense in depth.
- Selected default scope: Tika `Page.text`, `adaptive=True`, `table_heads=False`; Tika handles standalone PDF parsing and rasterization.

## Files requiring reconstruction

The lost crate included storage, image guard/decoding, model sessions, detection, orientation/skew, layout classification, crop/split handling, CTC recognition and script routing, math autoregressive decoding, reading order, text assembly, trace support, native information, a real OCR example, and focused tests. The reading-order port is delegated back to its original author.

The native build script must pin source commits, build CPU-only static GNU dependencies with portable runtime CPU dispatch, explicitly build ONNX Runtime's otherwise omitted RE2 archive, preserve native/model notices, and record real archive/configuration hashes. It must not silently fall back to downloaded ONNX binaries or accidentally omit JPEG NASM SIMD.

## Historical failure lessons to preserve

OpenCV's PNG decoder can recover from some damaged compressed-stream checksums and emit frames after warning. A corruption fixture must exercise an actual failure (for example an invalid IHDR color type), not presume every checksum warning is fatal. The former tests rejected a malformed JPEG lacking scan data and a truncated second TIFF frame without returning partial success.

The binding crate removes one leading `lib` when interpreting link-library configuration. Static archive filenames such as `liblibjpeg-turbo.a` therefore need their exact explicit filenames. The previous successful environment used `ORT_CXX_STDLIB=static:-bundle=stdc++`, the same `CXXSTDLIB`, an explicit GNU target and `RUSTFLAGS=-Ctarget-feature=+crt-static`.
