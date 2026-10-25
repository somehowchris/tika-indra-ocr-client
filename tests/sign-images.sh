#!/usr/bin/env bash
# Orchestration regressions. Test doubles never claim to prove cryptography;
# the release uses the pinned real Cosign/Syft binaries and OIDC identity.
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT
mkdir "$temporary/bin" "$temporary/state"
export GITHUB_REPOSITORY=example/repo GITHUB_REPOSITORY_OWNER=example
export GITHUB_REF=refs/heads/stack/11-signed-release GITHUB_RUN_ID=42 GITHUB_RUN_ATTEMPT=1
export GITHUB_WORKFLOW_REF="$GITHUB_REPOSITORY/.github/workflows/release.yml@$GITHUB_REF"
export GITHUB_SHA=0123456789012345678901234567890123456789
export SIGN_TEST_CALLS="$temporary/calls" SIGN_TEST_STATE="$temporary/state"
export SIGN_TEST_REAL_TIMEOUT
SIGN_TEST_REAL_TIMEOUT=$(command -v timeout)
export SIGN_TEST_REAL_STAT
SIGN_TEST_REAL_STAT=$(command -v stat)
cat > "$temporary/bin/stat" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
if [[ ${SIGN_TEST_MODE:-} = stat-failure && ${!#} = */sbom.spdx.json ]]; then exit 73; fi
exec "$SIGN_TEST_REAL_STAT" "$@"
SH
cat > "$temporary/bin/timeout" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
[[ $1 = --signal=TERM && $2 = --kill-after=10s && ( $3 = 180s || $3 = 300s ) ]]
if [[ ${SIGN_TEST_MODE:-} = hang ]]; then
  shift 3
  exec "$SIGN_TEST_REAL_TIMEOUT" --signal=TERM --kill-after=1s 0.1s "$@"
fi
exec "$SIGN_TEST_REAL_TIMEOUT" "$@"
SH
cat > "$temporary/bin/cosign" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
[[ $(ulimit -S -f) = 131072 && $(ulimit -H -f) = 131072 ]]
command=$1
shift
reference=${!#}
type=none bundle= predicate=
identity= issuer= sha= repository= ref=
while (( $# > 1 )); do
  case $1 in
    --type) type=$2; shift 2;;
    --bundle) bundle=$2; shift 2;;
    --predicate) predicate=$2; shift 2;;
    --certificate-identity) identity=$2; shift 2;;
    --certificate-oidc-issuer) issuer=$2; shift 2;;
    --certificate-github-workflow-sha) sha=$2; shift 2;;
    --certificate-github-workflow-repository) repository=$2; shift 2;;
    --certificate-github-workflow-ref) ref=$2; shift 2;;
    --yes) shift;;
    *) exit 91;;
  esac
