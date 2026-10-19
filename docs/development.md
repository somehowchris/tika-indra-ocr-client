# Local Rust development

The dependency-free Tika executable adapter is the root Cargo package. The persistent OCR service, native engine, gateway, model cache and telemetry live in the separate `runtime/` workspace. The evaluation runner has its own lockfile in `evaluation/runner/`. This keeps ONNX/OpenCV resolution out of the bridge's static build and makes each component independently testable.

## First checks

Install GNU Rust 1.99.1 from `rust-toolchain.toml`, a C/C++ compiler and Git. The following need neither models nor native OCR libraries:

```sh
rustup toolchain install 1.99.1 --profile minimal --component rustfmt,clippy
# CI instead verifies the pinned upstream GNU binary archive.
cargo +1.99.1 install cargo-nextest --locked --version 0.9.148
cargo +1.99.1 nextest run --locked --manifest-path runtime/Cargo.toml \
  --config-file .config/nextest.toml --profile ci \
  -p indra-core -p indra-models -p indra-telemetry --all-features --lib --test exports
cargo +1.99.1 test --locked --manifest-path runtime/Cargo.toml \
  -p indra-core -p indra-telemetry --all-features --test allocations
cargo +1.99.1 test --locked --manifest-path runtime/Cargo.toml \
  -p indra-core -p indra-models -p indra-telemetry --all-features --doc
cargo +1.99.1 clippy --manifest-path runtime/Cargo.toml -p indra-core --all-targets -- -D warnings
cargo +1.99.1 bench --manifest-path runtime/Cargo.toml -p indra-core --bench control_path
```

The [CI installer](../.github/actions/setup-nextest/action.yml) pins the upstream
GNU archive and its SHA-256 before extraction. The [nextest profile](../.config/nextest.toml)
runs at most two test processes, serializes collector/cache groups, applies finite
timeouts and never retries failures. Nextest process isolation does not replace
in-process lifecycle or repeated-initialization assertions: those remain within
their original test functions. Custom allocator harnesses and doctests run with
`cargo test` because they do not use nextest's normal libtest discovery. Native
acceptance still runs the exact exported static test binaries under Docker's
one/two-CPU limits; it is separate from this native-free correctness job.

The static service gate also runs release subprocess checks from a malformed
`.env` working directory, validates configuration precedence and redacted errors,
and generates all shell completions and OpenAPI without starting models.
The HTTP unit gate compares generated error/media contracts with actual router
responses. These checks establish operational behavior, not inference performance.

The core crate needs no model files or running Tika server. It supplies an admission/drain state machine, an array-backed FIFO and an aligned tensor-work guard. Its tests exercise cancellation ownership, maximum counters, bounded arithmetic and allocator calls. Criterion timings use mimalloc; the separate allocation test counts calls to Rust's allocator interface around only the control operations. Neither measurement describes allocations inside HTTP, image decoding or ONNX Runtime.

Model and telemetry tests use local fixtures and collectors. A `DOCS_RS=1` type
check bypasses native binding discovery and is not executable native acceptance.

## Run the native service

Use the [Docker quickstart](../README.md#local-docker) for native engine and service development. It builds the pinned GNU toolchain, ONNX/OpenCV archives and actual static service, then runs Tika alongside it. Rebuild the affected images after changing Rust source and restart the stack; the separate native dependency layer can be reused when its inputs are unchanged.

The [service guide](runtime-service.md#local-development) shows shell-free checks against that running stack. The builder's emitted environment is `/native/cargo-env.sh` **inside its GNU build stage**. Exporting `native-evidence` exports reports only; it does not install libraries or that path on the host. Do not combine arbitrary host OpenCV/ONNX packages with the recorded archive closure. `DOCS_RS=1` cannot validate native linking or execute OCR.

## Measure a change

```sh
cargo bench --locked --manifest-path runtime/Cargo.toml \
  -p indra-core --bench control_path
cargo bench --locked --manifest-path runtime/Cargo.toml \
  -p indra-telemetry --bench propagation
cargo bench --locked --bench streaming
```

Criterion timings and standalone allocation tests answer different questions. Timings use the declared allocator and compiler profile. Allocation programs disable the concurrent libtest harness and count all six allocation/deallocation/reallocation call and byte counters only inside the measured region; reporting happens afterward. The zero-allocation claim covers those bounded operations, not image decoding, HTTP internals or ONNX Runtime. Avoid changing a benchmark and its comparison fixture without explaining the new question.

The release profile uses optimization level 3, thin LTO, one codegen unit, checked arithmetic and aborting panics. `maxperf` changes only to fat LTO and is a comparison candidate. `profiling` preserves symbols. Neither a name nor `target-cpu=native` proves a performance improvement; portable published images use runtime dispatch. Whole-stack measurement must include Tika, OCR and the load generator in the same verified 1/2-CPU, 3-GiB parent cgroup. The [completed synthetic ablations](../runtime/validation/native-ablations/2026-10-18/README.md) do not promote fat LTO or another default. The [paired math experiment](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/runtime/validation/math-compaction/2026-10-18/paired-study/README.md) includes two-CPU regressions despite fewer decoder row steps.

Native compilation is the dominant observed rebuild cost: the linked historical job repeated 4,436.7 seconds after a missing cache blob. [The build-cost protocol](../evaluation/build-cost/README.md) separates native recipe/content/OCI identity, current Cargo artifact selection, cache ownership and compiled dependency reuse. [The SDK](../evaluation/build-cost/SDK.md) and [Cargo Chef layer](../evaluation/build-cost/CHEF-COOK.md) are opt-in experiments. Source/layout checks and same-builder restoration do not establish fresh-runner reuse, seven-role equivalence or a measured speedup. The trusted release workflow retains those execution gates separately; automatic cache writers are disabled by default.

## Review order

The implementation is consolidated into three coherent reviews:

| Review | Scope |
|---|---|
| Runtime and Tika integration | Native OCR, bounded service lifecycle, model cache, configuration/API, telemetry and local/Kubernetes setup |
| Reproducible builds and release | Static GNU images, complete distribution materials, signing, native-input reuse and bounded CI checks |
| Evaluation and measured choices | Frozen corpus/protocols, replay, profiling, comparative study and Quarto/Seaborn reporting |

Each review keeps its acceptance checks with the implementation. Build reuse and runtime optimization candidates remain opt-in until their measured gates pass. The [history index](history.md) preserves the former PR chronology and exact historical packets; the [results ledger](results.md) distinguishes accepted diagnostics, failed attempts and pending release/study work.
