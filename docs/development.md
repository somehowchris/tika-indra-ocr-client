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

The release profile uses optimization level 3, thin LTO, one codegen unit, checked arithmetic and aborting panics. `maxperf` changes only to fat LTO and is a comparison candidate. `profiling` preserves symbols. Neither a name nor `target-cpu=native` proves a performance improvement; portable published images use runtime dispatch. Whole-stack measurement must include Tika, OCR and the load generator in the same verified 1/2-CPU, 3-GiB parent cgroup. The [completed synthetic ablations](../runtime/validation/native-ablations/2026-10-18/README.md) do not promote fat LTO or another default. The [paired math experiment](../runtime/validation/math-compaction/2026-10-18/paired-study/README.md) includes two-CPU regressions despite fewer decoder row steps.

Native compilation is the dominant observed rebuild cost: the linked historical job repeated 4,436.7 seconds after a missing cache blob. [The build-cost protocol](../evaluation/build-cost/README.md) separates native recipe/content/OCI identity, current Cargo artifact selection, cache ownership and compiled dependency reuse. [The SDK](../evaluation/build-cost/SDK.md) and [Cargo Chef layer](../evaluation/build-cost/CHEF-COOK.md) are opt-in experiments. Source/layout checks and same-builder restoration do not establish fresh-runner reuse, seven-role equivalence or a measured speedup. The trusted release workflow retains those execution gates separately; automatic cache writers are disabled by default.

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