done
role=${reference##*/tika-indra-}
role=${role%@*}
kind=sbom
[[ $type = spdxjson ]] || kind=evidence
printf '%s\t%s\t%s\n' "$command" "$type" "$reference" >> "$SIGN_TEST_CALLS"
if IFS= read -r unexpected; then exit 92; fi
if [[ $command = verify* ]]; then
  [[ $identity = "https://github.com/$GITHUB_WORKFLOW_REF" && $issuer = https://token.actions.githubusercontent.com ]]
  [[ $sha = "$GITHUB_SHA" && $repository = "$GITHUB_REPOSITORY" && $ref = "$GITHUB_REF" ]]
fi
if [[ ${SIGN_TEST_MODE:-} = hang ]]; then sleep 10; fi
if [[ ( ${SIGN_TEST_MODE:-} = write-failure && $command = attest ) ||
      ( ${SIGN_TEST_MODE:-} = verify-failure && $command = verify ) ||
      ( ${SIGN_TEST_MODE:-} = transient && $command = verify && ! -e $SIGN_TEST_STATE/retried ) ]]; then
  touch "$SIGN_TEST_STATE/retried"
  head -c 524288 /dev/zero | tr '\0' x >&2
  exit 17
fi
[[ -z $bundle ]] || printf '{"test_double":true}\n' > "$bundle"
case $command in
  attest) cp -- "$predicate" "$SIGN_TEST_STATE/$role.$kind.json";;
  verify-attestation)
    if [[ ${SIGN_TEST_MODE:-} = predicate-mismatch ]]; then
      printf '{"different_predicate":true}\n' > "$SIGN_TEST_STATE/$role.$kind.json"
    fi
    digest=${reference##*@sha256:}
    [[ ${SIGN_TEST_MODE:-} != subject-mismatch ]] || digest=wrong
    jq -n --slurpfile predicate "$SIGN_TEST_STATE/$role.$kind.json" --arg digest "$digest" '
      {payloadType:"application/vnd.in-toto+json",payload:({predicate:$predicate[0],subject:[{digest:{sha256:$digest}}]} | tojson | @base64)}'
    exit 0;;
esac
head -c 524288 /dev/zero | tr '\0' x
SH
cat > "$temporary/bin/syft" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
printf 'sbom\t%s\n' "$1" >> "$SIGN_TEST_CALLS"
[[ $2 = -o && $3 = spdx-json=* ]]
[[ $4 = --select-catalogers && $5 = +cargo-auditable-binary-cataloger ]]
[[ $(ulimit -S -f) = 8388608 && $(ulimit -H -f) = 8388608 ]]
truncate -s 135266304 "$SIGN_TEST_STATE/syft-layer"
[[ $(stat --format=%s "$SIGN_TEST_STATE/syft-layer") = 135266304 ]]
rm "$SIGN_TEST_STATE/syft-layer"
role=${1##*/tika-indra-}
role=${role%@*}
packages='[]'
case $role in
  native|gateway|bridge)
    # Static Rust images are expected to expose cargo-auditable data as cargo purls.
    [[ ${SIGN_TEST_MODE:-} = empty-rust-sbom ]] || packages='[{"SPDXID":"SPDXRef-Package-rust-crate-tokio-1","name":"tokio","versionInfo":"1.53.3","externalRefs":[{"referenceCategory":"PACKAGE-MANAGER","referenceType":"purl","referenceLocator":"pkg:cargo/tokio@1.53.3"}]}]'
    [[ ${SIGN_TEST_MODE:-} != spdxid-collision ]] || packages='[{"SPDXID":"SPDXRef-Static-native-glibc","name":"tokio","versionInfo":"1.53.3","externalRefs":[{"referenceCategory":"PACKAGE-MANAGER","referenceType":"purl","referenceLocator":"pkg:cargo/tokio@1.53.3"}]}]';;
  *) packages='[{"SPDXID":"SPDXRef-Package-deb-libc6-1","name":"libc6","versionInfo":"2.41","externalRefs":[{"referenceCategory":"PACKAGE-MANAGER","referenceType":"purl","referenceLocator":"pkg:deb/debian/libc6@2.41"}]}]';;
esac
# Like Syft, the document describes one image root that contains the packages.
jq -n --argjson packages "$packages" --arg role "$role" --arg mode "${SIGN_TEST_MODE:-}" '
  ("SPDXRef-DocumentRoot-Image-tika-indra-" + $role) as $root |
  {spdxVersion:"SPDX-2.3",test_double:true,creationInfo:{creators:["Organization: Anchore, Inc","Tool: syft-1.54.1"]},
   packages:([{SPDXID:$root,name:("ghcr.io/example/tika-indra-" + $role)}] + $packages),
   relationships:([{spdxElementId:"SPDXRef-DOCUMENT",relationshipType:"DESCRIBES",relatedSpdxElement:$root} | select($mode != "no-root")] +
     [$packages[] | {spdxElementId:$root,relationshipType:"CONTAINS",relatedSpdxElement:.SPDXID}])}
' > "${3#spdx-json=}"
case ${SIGN_TEST_MODE:-} in
  oversized-stdout) truncate -s 135266304 /proc/self/fd/1;;
  oversized-stderr) truncate -s 135266304 /proc/self/fd/2;;
  oversized-spdx) truncate -s 135266304 "${3#spdx-json=}";;
  missing-spdx) rm "${3#spdx-json=}";;
