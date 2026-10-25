#!/usr/bin/env bash
# Separate optional build inputs, after the unchanged six-image release gates.
set -euo pipefail
export LC_ALL=C
[[ $# = 2 ]] || { echo 'usage: publish-native-build-inputs.sh RELEASE_EVIDENCE NEW_BUILD_INPUT_EVIDENCE' >&2; exit 2; }
# Without a seed the baselines are source builds that reuse the native layer the
# source-built release gates left on this builder. Gates built from an admitted
# SDK/Chef consumer leave none, and a cold native compile overruns its deadline.
if [[ -z ${PROOF_SDK_SEED:-} ]] && ! jq -e '.mode=="source"' "${NATIVE_INPUT_ADMISSION:?}/source.json" >/dev/null 2>&1; then
    echo 'run_sdk_proof without proof_sdk_seed needs source-built release gates; dispatch with native_input_mode=source' >&2
    exit 1
fi
release=$(realpath -e -- "$1")
output=$(realpath -m -- "$2")
[[ ! -e $output && ! -L $output ]]
[[ ${GITHUB_SHA:-} =~ ^[a-f0-9]{40}$ && ${GITHUB_RUN_ID:-} =~ ^[0-9]+$ && ${GITHUB_RUN_ATTEMPT:-} =~ ^[0-9]+$ ]]
[[ ${GITHUB_REPOSITORY:-} =~ ^[A-Za-z0-9._-]+/[A-Za-z0-9._-]+$ ]]
[[ ${GITHUB_REF:-} = refs/tags/v* && ${GITHUB_WORKFLOW_REF:-} = "$GITHUB_REPOSITORY/.github/workflows/release.yml@$GITHUB_REF" ]]
[[ $(git rev-parse HEAD) = "$GITHUB_SHA" ]]
git diff --quiet HEAD --
jq -e '.passed==true and .images==6' "$release/signing/result.json" >/dev/null
jq -e --arg sha "$GITHUB_SHA" --arg ref "$GITHUB_REF" --arg run "$GITHUB_RUN_ID" --arg attempt "$GITHUB_RUN_ATTEMPT" '
 .source_sha==$sha and .source_ref==$ref and .release_run_id==$run and .release_run_attempt==$attempt and
 ([.images[].role]|sort)==["backend","bridge","gateway","native","tesseract","tika"]' "$release/signing/candidates.json" >/dev/null
mkdir -m 0700 -- "$output"
cp -- "$release/signing/candidates.json" "$output/production-candidates.json"
cp -- "$release/signing/candidates.sigstore.json" "$output/production-candidates.sigstore.json"
step() {
    local name=$1 deadline=$2; shift 2
    printf '%q ' "$@" > "$output/$name.command.txt"; printf '\n' >> "$output/$name.command.txt"
    local status=0 file_limit_kib=1048576 evidence size
    # Syft caches uncompressed image layers, independently of its small report.
    if [[ $name = sdk-sbom || $name = chef-sbom ]]; then file_limit_kib=8388608; fi
    (ulimit -f "$file_limit_kib"; python3 evaluation/build-cost/audit.py record-phase --phase "$name" \
      --output "$output/$name.phase" -- timeout --signal=TERM --kill-after=15s "$deadline" "$@") < /dev/null || status=$?
    cp -- "$output/$name.phase/stdout" "$output/$name.stdout"
    cp -- "$output/$name.phase/stderr" "$output/$name.stderr"
    if [[ $name = sdk-sbom || $name = chef-sbom ]] && (( status == 0 )); then
        for evidence in "$output/$name.stdout" "$output/$name.stderr" "$output/${name%-sbom}.spdx.json"; do
            if [[ ! -f $evidence || -L $evidence ]] || ! size=$(stat --format=%s "$evidence") \
                || [[ ! $size =~ ^[0-9]+$ ]] || (( size > 1073741824 )); then
                printf 'Missing or oversized SBOM evidence: %s\n' "$evidence" >&2
                status=2
            fi
        done
    fi
    printf '%s\n' "$status" > "$output/$name.exit"
    if (( status != 0 )); then tail -c 8192 "$output/$name.stderr" >&2; return "$status"; fi
}
baseline_recipe=docker/native.Dockerfile
baseline_inputs=(--build-arg 'BUILD_CPUSET=0,1')
sdk_inputs=(--build-arg 'BUILD_CPUSET=0,1')
python3 scripts/native-sdk.py render-producer --source docker/native.Dockerfile --output "$output/sdk-source.Dockerfile"
if [[ -n ${PROOF_SDK_SEED:-} ]]; then
    # The release's current-source production gates already used this exact
    # admitted consumer. Never cold-build a different baseline behind it.
    [[ -n ${NATIVE_INPUT_ADMISSION:-} ]]
    python3 scripts/admit-native-inputs.py --proof-sdk-seed "$PROOF_SDK_SEED" --consumer-mode sdk \
      --prepare-producer-from-admission "$NATIVE_INPUT_ADMISSION" --output "$output/parent-sdk"
    baseline_recipe=$(jq -er .dockerfile "$output/parent-sdk/selection.json")
    baseline_inputs=()
    while IFS= read -r context; do baseline_inputs+=(--build-context "$context"); done < <(jq -er '.build_contexts[]' "$output/parent-sdk/selection.json")
    while IFS= read -r argument; do baseline_inputs+=(--build-arg "$argument"); done < <(jq -er '.build_args[]' "$output/parent-sdk/selection.json")
    sdk_inputs=("${baseline_inputs[@]}")
    # Package current producer.json from the verified parent closure. Rendering
    # the normal producer first preserves the exact existing SDK packager and
    # recipe identity; the same unmodified consumer renderer removes compilation.
    python3 scripts/native-sdk.py render-consumer --source "$output/sdk-source.Dockerfile" --output "$output/sdk.Dockerfile"
else
    mv -- "$output/sdk-source.Dockerfile" "$output/sdk.Dockerfile"
fi
# The released server bakes the commit in; the baseline must build the same bytes.
baseline_inputs+=(--build-arg "SOURCE_REVISION=$GITHUB_SHA")
python3 scripts/render-chef-consumer.py --source docker/native.Dockerfile --mode producer --output "$output/chef.Dockerfile"
# Same default builder already completed all six baseline release gates. No new
# privileged builder or cache writer is created. These are correctness builds.
step builder-before 30s docker buildx inspect
for target in engine-acceptance server-acceptance; do
    step "baseline-$target" 3600s docker buildx build --platform linux/amd64 --file "$baseline_recipe" \
      --target "$target" "${baseline_inputs[@]}" --provenance=false --sbom=false \
      --metadata-file "$output/baseline-$target.metadata.json" --output "type=local,dest=$output/baseline/$target" .
done
step sdk-build 3600s docker buildx build --platform linux/amd64 --file "$output/sdk.Dockerfile" --target native-sdk \
    --build-arg "SOURCE_COMMIT=$GITHUB_SHA" "${sdk_inputs[@]}" --provenance=false --sbom=false \
    --metadata-file "$output/sdk.metadata.json" --output "type=oci,dest=$output/sdk-oci,tar=false,compression=gzip" .
sdk_digest=$(jq -er '."containerimage.digest"' "$output/sdk.metadata.json")
step sdk-materialize 600s python3 scripts/native-sdk.py materialize-oci --layout "$output/sdk-oci" --digest "$sdk_digest" --inputs "$PWD" --output "$output/sdk-materialized"
sdk_recipe=$(jq -er .recipe_sha256 "$output/sdk-materialized/materialization.json")
sdk_content=$(jq -er .content_sha256 "$output/sdk-materialized/materialization.json")
sdk_digest=$(jq -er .platform_manifest_digest "$output/sdk-materialized/materialization.json")
common=(--platform linux/amd64 --file "$output/chef.Dockerfile" --build-context "sdk-input=oci-layout://$output/sdk-oci@$sdk_digest"
    --build-arg "NATIVE_SDK_RECIPE_SHA256=$sdk_recipe" --build-arg "NATIVE_SDK_CONTENT_SHA256=$sdk_content"
    --build-arg 'BUILD_CPUSET=0,1' --provenance=false --sbom=false)
step chef-cook-export 3600s docker buildx build "${common[@]}" --target chef-cache --metadata-file "$output/chef.metadata.json" \
    --output "type=oci,dest=$output/chef-oci,tar=false,compression=gzip" .
step chef-metadata-export 300s docker buildx build "${common[@]}" --target chef-cache-metadata --output "type=local,dest=$output/chef-manifests" .
chef_digest=$(jq -er '."containerimage.digest"' "$output/chef.metadata.json")
step chef-oci-integrity 600s python3 scripts/native-sdk.py verify-oci --layout "$output/chef-oci" --digest "$chef_digest" --output "$output/chef-oci-integrity.json"
chef_digest=$(jq -er .platform_manifest_digest "$output/chef-oci-integrity.json")
# Crane0.22.1 reads a directory as an OCI layout and preserves its manifest.
# Refs include source/run/attempt. No latest alias or original image schema moves.
prefix="ghcr.io/${GITHUB_REPOSITORY_OWNER,,}/tika-indra"
locator="sha-$GITHUB_SHA-run$GITHUB_RUN_ID-$GITHUB_RUN_ATTEMPT"
for role in sdk chef; do
    expected=$sdk_digest; [[ $role = sdk ]] || expected=$chef_digest
    step "$role-push" 900s crane push "$output/$role-oci" "$prefix-native-$role:$locator"
    reference=$(cat "$output/$role-push.stdout")
    [[ $reference = "$prefix-native-$role@$expected" ]]
    printf '%s\n' "$reference" > "$output/$role.reference"
    step "$role-registry-digest" 180s crane digest "$reference"
    [[ $(cat "$output/$role-registry-digest.stdout") = "$expected" ]]
done
for role in sdk chef; do
    reference=$(cat "$output/$role.reference")
    step "$role-sbom" 600s syft "registry:$reference" -o "spdx-json=$output/$role.spdx.json"
    jq -e '.spdxVersion|startswith("SPDX-")' "$output/$role.spdx.json" >/dev/null
done
python3 scripts/native-build-index.py create --source "$PWD" --release "$release/signing/candidates.json" --evidence "$output"
identity="https://github.com/$GITHUB_WORKFLOW_REF"
verify=(--certificate-identity "$identity" --certificate-oidc-issuer https://token.actions.githubusercontent.com
    --certificate-github-workflow-repository "$GITHUB_REPOSITORY" --certificate-github-workflow-ref "$GITHUB_REF" --certificate-github-workflow-sha "$GITHUB_SHA")
for role in sdk chef; do
    reference=$(cat "$output/$role.reference")
    step "$role-sign" 180s cosign sign --yes --bundle "$output/$role.sigstore.json" "$reference"
    step "$role-attest-sbom" 180s cosign attest --yes --type spdxjson --bundle "$output/$role-sbom.sigstore.json" --predicate "$output/$role.spdx.json" "$reference"
    step "$role-attest-inputs" 180s cosign attest --yes --type https://github.com/somehowchris/tika-indra-ocr-client/attestations/native-build-inputs/v1 \
        --bundle "$output/$role-inputs.sigstore.json" --predicate "$output/index.json" "$reference"
    step "$role-verify" 180s cosign verify "${verify[@]}" "$reference"
    for kind in spdxjson https://github.com/somehowchris/tika-indra-ocr-client/attestations/native-build-inputs/v1; do
        name=sbom; expected="$output/$role.spdx.json"
        if [[ $kind != spdxjson ]]; then name=inputs; expected="$output/index.json"; fi
        step "$role-verify-$name" 180s cosign verify-attestation "${verify[@]}" --type "$kind" "$reference"
        jq -se --slurpfile expected "$expected" --arg digest "${reference##*@sha256:}" '
          any(.[]; .payloadType=="application/vnd.in-toto+json" and
            (.payload|@base64d|fromjson|.predicate==$expected[0] and any(.subject[];.digest.sha256==$digest)))' \
          "$output/$role-verify-$name.stdout" >/dev/null
    done
done
step index-sign 180s cosign sign-blob --yes --bundle "$output/index.sigstore.json" "$output/index.json"
step index-verify 180s cosign verify-blob --bundle "$output/index.sigstore.json" "${verify[@]}" "$output/index.json"
jq -n '{schema_version:1,passed:true,production_candidate_images:6,build_input_roles:["sdk","chef"],fresh_runner_proof:false,performance_claim:false}' > "$output/result.json"

python3 scripts/native-build-index.py packet --evidence "$output" --output "$output/packet"

python3 scripts/native-build-index.py archive-packet --packet "$output/packet" --output "$output/transport"
