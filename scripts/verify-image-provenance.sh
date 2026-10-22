#!/usr/bin/env bash
# Verify the raw OCI descriptor chain before a release signs its index.
# crane 0.22.1 emits raw manifest/blob bytes; unlike imagetools' .Provenance
# projection this preserves the in-toto subject. No registry credential parsing.
# https://github.com/google/go-containerregistry/tree/v0.22.1/cmd/crane/cmd
# https://github.com/moby/buildkit/blob/v0.34.0/docs/attestations/slsa-definitions.md
set -euo pipefail
export LC_ALL=C
fail() { printf '%s\n' "$*" >&2; exit 1; }
[[ $# = 4 ]] || fail 'usage: verify-image-provenance.sh IMAGE_AT_DIGEST DOCKERFILE TARGET NEW_OUTPUT'
reference=$1 dockerfile=$2 target=$3 output=$4
[[ "$reference" =~ ^ghcr\.io/[a-z0-9._-]+/[a-z0-9._-]+@sha256:[a-f0-9]{64}$ ]] || fail 'immutable GHCR reference required'
[[ "$dockerfile" =~ ^(docker|backend)/[A-Za-z0-9._-]+$ && -f "$dockerfile" && ! -L "$dockerfile" ]] || fail 'regular repository Dockerfile required'
[[ "$target" =~ ^[A-Za-z0-9_-]*$ && ! -e "$output" ]] || fail 'invalid stage or existing output'
[[ ${GITHUB_SHA:-} =~ ^[a-f0-9]{40}$ && ${GITHUB_REPOSITORY:-} =~ ^[A-Za-z0-9._-]+/[A-Za-z0-9._-]+$ ]] || fail 'exact source identity required'
repository=${reference%@*}
index_digest=${reference##*@}
mkdir -p "$output"
printf 'stage\texit_status\n' > "$output/steps.tsv"

fetch() {
  local stage=$1 command=$2 digest=$3 expected_size=$4 status=0 bytes
  [[ "$digest" =~ ^sha256:[a-f0-9]{64}$ && "$expected_size" =~ ^[0-9]{1,8}$ ]] || fail 'invalid OCI descriptor'
  (( expected_size <= 16777216 )) || fail 'OCI evidence descriptor exceeds 16 MiB'
  (ulimit -f 16384; exec timeout --signal=TERM --kill-after=10s 120s crane "$command" "$repository@$digest") \
    < /dev/null > "$output/$stage.json" 2> "$output/$stage.stderr" || status=$?
  printf '%s\t%s\n' "$stage" "$status" >> "$output/steps.tsv"
  if (( status != 0 )); then tail -c 4096 "$output/$stage.stderr" >&2; fail "cannot retrieve OCI $stage"; fi
  bytes=$(stat --format=%s "$output/$stage.json")
  (( bytes > 0 && bytes <= 16777216 )) || fail 'OCI evidence length is outside its bound'
  if (( expected_size != 0 )); then [[ "$bytes" = "$expected_size" ]] || fail 'OCI descriptor size mismatch'; fi
  [[ $(sha256sum "$output/$stage.json" | cut -d' ' -f1) = "${digest#sha256:}" ]] || fail 'OCI descriptor SHA-256 mismatch'
}

fetch index manifest "$index_digest" 0
jq -e '
  .schemaVersion == 2 and .mediaType == "application/vnd.oci.image.index.v1+json" and
  (.manifests | length == 2) and
  ([.manifests[] | select(.platform.os == "linux" and .platform.architecture == "amd64")] | length == 1) and
  ([.manifests[] | select(.annotations["vnd.docker.reference.type"] == "attestation-manifest")] | length == 1)
' "$output/index.json" > /dev/null || fail 'expected one linux/amd64 image and one provenance manifest'
platform_digest=$(jq -er '.manifests[] | select(.platform.os == "linux" and .platform.architecture == "amd64") | .digest' "$output/index.json")
platform_size=$(jq -er '.manifests[] | select(.platform.os == "linux" and .platform.architecture == "amd64") | .size' "$output/index.json")
attestation_digest=$(jq -er --arg platform "$platform_digest" '.manifests[] |
  select(.annotations["vnd.docker.reference.type"] == "attestation-manifest" and
    .annotations["vnd.docker.reference.digest"] == $platform) | .digest' "$output/index.json")
attestation_size=$(jq -er '.manifests[] | select(.annotations["vnd.docker.reference.type"] == "attestation-manifest") | .size' "$output/index.json")
fetch platform manifest "$platform_digest" "$platform_size"
fetch attestation manifest "$attestation_digest" "$attestation_size"
config_digest=$(jq -er '.config.digest | select(test("^sha256:[a-f0-9]{64}$"))' "$output/platform.json")
jq -e '
  [.layers[] | select(.mediaType == "application/vnd.in-toto+json" and
    (.annotations["in-toto.io/predicate-type"] == "https://slsa.dev/provenance/v1" or
     .annotations["in-toto.io/predicate-type"] == "https://slsa.dev/provenance/v0.2"))] | length == 1
' "$output/attestation.json" > /dev/null || fail 'expected exactly one supported SLSA statement layer'
jq -e '[.layers[] | select(.mediaType == "application/vnd.in-toto+json" and
  (.annotations["in-toto.io/predicate-type"] == "https://slsa.dev/provenance/v1" or
   .annotations["in-toto.io/predicate-type"] == "https://slsa.dev/provenance/v0.2"))][0]' \
  "$output/attestation.json" > "$output/provenance-descriptor.json"
provenance_digest=$(jq -er .digest "$output/provenance-descriptor.json")
provenance_size=$(jq -er .size "$output/provenance-descriptor.json")
predicate_type=$(jq -er '.annotations["in-toto.io/predicate-type"]' "$output/provenance-descriptor.json")
fetch statement blob "$provenance_digest" "$provenance_size"
jq -e --arg digest "${platform_digest#sha256:}" --arg type "$predicate_type" \
  --arg sha "$GITHUB_SHA" --arg source "https://github.com/$GITHUB_REPOSITORY" --arg dockerfile "$dockerfile" --arg target "$target" '
  def provenance_fields:
    if .predicateType == "https://slsa.dev/provenance/v1" then
      {kind:.predicate.buildDefinition.buildType,request:.predicate.buildDefinition.externalParameters.request,
       config:.predicate.buildDefinition.externalParameters.configSource,
       graph:.predicate.buildDefinition.internalParameters.buildConfig.llbDefinition,
       vcs:.predicate.runDetails.metadata.buildkit_metadata.vcs,
       complete:.predicate.runDetails.metadata.buildkit_completeness.request}
    elif .predicateType == "https://slsa.dev/provenance/v0.2" then
      {kind:.predicate.buildType,request:.predicate.invocation.parameters,config:{path:.predicate.invocation.configSource.entryPoint},
       graph:.predicate.buildConfig.llbDefinition,vcs:.predicate.metadata["https://mobyproject.org/buildkit@v1#metadata"].vcs,
       complete:.predicate.metadata.completeness.parameters}
    else error("unsupported provenance version") end;
  .predicateType == $type and
  (._type == "https://in-toto.io/Statement/v1" or ._type == "https://in-toto.io/Statement/v0.1") and
  (.subject | length > 0) and all(.subject[]; .digest.sha256 == $digest) and
  (provenance_fields |
    (.kind == "https://github.com/moby/buildkit/blob/master/docs/attestations/slsa-definitions.md" or
     .kind == "https://mobyproject.org/buildkit@v1") and
    .request.frontend == "dockerfile.v0" and (.request.args.target // "") == $target and
    .request.args["label:org.opencontainers.image.revision"] == $sha and
    .request.args["label:org.opencontainers.image.source"] == $source and
    (.graph | type == "array" and length > 0) and .complete == true and
    .vcs.revision == $sha and (.vcs.source == $source or .vcs.source == ($source + ".git")) and
    (.vcs["localdir:dockerfile"] + "/" + .config.path) == $dockerfile)
' "$output/statement.json" > /dev/null || fail 'provenance subject, source hint, Dockerfile, target or max-mode build graph differs'
jq -n --arg reference "$reference" --arg platform "$platform_digest" --arg config "$config_digest" \
  --arg attestation "$attestation_digest" --arg provenance "$provenance_digest" --arg type "$predicate_type" '
  {schema_version:1,passed:true,reference:$reference,platform_digest:$platform,config_digest:$config,
   attestation_digest:$attestation,provenance_digest:$provenance,predicate_type:$type,
   scope:"Raw OCI sizes/SHA-256 and provenance subject/max-mode/source-metadata checks. Local-context VCS metadata is a client hint; the release separately checks the actual clean Git checkout. No SLSA level or hermetic-build claim."}
' > "$output/result.json"