esac
SH
chmod +x "$temporary/bin/"*
export PATH="$temporary/bin:$PATH"
jq -n --arg repo "$GITHUB_REPOSITORY" --arg sha "$GITHUB_SHA" --arg ref "$GITHUB_REF" --arg workflow "$GITHUB_WORKFLOW_REF" '
  {schema_version:1,repository:$repo,source_sha:$sha,source_ref:$ref,release_run_id:"42",release_run_attempt:"1",
   workflow_ref:$workflow,workflow_identity:("https://github.com/"+$workflow),oidc_issuer:"https://token.actions.githubusercontent.com",
   images:["native","gateway","bridge","tika","tesseract","backend"] | map(
     . as $role | {role:$role,digest:("sha256:" + ("0" * 64)),reference:("ghcr.io/example/tika-indra-"+$role+"@sha256:"+("0" * 64))})}' \
  > "$temporary/candidates.json"
jq -n --arg sha "$GITHUB_SHA" '{schema_version:1,source_sha:$sha,release_run_id:"42",release_run_attempt:"1",
  evidence_manifest:{sha256:("0"*64),bytes:1},verification:{passed:true}}' > "$temporary/predicate.json"
# The release's prepare step copies each Rust image's /relink here. The
# fixtures are component documents in the build's format.
fixtures="$root/tests/fixtures/static-components"
for role in native gateway bridge; do
  mkdir -p "$temporary/materials/$role"
  cp "$fixtures/$role.spdx.json" "$temporary/materials/$role/static-components.spdx.json"
done

run_case() {
  local name=$1 expected=$2 materials=${3:-$temporary/materials} candidates=${4:-$temporary/candidates.json} status=0
  : > "$SIGN_TEST_CALLS"
  rm -f "$SIGN_TEST_STATE/retried"
  SIGN_TEST_MODE="$name" bash "$root/scripts/sign-images.sh" "$candidates" "$temporary/$name" \
    "$temporary/predicate.json" "$materials" > "$temporary/$name.console" 2> "$temporary/$name.stderr" || status=$?
  [[ $status = "$expected" ]]
}
run_case pass 0
[[ $(wc -l < "$SIGN_TEST_CALLS") = 44 && $(wc -c < "$temporary/pass.console") -lt 8192 ]]
for role in native gateway bridge tika tesseract backend; do
  [[ $(wc -l < "$temporary/pass/$role/steps.tsv") = 8 ]]
  [[ $(wc -c < "$temporary/pass/$role/sign.attempt-1.stdout") = 524288 ]]
done
jq -e '.passed and .images == 6' "$temporary/pass/result.json" > /dev/null
# Each image attests its scan plus the build's static components of every
# static executable it ships, under the scanned image root, and keeps the scan
# beside it. Tika and Tesseract ship the gateway and the bridge. The backend
# attests the scan.
for role in native gateway bridge tika tesseract; do
  case $role in tika|tesseract) executables=(gateway bridge);; *) executables=("$role");; esac
  documents=()
  for executable in "${executables[@]}"; do documents+=("$fixtures/$executable.spdx.json"); done
  jq -en --slurpfile scan "$temporary/pass/$role/syft.spdx.json" --slurpfile attested "$SIGN_TEST_STATE/$role.sbom.json" \
    --arg root "SPDXRef-DocumentRoot-Image-tika-indra-$role" '
    [inputs.packages[]] as $static | $scan[0] as $scan | $attested[0] |
    .packages == $scan.packages + $static and
    .relationships == $scan.relationships +
      [$static[] | {spdxElementId:$root,relationshipType:"CONTAINS",relatedSpdxElement:.SPDXID}] and
    .creationInfo.creators == $scan.creationInfo.creators + ["Tool: runtime-static-components.sh"] and
    del(.packages, .relationships, .creationInfo.creators) == ($scan | del(.packages, .relationships, .creationInfo.creators))
  ' "${documents[@]}" > /dev/null
  cmp "$temporary/pass/$role/sbom.spdx.json" "$SIGN_TEST_STATE/$role.sbom.json"
  jq -e 'all(.packages[]; .SPDXID | startswith("SPDXRef-Static-") | not)' "$temporary/pass/$role/syft.spdx.json" > /dev/null
