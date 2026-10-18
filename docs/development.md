# Local Rust development

The dependency-free Tika executable adapter is the root Cargo package. The persistent OCR service, native engine, gateway, model cache and telemetry live in the separate `runtime/` workspace. The evaluation runner has its own lockfile in `evaluation/runner/`. This keeps ONNX/OpenCV resolution out of the bridge's static build and makes each component independently testable.

## First checks

Install GNU Rust 1.99.1 from `rust-toolchain.toml`, a C/C++ compiler and Git. The following need neither models nor native OCR libraries:

```sh
rustup toolchain install 1.99.1 --profile minimal --component rustfmt,clippy
cargo test --locked
cargo test --locked --manifest-path runtime/Cargo.toml \
  -p indra-core -p indra-models -p indra-telemetry -p indra-gateway
cargo test --locked --manifest-path evaluation/runner/Cargo.toml
cargo clippy --locked --manifest-path runtime/Cargo.toml \
  -p indra-core -p indra-models -p indra-telemetry -p indra-gateway \
  --all-targets -- -D warnings
```

Model cache tests use local HTTP fixtures. Telemetry tests decode actual OTLP and Sentry messages sent to local collectors. Process tests use controlled Rust children; separately marked native and deployment gates require real models, native linkage or Docker/Kubernetes. A `DOCS_RS=1` type check avoids native binding discovery and is **not** an executable native test.

## Run the native service

The [Docker quickstart](../README.md#local-docker) builds the complete service and Tika stack. For a host-linked development build, create the pinned native archives using [native-build.md](native-build.md), source that build's `cargo-env.sh`, and follow [the service guide](runtime-service.md#local-development). The archives, headers and compiler ABI must agree; do not combine an arbitrary system OpenCV with the recorded static closure.

```sh
. /opt/indra-native/cargo-env.sh
cargo build --locked --manifest-path runtime/Cargo.toml -p indra-server
runtime/target/debug/indra-server --math true \
  --model-dir ./models/ed345c7f158eddbc7dee0c6a7269afb75a94f7e8 models fetch
runtime/target/debug/indra-server --math true \
  --model-dir ./models/ed345c7f158eddbc7dee0c6a7269afb75a94f7e8 serve
```

Adjust the target path if the native environment sets `CARGO_TARGET_DIR` or an explicit target triple. `models verify`, `check-config`, `openapi`, `completions`, `probe` and `drain` are executable subcommands; consult `--help` for their arguments. `.env` is loaded only in debug builds. Release operation uses explicit config files, environment or flags.

## Measure a change

```sh
cargo bench --locked --manifest-path runtime/Cargo.toml \
  -p indra-core --bench control_path
cargo bench --locked --manifest-path runtime/Cargo.toml \
  -p indra-telemetry --bench propagation
cargo bench --locked --bench streaming
```

Criterion timings and standalone allocation tests answer different questions. Timings use the declared allocator and compiler profile. Allocation programs disable the concurrent libtest harness and count all six allocation/deallocation/reallocation call and byte counters only inside the measured region; reporting happens afterward. The zero-allocation claim covers those bounded operations, not image decoding, HTTP internals or ONNX Runtime. Avoid changing a benchmark and its comparison fixture without explaining the new question.

The release profile uses optimization level 3, thin LTO, one codegen unit, checked arithmetic and aborting panics. `maxperf` changes only to fat LTO and is a comparison candidate. `profiling` preserves symbols. Neither a name nor `target-cpu=native` proves a performance improvement; portable published images use runtime dispatch. Whole-stack measurement must include Tika, OCR and the load generator in the same verified 1/2-CPU, 3-GiB parent cgroup.

## Review order

| PR | Purpose | Depends on |
|---|---|---|
| [1](https://github.com/somehowchris/tika-indra-ocr-client/pull/1) | Typed configuration, bounded admission/drain/FIFO and telemetry foundation | Main |
| [3](https://github.com/somehowchris/tika-indra-ocr-client/pull/3) | Immutable, verified model cache | 1 |
| [4](https://github.com/somehowchris/tika-indra-ocr-client/pull/4) | Reproducible static GNU native dependencies | 3 |
| [5](https://github.com/somehowchris/tika-indra-ocr-client/pull/5) | Creusot proof of the bounded batch planner | 4; independent proof branch |
| [6](https://github.com/somehowchris/tika-indra-ocr-client/pull/6) | Native Indra pipeline and real Python/Rust parity | 4 |
| [7](https://github.com/somehowchris/tika-indra-ocr-client/pull/7) | Streaming Tika executable adapter | 6 |
| [8](https://github.com/somehowchris/tika-indra-ocr-client/pull/8) | Axum service, native workers, probes, lifecycle, useful telemetry | 7 |
| [9](https://github.com/somehowchris/tika-indra-ocr-client/pull/9) | Tika gateway, process supervision, Compose/Kubernetes drain | 8 |
| [10](https://github.com/somehowchris/tika-indra-ocr-client/pull/10) | Frozen multilingual and laboratory evaluation inputs | 9 |

Profiling/allocation/proof strengthening, signed native release execution and the final measured report follow as distinct reviewable changes. A draft remains a draft until its relevant acceptance gate is complete. The unrelated dependency-update PR is left untouched. [Results](results.md) records producing revisions and actual runs rather than inferring success from the presence of tests.
