# Local Rust development

The std-only Tika executable adapter is the root Cargo package. Its only external dependency is the `thiserror` derive; it shares the dependency-free `no_std` `indra-trace-context` path crate with the runtime telemetry. The OCR service, native engine, gateway, model cache and telemetry live in the `runtime/` workspace. Keeping ONNX/OpenCV out of the root build keeps the static bridge small and each component independently testable.

## Prerequisites

`rustup` reads `rust-toolchain.toml` (GNU Rust 1.99.1); install `cargo-nextest` 0.9.148 (`cargo install cargo-nextest --locked --version 0.9.148`). Nothing below needs models or native OCR libraries.

## Native-free checks

The first four commands are the core of the `check` job in [ci.yml](../.github/workflows/ci.yml), which runs on every pull request and every release. The rest come from [rust-runtime.yml](../.github/workflows/rust-runtime.yml), which runs on pull requests that touch `runtime/`, `shared/`, the root `Cargo.toml` or the toolchain files, and in every release before images are published.

```sh
cargo fmt --all --check
cargo nextest run --locked --workspace --lib --bins --test bridge_protocol --test trace_wire
cargo test --locked --test streaming_allocations
cargo clippy --locked --workspace --all-targets -- -D warnings

cargo nextest run --locked --manifest-path runtime/Cargo.toml --config-file runtime/.config/nextest.toml --profile ci \
  -p indra-core -p indra-models -p indra-telemetry --all-features --lib --test exports --test quiet_filter
cargo test --locked --manifest-path runtime/Cargo.toml -p indra-core -p indra-telemetry --all-features --test allocations
cargo fmt --manifest-path runtime/Cargo.toml --all -- --check
DOCS_RS=1 cargo clippy --locked --manifest-path runtime/Cargo.toml \
  -p indra-core -p indra-models -p indra-telemetry -p indra-engine -p indra-server -p indra-gateway \
  --all-targets --all-features -- -D warnings
```

`DOCS_RS=1` type-checks the native crates against documentation bindings; it never links or runs OCR. The gateway and an engine test target use Linux-only APIs, so that clippy command and the gateway tests need a Linux host; CI also runs `cargo test --locked --manifest-path runtime/Cargo.toml -p indra-gateway --all-features --test process_lifecycle`, the gateway library tests and its examples. The allocation harnesses run with `cargo test` because they replace the libtest harness. The repository forbids `dyn`, growable `Vec`, `.clone()`, `.to_owned()` and `anyhow` in production sources; both workflows grep for them.

## Where Python is allowed

The serving path has no Python. Python is allowed only in the pinned Python reference arm (`backend/`, the recorded parity harnesses under `runtime/crates/indra-engine/tests/evidence/*/recorded-harness/`, the language-model fixture generator `runtime/crates/indra-engine/tests/data/lm/generate.py` and the parity replay in `evaluation/parity-replay/`), in Quarto/Seaborn reporting (`evaluation/report/`), in the independent arithmetic replay (`evaluation/independent/`) and in the existing build, CI and admission tooling: `evaluation/build-cost/`, the scripts named in the lint step (the `scripts/admit-*.py` admissions are read-only), and the CI tests `tests/test_continuous_admission.py` (of the continuous-pilot admission) and `tests/test_trace_helper_context.py` (of `scripts/prepare-trace-helper.sh`). The two scripts in `evaluation/audits/main-metadata-recovery-20261019/offline-verification/` are hash-bound records of that audit. CI's lint job rejects any other tracked `*.py`, `*.ipynb` or `*.qmd` file and any Python base image under `docker/`; only `backend/Dockerfile` builds a Python image. New tooling is Rust or POSIX shell.

## Native service loop

On a fresh host, first build the four images listed under [Local Docker](../README.md#local-docker): Compose runs `indra-native:dev` and `tika-indra:dev`, and the Tika image copies the bridge and the gateway from `indra-bridge:dev` and `indra-gateway:dev`. After that, an OCR service or engine change needs only a new native image and a restart:

```sh
docker buildx build --load --platform linux/amd64 -f docker/native.Dockerfile --target server -t indra-native:dev .
docker compose up -d --no-build --wait --wait-timeout 600
```

A bridge or gateway change needs its own image rebuilt, then the Tika image. The native build compiles the pinned GNU toolchain, ONNX Runtime and OpenCV archives once (cached layer), then the service and its acceptance gate executables; CI runs those gates from the `server-acceptance` target under 1 and 2 CPU limits ([engine guide](native-engine.md), [service guide](runtime-service.md#local-development), [deployment guide](container-deployment.md)).

## Measure a change

```sh
cargo bench --locked --manifest-path runtime/Cargo.toml -p indra-core --bench control_path
cargo bench --locked --manifest-path runtime/Cargo.toml -p indra-telemetry --all-features --bench propagation
cargo bench --locked --bench streaming
```

Criterion timings use the declared profile. `control_path` runs on mimalloc, as do the models and engine benches except the experiment-only `math_compaction`; `propagation` and `streaming` use the system allocator. The standalone allocation programs count all six allocator counters inside the measured region only. The release profile is `opt-level = 3`, thin LTO, one codegen unit, checked arithmetic and aborting panics; `maxperf` (fat LTO) and `profiling` (symbols kept) are comparison profiles, and published images never use `target-cpu=native`. Whole-stack measurements must put Tika, OCR and the load generator in one verified 1/2-CPU, 3 GiB parent cgroup; that controller, the profiling pipeline and the build-cost experiments are delivered by the later reviews.

## Review order

The reviews are stacked pull requests; each is based on the row above it, and #80 targets `main`.

| Pull request | Review | Scope |
|---|---|---|
| #80 | Runtime and Tika integration | Native OCR, bounded service lifecycle, model cache, configuration/API, telemetry, Tika gateway and trace overlay, Creusot batch-plan proof, static GNU scratch/distroless images with their distribution materials, signed image release and local/Kubernetes setup |
| #84 | Indra OCR 1.1 | Four-script routing, n-gram language-model beam search, right-to-left and vertical pages, re-frozen parity |
| #81 | Native build reuse | Opt-in signed native SDK and Cargo Chef reuse, native-input admission, fresh-runner proof and build-cost evidence |
| #82 | Evaluation and measured choices | Frozen corpus/protocols, replay, profiling, comparative study and Quarto/Seaborn reporting |
| #83 | Delivery cleanup | Removes the dead Python baseline harness, moves baseline-era records to the history index, adds a documentation index and Dependabot updates for the three Cargo workspaces |

Each review keeps its acceptance checks with the implementation. The [history index](history.md) preserves the former PR chronology and exact historical packets; the [results ledger](results.md) distinguishes accepted diagnostics, failed attempts and pending release/study work.