done
[[ ! -e $temporary/pass/backend/syft.spdx.json ]]
cmp "$temporary/pass/backend/sbom.spdx.json" "$SIGN_TEST_STATE/backend.sbom.json"
# Signing Tika first shows that its own gate checks each embedded executable.
jq '.images |= map(select(.role == "tika")) + map(select(.role != "tika"))' "$temporary/candidates.json" > "$temporary/tika-first.json"
# Missing, foreign, incomplete or colliding static components block attestation.
static_case() {
  local name=$1 role=$2 source=$3 filter=$4 message=$5 attested=$6 candidates=${7:-$temporary/candidates.json}
  cp -a "$temporary/materials" "$temporary/$name.materials"
  rm "$temporary/$name.materials/$role/static-components.spdx.json"
  [[ $filter = absent ]] || jq "$filter" "$fixtures/$source.spdx.json" > "$temporary/$name.materials/$role/static-components.spdx.json"
  run_case "$name" 2 "$temporary/$name.materials" "$candidates"
  grep -Fq -- "$message" "$temporary/$name.stderr"
  [[ $(awk -F '\t' '$1 == "attest" {n++} END {print n+0}' "$SIGN_TEST_CALLS") = "$attested" ]]
  [[ ! -e $temporary/$name/result.json ]]
}
static_case missing-static native native absent 'native: the native static component SBOM is missing from the verified materials' 0
static_case foreign-static native bridge . 'not the SPDX 2.3 document of its executable' 0
static_case incomplete-static native native 'del(.packages[] | select(.name == "libtiff"))' \
  'native SBOM lacks static components of the native executable: ^pkg:generic/libtiff@' 0
static_case native-without-runtime native native 'del(.packages[] | select(.name | startswith("libstdc++") or startswith("libgcc")))' \
  'native SBOM lacks static components of the native executable: ^pkg:deb/debian/libgcc-[0-9]+-dev@ ^pkg:deb/debian/libstdc%2B%2B-[0-9]+-dev@' 0
# Native passed and was attested; the gateway then stops the run.
static_case gateway-without-glibc gateway gateway 'del(.packages[] | select(.name == "libc6-dev"))' \
  'gateway SBOM lacks static components of the gateway executable: ^pkg:deb/debian/libc6-dev@' 2
[[ ! -e $temporary/gateway-without-glibc/bridge ]]
# The gateway's glibc does not stand in for the bridge's.
static_case tika-bridge-without-glibc bridge bridge 'del(.packages[] | select(.name == "libc6-dev"))' \
  'tika SBOM lacks static components of the bridge executable: ^pkg:deb/debian/libc6-dev@' 0 "$temporary/tika-first.json"
static_case tika-without-bridge bridge bridge absent \
  'tika: the bridge static component SBOM is missing from the verified materials' 0 "$temporary/tika-first.json"
static_case tika-colliding-sets bridge gateway '.name = "static-components-bridge"' \
  'collides with another package' 0 "$temporary/tika-first.json"
run_case spdxid-collision 2
grep -Fq 'cannot be merged into the scanned SBOM' "$temporary/spdxid-collision.stderr"
grep -Fq 'collides with another package' "$temporary/spdxid-collision.stderr"
run_case no-root 2
grep -Fq 'does not describe exactly one root' "$temporary/no-root.stderr"
[[ $(awk -F '\t' '$1 == "attest" {n++} END {print n+0}' "$SIGN_TEST_CALLS") = 0 ]]
for output in stdout stderr spdx; do
  run_case "oversized-$output" 2
  [[ $(awk -F '\t' '$1 == "attest" {n++} END {print n+0}' "$SIGN_TEST_CALLS") = 0 ]]
  [[ ! -e $temporary/oversized-$output/gateway && ! -e $temporary/oversized-$output/result.json ]]
  path="$temporary/oversized-$output/native/sbom.attempt-1.$output"
  [[ $output != spdx ]] || path="$temporary/oversized-$output/native/sbom.spdx.json"
  [[ $(stat --format=%s "$path") = 135266304 ]]
  grep -q 'Missing or oversized SBOM evidence' "$temporary/oversized-$output.stderr"
  [[ $(awk -F '\t' '$1 == "sbom" {print $3}' "$temporary/oversized-$output/native/steps.tsv") = 2 ]]
