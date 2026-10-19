#!/usr/bin/env bash
# Synthetic receipt/transport controls only: no signatures, images or inference.
# jq variables deliberately remain literal in filter programs.
# shellcheck disable=SC2016
set -euo pipefail
export LC_ALL=C
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
[[ $# -le 1 ]] || { echo 'usage: current-http-reference.sh [NEW_EVIDENCE_DIRECTORY]' >&2; exit 2; }
if [[ $# = 1 ]]; then
 [[ ! -e $1 && ! -L $1 ]] || exit 2
 mkdir -- "$1";owned=$(realpath -e -- "$1")
else
 owned=$(mktemp -d)
 trap 'rm -rf -- "$owned"' EXIT
fi
printf 'case\texpected\texit_status\n' > "$owned/cases.tsv"
mkdir -p "$owned/controller/scripts/lib" "$owned/bin"
cp "$root/scripts/admit-current-http-reference.sh" "$root/scripts/verify-native-http-reference.sh" "$owned/controller/scripts/"
cp "$root/scripts/lib/cgroup-admission.sh" "$owned/controller/scripts/lib/"
cat > "$owned/controller/scripts/verify-daemon-image.sh" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
[[ $# = 4 && $1 = bind && ! -e $4 ]]
printf '%s\n' "$2" >> "$ADMISSION_STATE/bindings"
[[ ${ADMISSION_MODE:-} != binding-failure ]] || exit 41
jq -e --slurpfile source "$2/result.json" 'length==1 and .[0].Image==$source[0].root_digest and .[0].ImageManifestDescriptor.digest==$source[0].platform_digest' "$3" > /dev/null
mkdir "$4"
cp "$2/result.json" "$4/result.json"
if [[ ${ADMISSION_MODE:-} = bound-config ]]; then jq '.config_digest="sha256:"+("f"*64)' "$4/result.json" > "$4/change";mv "$4/change" "$4/result.json";fi
STUB
cat > "$owned/bin/docker" <<'STUB'
#!/usr/bin/env bash
printf 'unexpected Docker invocation\n' >> "$ADMISSION_STATE/docker-called"
exit 92
STUB
cat > "$owned/bin/git" <<'STUB'
#!/usr/bin/env bash
[[ $# = 4 && $1 = -C && $3 = rev-parse && $4 = HEAD ]]
printf '%040d\n' 42
STUB
chmod +x "$owned/bin/"*
export PATH="$owned/bin:$PATH"
sha() { sha256sum -- "$1" | cut -d' ' -f1; }
digest() { printf 'sha256:%064d' "$1"; }
source_sha=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
image=$(digest 90);native_root=$(digest 1);platform=$(digest 11);config=$(digest 21)
# File names are the admission packet's relative-path public contract.
pairs=('candidates candidates.json' 'index_bundle bundle.json' 'signing_result signing.json' 'index_steps index.tsv' 'native_steps native.tsv' 'native_signature signature.json' 'native_sbom sbom.json' 'native_evidence evidence.json' 'native_provenance provenance.json' 'release_manifest release.sha256' 'native_resolution native-resolution/result.json' 'native_container native-container.json' 'assembled_resolution assembled-resolution/result.json' 'assembled_container assembled-container.json' 'assembly assembly.json' 'materials materials/SHA256SUMS' 'build_identity materials/build-identity.txt' 'native_binary native.bin')
refresh_materials() {
 (cd "$state/materials" && sha256sum build-identity.txt closure.txt | sed 's/  /  .\//') > "$state/materials/SHA256SUMS"
 printf '%s  ./bin/indra-server\n%s  ./materials/native/SHA256SUMS\n' "$(sha "$state/native.bin")" "$(sha "$state/materials/SHA256SUMS")" > "$state/release.sha256"
}
refresh_evidence() {
 jq -nc --arg source "$source_sha" --arg digest "${native_root#sha256:}" --arg hash "$(sha "$state/release.sha256")" --argjson bytes "$(stat -c%s "$state/release.sha256")" '{_type:"https://in-toto.io/Statement/v0.1",predicateType:"https://github.com/somehowchris/tika-indra-ocr-client/attestations/release-evidence/v1",subject:[{digest:{sha256:$digest}}],predicate:{source_sha:$source,release_run_id:"123",release_run_attempt:"1",verification:{passed:true},evidence_manifest:{sha256:$hash,bytes:$bytes}}}' > "$state/statement.json"
 jq -nc --arg payload "$(base64 -w0 "$state/statement.json")" '{payloadType:"application/vnd.in-toto+json",payload:$payload}' > "$state/evidence.json"
}
refresh_identity() {
 jq -nc --arg source "$source_sha" --arg image "$image" --arg native "$(sha "$state/native.bin")" --arg materials "$(sha "$state/materials/SHA256SUMS")" '{source_sha:$source,assembled_image_root:$image,role_sha256:{native:$native},native_materials_sha256:$materials,native_equivalence_passed:false}' > "$state/identity.json"
 jq -n --slurpfile index "$state/candidates.json" --arg source "$source_sha" --arg image "$image" --arg native "$(sha "$state/native.bin")" --arg materials "$(sha "$state/materials/SHA256SUMS")" '{schema_version:1,source_sha:$source,assembled_image_root:$image,native_sha256:$native,native_materials_sha256:$materials,parents:($index[0].images|map(select(.role=="native" or .role=="gateway" or .role=="bridge" or .role=="tika")|{key:.role,value:{reference:.reference,root_digest:.digest}})|from_entries)}' > "$state/assembly.json"
}
packet() {
 for pair in "${pairs[@]}";do
  read -r key file <<< "$pair"
  jq -nc --arg key "$key" --arg path "$file" --arg hash "$(sha "$state/$file")" '{key:$key,value:{path:$path,sha256:$hash}}'
 done | jq -s from_entries > "$state/files.json"
 jq -n --arg source "$source_sha" --arg image "$image" --arg native "$(sha "$state/native.bin")" --arg models "$(sha "$state/models.sha256")" --slurpfile files "$state/files.json" '{schema_version:1,source_sha:$source,assembled_image_root:$image,native_sha256:$native,models_inventory_sha256:$models,files:$files[0]}' > "$state/admission.json"
}
base() {
 state="$owned/$1";mkdir -p "$state/materials" "$state/models" "$state/native-resolution" "$state/assembled-resolution"
 export ADMISSION_STATE="$state" ADMISSION_MODE="$1"
 printf 'synthetic native witness\n' > "$state/native.bin"
 printf 'binary_sha256=%s\n' "$(sha "$state/native.bin")" > "$state/materials/build-identity.txt"
 printf 'synthetic native archive/header closure\n' > "$state/materials/closure.txt"
 printf 'synthetic model bytes\n' > "$state/models/model.bin"
 (cd "$state/models" && find . -type f -print0|sort -z|xargs -0 sha256sum) > "$state/models.sha256"
 printf '{"test_double":"bundle; no cryptographic admission claim"}\n' > "$state/bundle.json"
 printf 'stage\tattempt\texit_status\telapsed_seconds\nverify-blob\t1\t0\t1\n' > "$state/index.tsv"
 printf 'stage\tattempt\texit_status\telapsed_seconds\nverify\t1\t0\t1\nverify-sbom\t1\t0\t1\nverify-evidence\t1\t0\t1\n' > "$state/native.tsv"
 printf '{"schema_version":1,"passed":true,"images":6,"claims":["candidate-index-blob"]}\n' > "$state/signing.json"
 jq -nc --arg source "$source_sha" '["native","gateway","bridge","tika","tesseract","backend"]|to_entries|map({role:.value,digest:("sha256:"+((.key+1)|tostring|"0"*(64-length)+.)),reference:("ghcr.io/somehowchris/tika-indra-"+.value+"@sha256:"+((.key+1)|tostring|"0"*(64-length)+.))}) as $images|{schema_version:1,repository:"somehowchris/tika-indra-ocr-client",source_sha:$source,source_ref:"refs/tags/v-test",release_run_id:"123",release_run_attempt:"1",workflow_ref:"somehowchris/tika-indra-ocr-client/.github/workflows/release.yml@refs/tags/v-test",workflow_identity:"https://github.com/somehowchris/tika-indra-ocr-client/.github/workflows/release.yml@refs/tags/v-test",oidc_issuer:"https://token.actions.githubusercontent.com",images:$images}' > "$state/candidates.json"
 jq -nc --arg root "$native_root" '[{critical:{image:{"docker-manifest-digest":$root}}}]' > "$state/signature.json"
 jq -nc --arg digest "${native_root#sha256:}" '{_type:"https://in-toto.io/Statement/v1",predicateType:"https://spdx.dev/Document",subject:[{digest:{sha256:$digest}}],predicate:{SPDXID:"SPDXRef-DOCUMENT",dataLicense:"CC0-1.0",spdxVersion:"SPDX-2.3",name:"synthetic-native-fixture",documentNamespace:"https://example.invalid/fixture"}}' > "$state/sbom-statement.json"
 jq -nc --arg payload "$(base64 -w0 "$state/sbom-statement.json")" '{payloadType:"application/vnd.in-toto+json",payload:$payload}' > "$state/sbom.json"
 jq -nc --arg reference "ghcr.io/somehowchris/tika-indra-native@$native_root" --arg platform "$platform" --arg config "$config" '{passed:true,reference:$reference,platform_digest:$platform,config_digest:$config}' > "$state/provenance.json"
 for role in native assembled;do
  expected=$native_root;[[ $role != assembled ]] || expected=$image
  jq -nc --arg root "$expected" --arg platform "$platform" --arg config "$config" '{passed:true,store:"containerd",image_id:$root,root_digest:$root,platform_digest:$platform,config_digest:$config,os:"linux",architecture:"amd64"}' > "$state/$role-resolution/result.json"
  jq -nc --arg root "$expected" --arg platform "$platform" '[{Image:$root,ImageManifestDescriptor:{digest:$platform}}]' > "$state/$role-container.json"
 done
 jq -nc --arg source "$source_sha" '{config:{Labels:{"org.opencontainers.image.revision":$source}}}' > "$state/native-resolution/config.json"
 refresh_materials;refresh_evidence;refresh_identity;packet
}
mutate() { local file=$1 filter=$2;jq "$filter" "$state/$file" > "$state/change";mv "$state/change" "$state/$file"; }
checks=0
run() {
 local expected=$1 status=0
 bash "$owned/controller/scripts/admit-current-http-reference.sh" "$state/admission.json" "$state/identity.json" "$image" "$state/models" "$state/output" > "$state/stdout" 2> "$state/stderr" || status=$?
 if [[ $expected = pass ]];then
  [[ $status = 0 && $(wc -l < "$state/bindings") = 2 ]]
  jq -e --arg source "$source_sha" '.admitted==true and .release_source_sha==$source and .controller_source_sha!=$source' "$state/output/result.json" > /dev/null
 else [[ $status != 0 && ! -e $state/output/result.json ]];fi
 [[ ! -e $state/docker-called ]]
 printf '%s\t%s\t%s\n' "${state##*/}" "$expected" "$status" >> "$owned/cases.tsv"
 checks=$((checks+1))
}
base pass;run pass
base corrupt-receipt;printf 'changed\n' >> "$state/signature.json";run fail
base missing-receipt;rm "$state/bundle.json";run fail
base boolean-only;printf 'stage\tattempt\texit_status\telapsed_seconds\n' > "$state/index.tsv";packet;run fail
base native-verification-failed;sed -i 's/verify-evidence\t1\t0/verify-evidence\t1\t1/' "$state/native.tsv";packet;run fail
base wrong-source;mutate candidates.json '.source_sha="bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"';packet;run fail
base wrong-signature;mutate signature.json '.[0].critical.image["docker-manifest-digest"]="sha256:"+("f"*64)';packet;run fail
base wrong-sbom-subject;mutate sbom-statement.json '.subject[0].digest.sha256="f"*64';jq -nc --arg payload "$(base64 -w0 "$state/sbom-statement.json")" '{payloadType:"application/vnd.in-toto+json",payload:$payload}' > "$state/sbom.json";packet;run fail
base wrong-sbom-document;mutate sbom-statement.json '.predicate.spdxVersion="not-SPDX"';jq -nc --arg payload "$(base64 -w0 "$state/sbom-statement.json")" '{payloadType:"application/vnd.in-toto+json",payload:$payload}' > "$state/sbom.json";packet;run fail
base wrong-sbom-type;mutate sbom-statement.json '.predicateType="https://example.invalid/other"';jq -nc --arg payload "$(base64 -w0 "$state/sbom-statement.json")" '{payloadType:"application/vnd.in-toto+json",payload:$payload}' > "$state/sbom.json";packet;run fail
base wrong-evidence;mutate statement.json '.subject[0].digest.sha256="f"*64';jq -nc --arg payload "$(base64 -w0 "$state/statement.json")" '{payloadType:"application/vnd.in-toto+json",payload:$payload}' > "$state/evidence.json";packet;run fail
base wrong-native-manifest;sed -i 's/^.\{64\}  \.\/bin\/indra-server/ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff  .\/bin\/indra-server/' "$state/release.sha256";refresh_evidence;packet;run fail
base broken-closure;printf 'corrupt\n' >> "$state/materials/closure.txt";run fail
base wrong-build-native;printf 'binary_sha256=%064d\n' 99 > "$state/materials/build-identity.txt";refresh_materials;refresh_evidence;refresh_identity;packet;run fail
base wrong-parent;mutate assembly.json '.parents.native.root_digest="sha256:"+("f"*64)';packet;run fail
base wrong-image-identity;mutate identity.json '.assembled_image_root="sha256:"+("f"*64)';run fail
base wrong-provenance;mutate provenance.json '.config_digest="sha256:"+("f"*64)';packet;run fail
base wrong-container;mutate native-container.json '.[0].Image="sha256:"+("f"*64)';packet;run fail
base binding-failure;run fail
base bound-config;run fail
base source-label;mutate native-resolution/config.json '.config.Labels["org.opencontainers.image.revision"]="b"*40';run fail
base changed-model;printf 'other\n' >> "$state/models/model.bin";run fail
base model-symlink;ln -s model.bin "$state/models/link";run fail
base receipt-symlink;mv "$state/provenance.json" "$state/original-provenance";ln -s original-provenance "$state/provenance.json";run fail
base malformed-stream;printf 'true\n' >> "$state/candidates.json";packet;run fail
base absolute-path;mutate admission.json '.files.candidates.path="/tmp/not-a-packet-file"';run fail
# The opt-in wrapper must reach admission before protocol/client/Docker work.
base mode-hook
cat > "$owned/controller/scripts/admit-current-http-reference.sh" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$@" > "$ADMISSION_STATE/hook-arguments"
exit 73
STUB
mkdir "$state/reference" "$state/inputs" "$state/client-build"
status=0
bash "$owned/controller/scripts/verify-native-http-reference.sh" --current-admission "$state/admission.json" /bin/true "$state/identity.json" "$image" "$state/models" "$state/inputs" "$state/reference" "$state/client-build" "$state/wrapper-output" > "$state/wrapper.stdout" 2> "$state/wrapper.stderr" || status=$?
[[ $status = 73 && $(wc -l < "$state/hook-arguments") = 5 && ! -e $state/docker-called ]]
printf 'mode-hook\tfail-before-Docker\t%s\n' "$status" >> "$owned/cases.tsv"
checks=$((checks+1))
# Historical invocation still rejects a current image using its original pins.
rm "$state/hook-arguments";status=0
bash "$owned/controller/scripts/verify-native-http-reference.sh" /bin/true "$state/identity.json" "$image" "$state/models" "$state/inputs" "$state/reference" "$state/client-build" "$state/historical-output" > "$state/legacy.stdout" 2> "$state/legacy.stderr" || status=$?
[[ $status != 0 && ! -e $state/hook-arguments && ! -e $state/docker-called ]]
printf 'historical-pins\tfail-before-Docker\t%s\n' "$status" >> "$owned/cases.tsv"
checks=$((checks+1))
printf 'Current HTTP reference: %s synthetic controls passed; delegated daemon contracts and fake transports, no signature/image/model-inference execution.\n' "$checks"
