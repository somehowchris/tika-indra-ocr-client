#!/usr/bin/env bash
# Independent actual-Cargo protocol check; never invokes native OCR or emits timings.
set -euo pipefail
[[ $# = 2 ]] || { echo 'usage: test-cargo-selector-real.sh ABSOLUTE_SELECTOR NEW_OUTPUT' >&2; exit 2; }
selector=$(realpath -e -- "$1")
work=$(realpath -m -- "$2")
[[ -f $selector && ! -e $work && ! -L $work ]]
mkdir -p -- "$(dirname "$work")"
mkdir -m 0700 -- "$work"
sha256sum "$selector" > "$work/selector.before.sha256"
mkdir "$work/src" "$work/tests" "$work/examples" "$work/selected"
cat > "$work/Cargo.toml" <<'TOML'
[package]
name = "indra-server"
version = "0.2.0"
edition = "2024"
[workspace]
TOML
printf 'pub fn ready() -> bool {true}\n#[test] fn library_test(){assert!(ready());}\n' > "$work/src/lib.rs"
printf 'fn main() {assert!(indra_server::ready());}\n' > "$work/src/main.rs"
printf '#[test] fn lifecycle_native_fixture(){assert!(indra_server::ready());}\n' > "$work/tests/lifecycle_native.rs"
printf 'fn main() {assert!(indra_server::ready());}\n' > "$work/examples/ocr.rs"
manifest=$work/Cargo.toml
# Ask real Cargo for its ID; directory names can change its displayed fragment.
package=$(cargo +1.99.1 metadata --offline --no-deps --manifest-path "$manifest" --format-version 1 | jq -r '.packages[0].id')
for role in unit native examples; do
 case $role in
 unit) cargo +1.99.1 test --offline --manifest-path "$manifest" --target x86_64-unknown-linux-gnu --lib --no-run --message-format=json-render-diagnostics > "$work/$role.jsonl";;
 native) cargo +1.99.1 test --offline --manifest-path "$manifest" --target x86_64-unknown-linux-gnu --test lifecycle_native --no-run --message-format=json-render-diagnostics > "$work/$role.jsonl";;
 examples) cargo +1.99.1 build --offline --manifest-path "$manifest" --target x86_64-unknown-linux-gnu --examples --message-format=json-render-diagnostics > "$work/$role.jsonl";;
 esac
done
for record in 'unit indra_server lib true' 'native lifecycle_native test true' 'native indra-server bin false' 'examples ocr example false'; do
 read -r stream name kind harness <<< "$record"
 python3 -B "$selector" --json "$work/$stream.jsonl" --package-id "$package" \
  --manifest-path "$manifest" --name "$name" --kind "$kind" --test "$harness" \
  --artifact-root "$work/target/x86_64-unknown-linux-gnu/debug" --destination "$work/selected/$name" --receipt "$work/selected/$name.json"
 source=$(jq -er '.source' "$work/selected/$name.json")
 cmp "$source" "$work/selected/$name"
 digest=$(sha256sum "$work/selected/$name" | cut -d' ' -f1)
 jq -e --arg digest "$digest" '.binary_sha256==$digest' "$work/selected/$name.json" > /dev/null
done
sha256sum "$selector" > "$work/selector.after.sha256"
cmp "$work/selector.before.sha256" "$work/selector.after.sha256"
cargo +1.99.1 -V > "$work/cargo-version.txt"
rustc +1.99.1 -Vv > "$work/rustc-version.txt"
jq -n '{passed:true,scope:"Actual Cargo JSON role selection only; tiny offline fixture. No native OCR, static-link, production equivalence or speed claim.",roles:["lib-test","integration-test","ordinary-bin-from-integration-test","example"]}' > "$work/result.json"
printf '%s\n' "$work"