done
for name in missing-spdx stat-failure; do
  run_case "$name" 2
  [[ $(awk -F '\t' '$1 == "attest" {n++} END {print n+0}' "$SIGN_TEST_CALLS") = 0 ]]
  [[ $(awk -F '\t' '$1 == "sbom" {print $3}' "$temporary/$name/native/steps.tsv") = 2 ]]
  grep -q 'Missing or oversized SBOM evidence' "$temporary/$name.stderr"
done
[[ ! -e $temporary/missing-spdx/native/sbom.spdx.json ]]
jq -e '.spdxVersion == "SPDX-2.3"' "$temporary/stat-failure/native/sbom.spdx.json" > /dev/null
# A Rust image whose SBOM lists no cargo packages must not be attested.
run_case empty-rust-sbom 2
[[ $(awk -F '\t' '$1 == "attest" {n++} END {print n+0}' "$SIGN_TEST_CALLS") = 0 ]]
grep -q 'names no Rust packages' "$temporary/empty-rust-sbom.stderr"
[[ ! -e $temporary/empty-rust-sbom/gateway ]]
run_case transient 0
[[ $(wc -l < "$SIGN_TEST_CALLS") = 45 && $(wc -c < "$temporary/transient/native/verify.attempt-1.stderr") = 524288 ]]
[[ $(wc -c < "$temporary/transient.stderr") -le 4096 ]]
run_case verify-failure 17
[[ $(awk -F '\t' '$1 == "verify" {n++} END {print n}' "$SIGN_TEST_CALLS") = 3 ]]
[[ ! -e $temporary/verify-failure/gateway && ! -e $temporary/verify-failure/candidates.sigstore.json ]]
[[ $(wc -c < "$temporary/verify-failure.stderr") -le 12288 ]]
run_case write-failure 17
[[ $(awk -F '\t' '$1 == "attest" {n++} END {print n}' "$SIGN_TEST_CALLS") = 1 ]]
[[ ! -e $temporary/write-failure/native/verify.attempt-1.stdout ]]
run_case predicate-mismatch 2
run_case subject-mismatch 2
run_case hang 124
[[ ! -e $temporary/hang/native/sbom.spdx.json ]]
cp "$temporary/candidates.json" "$temporary/original.json"
for mutation in '.images[0].reference = "ghcr.io/example/tika-indra-native:latest"' '.images[1] = .images[0]' '.release_run_attempt = "2"' '.source_sha = ("1"*40)'; do
  jq "$mutation" "$temporary/original.json" > "$temporary/candidates.json"
  status=0
  bash "$root/scripts/sign-images.sh" "$temporary/candidates.json" "$temporary/invalid" "$temporary/predicate.json" \
    "$temporary/materials" > /dev/null 2> /dev/null || status=$?
  [[ $status = 2 && ! -e $temporary/invalid ]]
done
cp "$temporary/original.json" "$temporary/candidates.json"
for materials in "$temporary/absent-materials" "$fixtures/native.spdx.json"; do
  status=0
  bash "$root/scripts/sign-images.sh" "$temporary/candidates.json" "$temporary/invalid" "$temporary/predicate.json" \
    "$materials" > /dev/null 2> "$temporary/materials.stderr" || status=$?
  [[ $status = 2 && ! -e $temporary/invalid ]]
  grep -Fq 'verified relink materials directory is required' "$temporary/materials.stderr"
done
printf 'Signing orchestration passed: six exact roles, bounded retries/timeouts/console, finite 8 GiB scanner files and 128 MiB evidence, full evidence, EOF stdin, exact predicates and subjects, no write retries, invalid identity rejection, merged and required static components in the five SBOMs of images with static executables.\n'
