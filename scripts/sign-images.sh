#!/usr/bin/env bash
# Sign exactly the already-tested immutable images. Preserve full output off-console.
set -euo pipefail

digest_file=${1:?pass the tested image digest file}
evidence_dir=${2:?pass the signing evidence directory}
identity=${3:?pass the exact workflow certificate identity}
mapfile -t references < "$digest_file"
if (( ${#references[@]} != 4 )); then
  printf 'Expected exactly four tested image digests\n' >&2
  exit 2
fi
declare -A seen
for reference in "${references[@]}"; do
  if [[ ! $reference =~ ^ghcr\.io/[a-z0-9._-]+/[a-z0-9._-]+@sha256:[a-f0-9]{64}$ ]]; then
    printf 'Expected an immutable GHCR image digest\n' >&2
    exit 2
  fi
  image_name=${reference##*/}
  image_name=${image_name%@*}
  if [[ -v seen[$image_name] ]]; then
    printf 'Duplicate image name would overwrite signing evidence\n' >&2
    exit 2
  fi
  seen[$image_name]=1
done
mkdir -p "$evidence_dir"
cp "$digest_file" "$evidence_dir/image-digests.txt"

run_step() {
  local stage=$1
  local deadline=$2
  shift 2
  local started=$SECONDS
  local status=0
  timeout --signal=TERM --kill-after=10s "$deadline" "$@" \
    < /dev/null > "$image_dir/$stage.stdout" 2> "$image_dir/$stage.stderr" || status=$?
  local elapsed=$((SECONDS - started))
  printf '%s\t%s\t%s\n' "$stage" "$status" "$elapsed" >> "$image_dir/steps.tsv"
  printf '%s: %s finished with status %s in %ss\n' "$image_name" "$stage" "$status" "$elapsed"
  if (( status != 0 )); then
    tail -n 25 "$image_dir/$stage.stderr" >&2
    return "$status"
  fi
}

for reference in "${references[@]}"; do
  image_name=${reference##*/}
  image_name=${image_name%@*}
  image_dir="$evidence_dir/$image_name"
  # A duplicate name would overwrite provenance and is an input error.
  mkdir "$image_dir"
  printf '%s\n' "$reference" > "$image_dir/reference.txt"
  run_step sign 180s cosign sign --yes "$reference"
  run_step sbom 300s syft "registry:$reference" -o "spdx-json=$image_dir/sbom.spdx.json"
  run_step attest 180s cosign attest --yes --type spdxjson \
    --predicate "$image_dir/sbom.spdx.json" "$reference"
  run_step verify 180s cosign verify --certificate-identity "$identity" \
    --certificate-oidc-issuer https://token.actions.githubusercontent.com "$reference"
  run_step verify-attestation 180s cosign verify-attestation --type spdxjson \
    --certificate-identity "$identity" \
    --certificate-oidc-issuer https://token.actions.githubusercontent.com "$reference"
done
