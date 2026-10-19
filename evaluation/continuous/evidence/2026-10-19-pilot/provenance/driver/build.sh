set -euo pipefail
source /native/cargo-env.sh
mkdir /tmp/continuous-export
export CARGO_TARGET_DIR=/target/continuous-85fef71
cargo build --locked --profile profiling --manifest-path runtime/Cargo.toml --target x86_64-unknown-linux-gnu -p indra-engine --example continuous_experiment -p indra-gateway --example continuous_http --message-format=json-render-diagnostics > /tmp/continuous-export/cargo-artifacts.jsonl
rustc -Vv > /tmp/continuous-export/rustc.txt
sha256sum "$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/profiling/examples/continuous_experiment" "$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/profiling/examples/continuous_http" > /tmp/continuous-export/executables.sha256
cp "$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/profiling/examples/continuous_experiment" "$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/profiling/examples/continuous_http" /tmp/continuous-export/
