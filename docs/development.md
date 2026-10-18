# Local Rust development

Install the pinned GNU Rust toolchain from `rust-toolchain.toml`, a GNU C/C++ compiler and Git. The measured Tika CLI adapter stays in the root Cargo package; the persistent Rust service has its own workspace under `runtime/`. This keeps ONNX, HTTP and telemetry dependency resolution out of the adapter's offline scratch build and glibc relink bundle.

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

The root [Docker quickstart](../README.md#local-docker) remains the executable reference stack while the dependent Rust PRs add real inference, the HTTP service and replacement images. The exact baseline source, images and measurements remain distinguishable from the new runtime.

## Review order

1. **Bounded core:** establish admission, drain and batch-memory contracts and measure their cost.
2. **Model acquisition and Rust inference:** verify cached content and match the pinned Python pipeline stage by stage through the Tika text-output contract.
3. **HTTP service and configuration:** connect real inference to Axum, typed configuration, startup/readiness/liveness probes, documented errors and bounded shutdown.
4. **Operational telemetry:** correlate request, queue, worker and model spans; verify OTLP and Sentry output, redaction and shutdown flushing.
5. **Scheduling and packaging:** evaluate continuous batching and overload behavior; build and sign the GNU/distroless runtime images.
6. **Comparative evidence:** run the frozen multilingual corpus, allocation/profile experiments and one/two-CPU comparisons; render and inspect the Quarto/Seaborn report.

Each PR is validated against its own scope. The core alone is not an OCR server. Aligned tensor accounting is a resource guard; upstream's unaligned grouping proxy remains a separately identified reference policy until differential tests justify a different default.
