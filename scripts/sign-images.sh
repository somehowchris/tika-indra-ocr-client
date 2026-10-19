#!/usr/bin/env bash
# Preserve cf15fa2's deadlines, EOF stdin, complete per-command evidence and
# fail-closed writes. Only read-only verification retries (at most three times).
# CLI contract: https://github.com/sigstore/cosign/tree/v3.1.3/doc
set -euo pipefail
export LC_ALL=C
fail() { printf '%s\n' "$*" >&2; exit 2; }
[[ $# = 3 ]] || fail 'usage: sign-images.sh CANDIDATES_JSON NEW_EVIDENCE_DIRECTORY RELEASE_PREDICATE_JSON'
index=$1
evidence=$2
predicate=$3
[[ -s "$index" && -f "$index" && ! -L "$index" && $(stat --format=%s "$index") -le 65536 ]] || fail 'candidate index must be a bounded regular file'
[[ -s "$predicate" && -f "$predicate" && ! -L "$predicate" && $(stat --format=%s "$predicate") -le 65536 ]] || fail 'release predicate must be a bounded regular file'
[[ ! -e "$evidence" ]] || fail 'refusing to overwrite signing evidence'
[[ ${GITHUB_SHA:-} =~ ^[a-f0-9]{40}$ && ${GITHUB_RUN_ID:-} =~ ^[0-9]+$ && ${GITHUB_RUN_ATTEMPT:-} =~ ^[0-9]+$ ]] || fail 'exact GitHub invocation identity is required'
[[ ${GITHUB_REPOSITORY:-} =~ ^[A-Za-z0-9._-]+/[A-Za-z0-9._-]+$ ]] || fail 'invalid repository identity'
[[ ${GITHUB_WORKFLOW_REF:-} = "$GITHUB_REPOSITORY/.github/workflows/release.yml@${GITHUB_REF:-}" ]] || fail 'unexpected signing workflow identity'
identity="https://github.com/$GITHUB_WORKFLOW_REF"
issuer=https://token.actions.githubusercontent.com
predicate_type=https://github.com/somehowchris/tika-indra-ocr-client/attestations/release-evidence/v1
image_prefix="ghcr.io/${GITHUB_REPOSITORY_OWNER,,}/tika-indra"

jq -e --arg repository "$GITHUB_REPOSITORY" --arg sha "$GITHUB_SHA" --arg ref "$GITHUB_REF" \
  --arg run "$GITHUB_RUN_ID" --arg attempt "$GITHUB_RUN_ATTEMPT" --arg workflow "$GITHUB_WORKFLOW_REF" \
  --arg identity "$identity" --arg issuer "$issuer" --arg prefix "$image_prefix" '
  .schema_version == 1 and .repository == $repository and .source_sha == $sha and .source_ref == $ref and
  .release_run_id == $run and .release_run_attempt == $attempt and .workflow_ref == $workflow and
  .workflow_identity == $identity and .oidc_issuer == $issuer and
  (.images | length == 6) and
  ([.images[].role] | sort == ["backend","bridge","gateway","native","tesseract","tika"]) and
  all(.images[]; (.digest | test("^sha256:[a-f0-9]{64}$")) and
    .reference == ($prefix + "-" + .role + "@" + .digest))
' "$index" > /dev/null || fail 'candidate identity or exact six-image set differs from this invocation'
jq -e --arg sha "$GITHUB_SHA" --arg run "$GITHUB_RUN_ID" --arg attempt "$GITHUB_RUN_ATTEMPT" '
  .schema_version == 1 and .source_sha == $sha and .release_run_id == $run and .release_run_attempt == $attempt and
  (.evidence_manifest.sha256 | test("^[a-f0-9]{64}$")) and
  .evidence_manifest.bytes > 0 and .verification.passed == true
' "$predicate" > /dev/null || fail 'release predicate does not attest this tested invocation'

mkdir -p "$evidence"
cp -- "$index" "$evidence/candidates.json"
cp -- "$predicate" "$evidence/release-predicate.json"
jq -r '.images[] | [.role,.reference] | @tsv' "$index" > "$evidence/images.tsv"
verify_identity=(--certificate-identity "$identity" --certificate-oidc-issuer "$issuer"
  --certificate-github-workflow-repository "$GITHUB_REPOSITORY"
  --certificate-github-workflow-ref "$GITHUB_REF" --certificate-github-workflow-sha "$GITHUB_SHA")

# The successful attempt's stdout path is exposed to the predicate comparison.
# A file-size limit fails the command instead of quietly truncating its output.
run_step() {
  local stage=$1 deadline=$2 attempts=$3 attempt status started elapsed
  shift 3
  for ((attempt=1; attempt<=attempts; attempt++)); do
    stage_stdout="$image_dir/$stage.attempt-$attempt.stdout"
    started=$SECONDS
    status=0
    (ulimit -f 131072; exec timeout --signal=TERM --kill-after=10s "$deadline" "$@") \
      < /dev/null > "$stage_stdout" 2> "$image_dir/$stage.attempt-$attempt.stderr" || status=$?
    elapsed=$((SECONDS - started))
    printf '%s\t%s\t%s\t%s\n' "$stage" "$attempt" "$status" "$elapsed" >> "$image_dir/steps.tsv"
    printf '%s: %s attempt %s/%s finished with status %s in %ss\n' "$role" "$stage" "$attempt" "$attempts" "$status" "$elapsed"
    if (( status == 0 )); then return 0; fi
    # Bytes, not lines: one huge diagnostic line must not flood the job console.
    tail -c 4096 "$image_dir/$stage.attempt-$attempt.stderr" >&2
    if (( attempt < attempts )); then sleep 2; fi
  done
  return "$status"
}

require_predicate() {
  local expected=$1 digest=${reference##*@sha256:}
  # Cosign 3.1.3 prints each verified DSSE envelope as JSON on stdout. Compare
  # the decoded predicate too: a valid older attestation is not this receipt.
  jq -se --slurpfile expected "$expected" --arg digest "$digest" '
    any(.[]; .payloadType == "application/vnd.in-toto+json" and
      (.payload | @base64d | fromjson |
        .predicate == $expected[0] and any(.subject[]; .digest.sha256 == $digest)))
  ' "$stage_stdout" > /dev/null || fail 'verified attestation does not contain the exact submitted predicate and digest'
}

while IFS=$'\t' read -r role reference; do
  image_dir="$evidence/$role"
  mkdir "$image_dir"
  printf 'stage\tattempt\texit_status\telapsed_seconds\n' > "$image_dir/steps.tsv"
  printf '%s\n' "$reference" > "$image_dir/reference.txt"
  run_step sign 180s 1 cosign sign --yes --bundle "$image_dir/image.sigstore.json" "$reference"
  run_step sbom 300s 1 syft "registry:$reference" -o "spdx-json=$image_dir/sbom.spdx.json"
  jq -e '.spdxVersion | startswith("SPDX-")' "$image_dir/sbom.spdx.json" > /dev/null
  run_step attest-sbom 180s 1 cosign attest --yes --type spdxjson \
    --bundle "$image_dir/sbom.sigstore.json" --predicate "$image_dir/sbom.spdx.json" "$reference"
  run_step attest-evidence 180s 1 cosign attest --yes --type "$predicate_type" \
    --bundle "$image_dir/evidence.sigstore.json" --predicate "$predicate" "$reference"
  run_step verify 180s 3 cosign verify "${verify_identity[@]}" "$reference"
  run_step verify-sbom 180s 3 cosign verify-attestation --type spdxjson "${verify_identity[@]}" "$reference"
  require_predicate "$image_dir/sbom.spdx.json"
  run_step verify-evidence 180s 3 cosign verify-attestation --type "$predicate_type" "${verify_identity[@]}" "$reference"
  require_predicate "$predicate"
done < "$evidence/images.tsv"

role=candidate-index
image_dir="$evidence/index"
mkdir "$image_dir"
printf 'stage\tattempt\texit_status\telapsed_seconds\n' > "$image_dir/steps.tsv"
run_step sign-blob 180s 1 cosign sign-blob --yes --bundle "$evidence/candidates.sigstore.json" "$evidence/candidates.json"
run_step verify-blob 180s 3 cosign verify-blob --bundle "$evidence/candidates.sigstore.json" \
  "${verify_identity[@]}" "$evidence/candidates.json"
jq -n '{schema_version:1,passed:true,images:6,claims:["exact-image-digest","workflow-identity","issuer","repository","source-ref","source-sha","transparency-log","submitted-spdx-predicate","submitted-release-evidence-predicate","candidate-index-blob"]}' \
  > "$evidence/result.json"