The original profiling/proof and documentation branches continue in [11](https://github.com/somehowchris/tika-indra-ocr-client/pull/11) and [12](https://github.com/somehowchris/tika-indra-ocr-client/pull/12). Recovery [26](https://github.com/somehowchris/tika-indra-ocr-client/pull/26) retains signed-release source, [27](https://github.com/somehowchris/tika-indra-ocr-client/pull/27) adds real native continuous acceptance, and [28](https://github.com/somehowchris/tika-indra-ocr-client/pull/28) retains signed-candidate scientific execution. The completion integration reconciles these branches; they are not one numerical PR chain.

Review the completion stack in this dependency order. Each row after 29 is based on the previous completion row:

| PR | Purpose |
|---|---|
| [29](https://github.com/somehowchris/tika-indra-ocr-client/pull/29) | Reconcile interrupted release, evaluation, tracing and documentation checkpoints |
| [30](https://github.com/somehowchris/tika-indra-ocr-client/pull/30) | Actual W3C context through gateway, forked Java, bridge and native OCR |
| [31](https://github.com/somehowchris/tika-indra-ocr-client/pull/31) | Bounded nextest, model-cache/configuration/API operational checks |
| [32](https://github.com/somehowchris/tika-indra-ocr-client/pull/32) | Correct perf folding and bounded Docker profiling admission |
| [33](https://github.com/somehowchris/tika-indra-ocr-client/pull/33) | Frozen local study with an owned aggregate application/load-generator cgroup |
| [34](https://github.com/somehowchris/tika-indra-ocr-client/pull/34) | Canonical-padding two-owner pilot |
| [35](https://github.com/somehowchris/tika-indra-ocr-client/pull/35) | Strict admitted artifacts and reproducible Quarto/Seaborn reporting |
| [36](https://github.com/somehowchris/tika-indra-ocr-client/pull/36) | Exact full-stack trace gate before release signing |
| [37](https://github.com/somehowchris/tika-indra-ocr-client/pull/37) | Setup documentation and qualified native/trace evidence |
| [38](https://github.com/somehowchris/tika-indra-ocr-client/pull/38) | Fresh native parity, deployment evidence and strict immutable admission |
| [39](https://github.com/somehowchris/tika-indra-ocr-client/pull/39) | Frozen build-cost experiments and cache/Chef investigation |
| [40](https://github.com/somehowchris/tika-indra-ocr-client/pull/40) | Exact current-Cargo executable selection and independent material collection |
| [41](https://github.com/somehowchris/tika-indra-ocr-client/pull/41) | Immutable native SDK packaging and content/recipe/OCI admission |
| [42](https://github.com/somehowchris/tika-indra-ocr-client/pull/42) | Prespecified native LTO, spinning and batch-cap ablations |
| [43](https://github.com/somehowchris/tika-indra-ocr-client/pull/43) | Actual live continuous resource/PID admission and bounded owned cleanup |
| [44](https://github.com/somehowchris/tika-indra-ocr-client/pull/44) | Isolated same-age math-row compaction experiment |
| [45](https://github.com/somehowchris/tika-indra-ocr-client/pull/45) | Include Java and exporter teardown in deployment grace budgets |
| [46](https://github.com/somehowchris/tika-indra-ocr-client/pull/46) | Complete 46-case/two-cap native profile with verified private bundle transport |
| [47](https://github.com/somehowchris/tika-indra-ocr-client/pull/47) | Purpose-owned cache policy and exact historical build-cost evidence |
| [48](https://github.com/somehowchris/tika-indra-ocr-client/pull/48) | Exact HTTP reference replay and bounded telemetry experiment |
| [49](https://github.com/somehowchris/tika-indra-ocr-client/pull/49) | Paired math measurements and strict ablation resource evidence |
| [50](https://github.com/somehowchris/tika-indra-ocr-client/pull/50) | Supported CRI version receipt and opt-in signed SDK/Cargo Chef fresh-runner proof |

The completed profile, math and ablation results are synthetic diagnostics; telemetry paired acceptance, signed native release, SDK/Chef fresh-runner cost proof and the complete multilingual report remain separate milestones. A draft remains a draft until its relevant acceptance gate is complete. The unrelated dependency-update PR is left untouched. [Results](results.md) records producing revisions and actual runs rather than inferring success from the presence of tests.

## Continuation parts

These twenty published drafts continue #50 in order. Each base is the preceding row; #51 is based on #50. The immutable source and accepted measurements remain separately named in the results ledger. All drafts remain unmerged.

| PR | Purpose | Head |
|---|---|---|
| [51](https://github.com/somehowchris/tika-indra-ocr-client/pull/51) | Complete native ablation evidence | `0d2c931` |
| [52](https://github.com/somehowchris/tika-indra-ocr-client/pull/52) | Actual telemetry correctness and Sentry wire decoding | `e5dbf62` |
| [53](https://github.com/somehowchris/tika-indra-ocr-client/pull/53) | Frozen build comparison controls and complete materials | `ad24c12` |
| [54](https://github.com/somehowchris/tika-indra-ocr-client/pull/54) | Seaborn ablation figures and measured-result ledger | `1e032a3` |
| [55](https://github.com/somehowchris/tika-indra-ocr-client/pull/55) | Explicit verified native-input selection; no enabled lock | `fb46176` |
| [56](https://github.com/somehowchris/tika-indra-ocr-client/pull/56) | Complete continuous-pilot negative result | `90130a6` |
| [57](https://github.com/somehowchris/tika-indra-ocr-client/pull/57) | Bounded module and ownership architecture contract | `02e3ad5` |
| [58](https://github.com/somehowchris/tika-indra-ocr-client/pull/58) | Exact generated Chef Docker context | `7871663` |
| [59](https://github.com/somehowchris/tika-indra-ocr-client/pull/59) | Fixed native application epoch and Chef key | `66a8c15` |
| [60](https://github.com/somehowchris/tika-indra-ocr-client/pull/60) | Complete telemetry raw packet and offline replay | `0dbccad` |
| [61](https://github.com/somehowchris/tika-indra-ocr-client/pull/61) | Actual gateway Sentry request correlation | `29db93e` |
| [62](https://github.com/somehowchris/tika-indra-ocr-client/pull/62) | Disabled experimental SDK proof recovery | `2060e20` |
| [63](https://github.com/somehowchris/tika-indra-ocr-client/pull/63) | Completed amended seven-binary raw content proof | `3fc9576` |
| [64](https://github.com/somehowchris/tika-indra-ocr-client/pull/64) | Checksum-pinned build-test dependencies | `54165c5` |
| [65](https://github.com/somehowchris/tika-indra-ocr-client/pull/65) | Actual pinned CRI version parsing | `78105ec` |
| [66](https://github.com/somehowchris/tika-indra-ocr-client/pull/66) | Pinned evaluator formatting | `a1feb46` |
| [67](https://github.com/somehowchris/tika-indra-ocr-client/pull/67) | Exact Docker-owned kind cgroup admission | `8dea245` |
| [68](https://github.com/somehowchris/tika-indra-ocr-client/pull/68) | Portable evaluator test fixture storage | `cb71591` |
| [69](https://github.com/somehowchris/tika-indra-ocr-client/pull/69) | Raw server PodList/watch boundary | `8fed914` |
| [70](https://github.com/somehowchris/tika-indra-ocr-client/pull/70) | Complete Git history for frozen-input checks | `60c849a` |
