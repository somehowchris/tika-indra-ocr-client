#!/usr/bin/env bash
# Lock local relink materials BEFORE timing. This is a caller-supplied receipt,
# not an upstream signature or proof of a fresh native archive compilation.
set -euo pipefail
[[ $# = 6 ]] || { echo "usage: $0 SOURCE_COMMIT EXPORTED_ELF NATIVE_RELINK_DIRECTORY MODELS BUILD_LOG NEW_RECEIPT_DIRECTORY" >&2; exit 2; }
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
commit=$1
elf=$(realpath -e -- "$2")
native=$(realpath -e -- "$3")
models=$(realpath -e -- "$4")
build_log=$(realpath -e -- "$5")
receipt=$(realpath -m -- "$6")
[[ $commit =~ ^[0-9a-f]{40}$ && ! -e $receipt && ! -L $receipt ]]
[[ $(git -C "$root" rev-parse --verify "$commit^{commit}") = "$commit" ]]
mkdir -p -- "$(dirname "$receipt")"
mkdir -m 0700 -- "$receipt"
for pair in 'runtime/validation/math-compaction-protocol.json:protocol.json' 'runtime/Cargo.lock:Cargo.lock' 'scripts/native-build-tools.lock:native-build-tools.lock' 'backend/models.lock.json:models.lock.json'; do
    git -C "$root" show "$commit:${pair%:*}" > "$receipt/${pair#*:}"
done
cp "$native/native-evidence/static-archives.sha256" "$receipt/native-static-archives.sha256"
cp "$native/glibc-archives.txt" "$receipt/glibc-archives.txt"
cp "$native/build-identity.txt" "$receipt/native-build-identity.txt"
cp "$build_log" "$receipt/build.log"
tar -xOf "$native/native-link-inputs.tar.gz" native/cargo-env.sh > "$receipt/cargo-env.sh"
sha256sum "$native/native-link-inputs.tar.gz" | awk '{print $1 "  native-link-inputs.tar.gz"}' > "$receipt/native-link-inputs.sha256"
git -C "$root" archive --format=tar "$commit" Cargo.toml Cargo.lock rust-toolchain.toml src tests benches shared runtime backend/models.lock.json scripts docker licenses LICENSE THIRD_PARTY_NOTICES.md > "$receipt/source.tar"
jq -er '.files|sort_by(.name)[]|select(.name|test("^[A-Za-z0-9._-]+$"))|"\(.sha256)  \(.name)"' "$receipt/models.lock.json" > "$receipt/models.sha256"
[[ $(wc -l < "$receipt/models.sha256") = 14 ]]
(cd "$models" && sha256sum --check "$receipt/models.sha256") > "$receipt/models-verification.log"
elf_sha=$(sha256sum "$elf" | awk '{print $1}')
files=(protocol.json Cargo.lock native-build-tools.lock native-static-archives.sha256 glibc-archives.txt native-build-identity.txt cargo-env.sh native-link-inputs.sha256 models.lock.json models.sha256 models-verification.log source.tar build.log)
(cd "$receipt" && sha256sum "${files[@]}") > "$receipt/files.sha256"
jq -Rn --arg source "$commit" --arg elf "$elf_sha" '
  [inputs|capture("^(?<sha256>[0-9a-f]{64})  (?<name>[A-Za-z0-9._-]+)$")]|map({key:.name,value:.sha256})|from_entries|
  {schema_version:1,kind:"caller_locked_local_relink_receipt",source_commit:$source,elf_sha256:$elf,files:.}
' < "$receipt/files.sha256" > "$receipt/receipt.json"
sha256sum "$receipt/receipt.json"
