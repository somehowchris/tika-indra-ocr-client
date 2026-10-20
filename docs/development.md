# Local Rust development

The std-only Tika executable adapter (its one dependency is the `thiserror` derive) is the root Cargo package. The OCR service, native engine, gateway, model cache and telemetry live in the `runtime/` workspace. Keeping ONNX/OpenCV out of the root build keeps the static bridge small and each component independently testable.

## Prerequisites

`rustup` reads `rust-toolchain.toml` (GNU Rust 1.99.1); install `cargo-nextest` 0.9.148 (`cargo install cargo-nextest --locked --version 0.9.148`). Nothing below needs models or native OCR libraries.

## Native-free checks (what CI runs on every pull request)

```sh
cargo nextest run --locked --workspace
cargo test --locked --test streaming_allocations
cargo nextest run --locked --manifest-path runtime/Cargo.toml --config-file runtime/.config/nextest.toml --profile ci \
  -p indra-core -p indra-models -p indra-telemetry --all-features --lib --test exports
cargo test --locked --manifest-path runtime/Cargo.toml -p indra-core -p indra-telemetry --all-features --test allocations
DOCS_RS=1 cargo clippy --locked --manifest-path runtime/Cargo.toml \
  -p indra-core -p indra-models -p indra-telemetry -p indra-engine -p indra-server -p indra-gateway \
  --all-targets --all-features -- -D warnings
```

`DOCS_RS=1` type-checks the native crates against documentation bindings; it never links or runs OCR. The gateway's process-lifecycle gate (`cargo test -p indra-gateway --test process_lifecycle`) needs Linux; the bridge protocol test (`cargo test --test bridge_protocol`) is std-only and runs anywhere. The allocation harnesses run with `cargo test` because they replace the libtest harness. The repository forbids `dyn`, growable `Vec`, `.clone()`, `.to_owned()` and `anyhow` in runtime production sources; CI greps for them.

## Native service loop

```sh
docker buildx build --load --platform linux/amd64 -f docker/native.Dockerfile --target server -t indra-native:dev .
docker compose up -d --no-build --wait --wait-timeout 600
```

The native image builds the pinned GNU toolchain, ONNX Runtime and OpenCV archives once (cached layer), then the service and its acceptance gates, which run the exact static test binaries under 1 and 2 CPU limits ([engine guide](native-engine.md), [service guide](runtime-service.md#local-development), [deployment guide](container-deployment.md)).

## Measure a change

```sh
cargo bench --locked --manifest-path runtime/Cargo.toml -p indra-core --bench control_path
cargo bench --locked --manifest-path runtime/Cargo.toml -p indra-telemetry --all-features --bench propagation
cargo bench --locked --bench streaming
```

Criterion timings use mimalloc and the declared profile; the standalone allocation programs count all six allocator counters inside the measured region only. The release profile is `opt-level = 3`, thin LTO, one codegen unit, checked arithmetic and aborting panics; `maxperf` (fat LTO) and `profiling` (symbols kept) are comparison profiles, and published images never use `target-cpu=native`. Whole-stack measurements must put Tika, OCR and the load generator in one verified 1/2-CPU, 3 GiB parent cgroup; that controller, the profiling pipeline and the build-cost experiments are delivered by the later reviews.

## Review order

| Review | Scope |
|---|---|
| Runtime and Tika integration | Native OCR, bounded service lifecycle, model cache, configuration/API, telemetry, Creusot batch-plan proof and local/Kubernetes setup |
| Indra OCR 1.1 | Four-script routing, n-gram language-model beam search, right-to-left and vertical pages, re-frozen parity |
| Reproducible builds and release | Static GNU images, complete distribution materials, signing, native-input reuse and bounded CI checks |
| Evaluation and measured choices | Frozen corpus/protocols, replay, profiling, comparative study and Quarto/Seaborn reporting |

Each review keeps its acceptance checks with the implementation. The [history index](history.md) preserves the former PR chronology and exact historical packets; the [results ledger](results.md) distinguishes accepted diagnostics, failed attempts and pending release/study work.
