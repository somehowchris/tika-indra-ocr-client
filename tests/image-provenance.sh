#!/usr/bin/env bash
# Synthetic OCI transport fixtures test descriptor/predicate admission, not a
# real registry or build. Hosted release replays the same checks on raw blobs.
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT
mkdir "$temporary/bin"
export GITHUB_REPOSITORY=example/repo GITHUB_SHA=0123456789012345678901234567890123456789
cat > "$temporary/bin/crane" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
[[ $# = 2 && ( $1 = manifest || $1 = blob ) && $2 =~ ^ghcr\.io/example/native@sha256:[a-f0-9]{64}$ ]]
if IFS= read -r unexpected; then exit 92; fi
printf '%s\t%s\n' "$1" "$2" >> "$PROVENANCE_TEST_FIXTURE/calls.tsv"
cat "$PROVENANCE_TEST_FIXTURE/blobs/${2##*@sha256:}"
SH
chmod +x "$temporary/bin/crane"
export PATH="$temporary/bin:$PATH"

blob() {
  digest=$(sha256sum "$1" | cut -d' ' -f1)
  cp -- "$1" "$PROVENANCE_TEST_FIXTURE/blobs/$digest"
}

run_case() {
  local name=$1 version=$2 expected=$3 mutation=$4 status=0
  export PROVENANCE_TEST_FIXTURE="$temporary/$name"
  mkdir -p "$PROVENANCE_TEST_FIXTURE/blobs"
  printf '{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json","config":{"digest":"sha256:%064d"},"layers":[]}\n' 0 > "$PROVENANCE_TEST_FIXTURE/platform.json"
  blob "$PROVENANCE_TEST_FIXTURE/platform.json"
  platform=$digest
  platform_size=$(stat --format=%s "$PROVENANCE_TEST_FIXTURE/platform.json")
  jq -n --arg version "$version" --arg sha "$GITHUB_SHA" --arg digest "$platform" '
    {"vcs":{"revision":$sha,"source":"https://github.com/example/repo","localdir:dockerfile":"docker"},
     "request":{"frontend":"dockerfile.v0","args":{"target":"server",
       "label:org.opencontainers.image.revision":$sha,"label:org.opencontainers.image.source":"https://github.com/example/repo"}},
     "graph":[{"id":"step0"}]} as $p |
    {_type:"https://in-toto.io/Statement/v1",subject:[{name:"pkg:docker/example/native",digest:{sha256:$digest}}],
     predicateType:("https://slsa.dev/provenance/"+$version),predicate:
      (if $version == "v1" then
        {buildDefinition:{buildType:"https://github.com/moby/buildkit/blob/master/docs/attestations/slsa-definitions.md",
          externalParameters:{configSource:{path:"native.Dockerfile"},request:$p.request},
          internalParameters:{buildConfig:{llbDefinition:$p.graph}}},
         runDetails:{metadata:{buildkit_metadata:{vcs:$p.vcs},buildkit_completeness:{request:true}}}}
       else
        {buildType:"https://mobyproject.org/buildkit@v1",invocation:{parameters:$p.request,configSource:{entryPoint:"native.Dockerfile"}},
         buildConfig:{llbDefinition:$p.graph},metadata:{"https://mobyproject.org/buildkit@v1#metadata":{vcs:$p.vcs},completeness:{parameters:true}}}
       end)}
  ' > "$PROVENANCE_TEST_FIXTURE/base-statement.json"
  jq "$mutation" "$PROVENANCE_TEST_FIXTURE/base-statement.json" > "$PROVENANCE_TEST_FIXTURE/statement.json"
  blob "$PROVENANCE_TEST_FIXTURE/statement.json"
  statement=$digest
  statement_size=$(stat --format=%s "$PROVENANCE_TEST_FIXTURE/statement.json")
  if [[ "$name" = wrong-size ]]; then statement_size=$((statement_size + 1)); fi
  if [[ "$name" = wrong-hash ]]; then
    sed 's/dockerfile.v0/dockerfile.v1/' "$PROVENANCE_TEST_FIXTURE/statement.json" > "$PROVENANCE_TEST_FIXTURE/blobs/$statement"
  fi
  jq -n --arg digest "$statement" --argjson bytes "$statement_size" --arg version "$version" '
    {schemaVersion:2,mediaType:"application/vnd.oci.image.manifest.v1+json",
     layers:[{mediaType:"application/vnd.in-toto+json",digest:("sha256:"+$digest),size:$bytes,
       annotations:{"in-toto.io/predicate-type":("https://slsa.dev/provenance/"+$version)}}]}
  ' > "$PROVENANCE_TEST_FIXTURE/attestation.json"
  blob "$PROVENANCE_TEST_FIXTURE/attestation.json"
  attestation=$digest
  attestation_size=$(stat --format=%s "$PROVENANCE_TEST_FIXTURE/attestation.json")
  subject=$platform
  [[ "$name" != wrong-link ]] || subject=$(printf '%064d' 0)
  architecture=amd64
  [[ "$name" != wrong-platform ]] || architecture=arm64
  jq -n --arg platform "$platform" --arg subject "$subject" --arg attestation "$attestation" \
    --argjson platform_size "$platform_size" --argjson attestation_size "$attestation_size" --arg architecture "$architecture" '
    {schemaVersion:2,mediaType:"application/vnd.oci.image.index.v1+json",manifests:[
      {digest:("sha256:"+$platform),size:$platform_size,platform:{os:"linux",architecture:$architecture}},
      {digest:("sha256:"+$attestation),size:$attestation_size,platform:{os:"unknown",architecture:"unknown"},
       annotations:{"vnd.docker.reference.type":"attestation-manifest","vnd.docker.reference.digest":("sha256:"+$subject)}}]}
  ' > "$PROVENANCE_TEST_FIXTURE/index.json"
  blob "$PROVENANCE_TEST_FIXTURE/index.json"
  (cd "$root"; bash scripts/verify-image-provenance.sh "ghcr.io/example/native@sha256:$digest" docker/native.Dockerfile server \
    "$PROVENANCE_TEST_FIXTURE/output") > "$PROVENANCE_TEST_FIXTURE/stdout" 2> "$PROVENANCE_TEST_FIXTURE/stderr" || status=$?
  if [[ "$expected" = pass ]]; then
    [[ "$status" = 0 && $(wc -l < "$PROVENANCE_TEST_FIXTURE/calls.tsv") = 4 ]]
    jq -e '.passed and .predicate_type == ("https://slsa.dev/provenance/"+$version)' --arg version "$version" "$PROVENANCE_TEST_FIXTURE/output/result.json" > /dev/null
    # Completed evidence is immutable within this invocation.
    status=0
    (cd "$root"; bash scripts/verify-image-provenance.sh "ghcr.io/example/native@sha256:$digest" docker/native.Dockerfile server \
      "$PROVENANCE_TEST_FIXTURE/output") > /dev/null 2>&1 || status=$?
    [[ "$status" != 0 ]]
  else
    [[ "$status" != 0 && ! -e "$PROVENANCE_TEST_FIXTURE/output/result.json" ]]
  fi
}

run_case v1 v1 pass '.'
run_case v02 v0.2 pass '.'
run_case wrong-subject v1 fail '.subject[0].digest.sha256 = ("1"*64)'
run_case wrong-revision v1 fail '.predicate.runDetails.metadata.buildkit_metadata.vcs.revision = ("1"*40)'
run_case wrong-source v1 fail '.predicate.runDetails.metadata.buildkit_metadata.vcs.source = "https://github.com/wrong/repo"'
run_case wrong-target v1 fail '.predicate.buildDefinition.externalParameters.request.args.target = "engine-acceptance"'
run_case wrong-dockerfile v1 fail '.predicate.buildDefinition.externalParameters.configSource.path = "other.Dockerfile"'
run_case minimal-mode v1 fail 'del(.predicate.buildDefinition.internalParameters.buildConfig)'
run_case incomplete-parameters v1 fail '.predicate.runDetails.metadata.buildkit_completeness.request = false'
run_case wrong-size v1 fail '.'
grep -Fq 'size mismatch' "$temporary/wrong-size/stderr"
run_case wrong-hash v1 fail '.'
grep -Fq 'SHA-256 mismatch' "$temporary/wrong-hash/stderr"
run_case wrong-link v1 fail '.'
run_case wrong-platform v1 fail '.'
run_case wrong-type v1 fail '.predicateType = "https://example.invalid/not-slsa"'
printf 'OCI provenance admission: 14 cases passed; raw SHA/size, descriptor links, platform, subject, source, Dockerfile, target, max-mode and overwrite checks. No real registry/signature claim.\n'
