#!/usr/bin/env bash
# Offline binding of an externally verified signed release to a new HTTP replay.
# Signature authentication remains the caller's retained Cosign admission, not a
# passed boolean in this packet. No Docker/network/model command is run here.
# jq variables deliberately remain literal in filter programs.
# shellcheck disable=SC2016
set -euo pipefail
export LC_ALL=C
fail() { printf 'current HTTP reference admission: %s\n' "$*" >&2; exit 1; }
[[ $# = 5 ]] || fail 'usage: admit-current-http-reference.sh ADMISSION_JSON IDENTITY_JSON IMAGE MODELS NEW_OUTPUT'
[[ ! -L $1 && ! -L $2 && ! -L $4 ]] || fail 'symlink input rejected'
packet=$(realpath -e -- "$1"); identity=$(realpath -e -- "$2"); image=$3; models=$(realpath -e -- "$4"); output=$(realpath -m -- "$5")
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
sha() { sha256sum -- "$1" | cut -d' ' -f1; }
regular() { [[ -f $1 && ! -L $1 && $(stat -c %s -- "$1") -gt 0 && $(stat -c %s -- "$1") -le $2 ]]; }
json() { regular "$1" 4194304 && jq -e -s 'length==1' "$1" > /dev/null; }
json "$packet" && json "$identity" || fail 'bounded single JSON input required'
[[ ! -e $output && ! -L $output && $image =~ ^sha256:[a-f0-9]{64}$ ]] || fail 'immutable image and new output required'
jq -e '
 .schema_version==1 and (.source_sha|test("^[a-f0-9]{40}$")) and
 (.assembled_image_root|test("^sha256:[a-f0-9]{64}$")) and (.native_sha256|test("^[a-f0-9]{64}$")) and
 (.models_inventory_sha256|test("^[a-f0-9]{64}$")) and
 (.files|keys|sort)==["assembled_container","assembled_resolution","assembly","build_identity","candidates","index_bundle","index_steps","materials","native_binary","native_container","native_evidence","native_provenance","native_resolution","native_sbom","native_signature","native_steps","release_manifest","signing_result"] and
 all(.files[]; keys==["path","sha256"] and (.path|type=="string" and test("^[A-Za-z0-9_./-]+$") and (startswith("/")|not)) and (.sha256|test("^[a-f0-9]{64}$")))
' "$packet" > /dev/null || fail 'admission schema differs'
# Resolve only regular files within the packet directory. Each witness is bound
# by actual bytes before its existing JSON/TSV contract is inspected.
declare -A files
base=$(dirname -- "$packet")
while IFS=$'\t' read -r name relative expected; do
 [[ /$relative/ != *'/../'* && /$relative/ != *'/./'* && $relative != . && $relative != .. ]]
 path=$(realpath -e -- "$base/$relative")
 [[ $path = "$base/"* && ! -L $base/$relative ]] || fail 'receipt escapes packet or is a symlink'
 limit=4194304; [[ $name != native_binary ]] || limit=536870912
 regular "$path" "$limit" && [[ $(sha "$path") = "$expected" ]] || fail "receipt bytes differ: $name"
 files[$name]=$path
done < <(jq -r '.files|to_entries[]|[.key,.value.path,.value.sha256]|@tsv' "$packet")
for name in candidates signing_result native_provenance native_resolution assembled_resolution assembly; do
 json "${files[$name]}" || fail "single JSON witness required: $name"
done
source_sha=$(jq -er .source_sha "$packet"); server_sha=$(jq -er .native_sha256 "$packet")
[[ $(jq -er .assembled_image_root "$packet") = "$image" && $(sha "${files[native_binary]}") = "$server_sha" ]] || fail 'image/native witness differs'
index=${files[candidates]}
jq -e --arg source "$source_sha" '
 .schema_version==1 and .repository=="somehowchris/tika-indra-ocr-client" and .source_sha==$source and
 (.release_run_id|tostring|test("^[0-9]+$")) and (.release_run_attempt|tostring|test("^[1-9][0-9]*$")) and
 (.source_ref|test("^refs/(heads|tags)/[A-Za-z0-9_./-]+$")) and
 .workflow_ref==(.repository+"/.github/workflows/release.yml@"+.source_ref) and
 .workflow_identity==("https://github.com/"+.workflow_ref) and .oidc_issuer=="https://token.actions.githubusercontent.com" and
 ([.images[].role]|sort)==["backend","bridge","gateway","native","tesseract","tika"] and
 all(.images[]; (.digest|test("^sha256:[a-f0-9]{64}$")) and .reference==("ghcr.io/somehowchris/tika-indra-"+.role+"@"+.digest))
' "$index" > /dev/null || fail 'signed candidate coordinates differ'
native_ref=$(jq -er '.images[]|select(.role=="native")|.reference' "$index")
native_root=${native_ref##*@}
jq -e '.schema_version==1 and .passed==true and .images==6 and (.claims|index("candidate-index-blob"))!=null' "${files[signing_result]}" > /dev/null || fail 'completed signing result absent'
step() { awk -F '\t' -v stage="$2" 'NR>1 && $1==stage && $2~/^[1-9][0-9]*$/ && $3=="0" {ok=1} END{exit !ok}' "$1"; }
step "${files[index_steps]}" verify-blob || fail 'successful external index verification absent'
for stage in verify verify-sbom verify-evidence; do step "${files[native_steps]}" "$stage" || fail "external native $stage receipt absent"; done
jq -e -s --arg digest "$native_root" 'any(.[]; (if type=="array" then .[] else . end)|.critical.image["docker-manifest-digest"]==$digest)' "${files[native_signature]}" > /dev/null || fail 'verified native signature subject differs'
jq -e -s --arg digest "${native_root#sha256:}" '
 any(.[]; .payloadType=="application/vnd.in-toto+json" and
  (.payload|@base64d|fromjson| (._type=="https://in-toto.io/Statement/v0.1" or ._type=="https://in-toto.io/Statement/v1") and
   .predicateType=="https://spdx.dev/Document" and (.subject|length)==1 and .subject[0].digest.sha256==$digest and
   (.predicate| .SPDXID=="SPDXRef-DOCUMENT" and .dataLicense=="CC0-1.0" and
    (.spdxVersion|test("^SPDX-2\\.[0-9]+$")) and (.name|type=="string" and length>0) and
    (.documentNamespace|type=="string" and length>0))))
' "${files[native_sbom]}" > /dev/null || fail 'verified native SPDX subject/document differs'
manifest_sha=$(sha "${files[release_manifest]}")
jq -e -s --slurpfile index "$index" --arg digest "${native_root#sha256:}" --arg manifest "$manifest_sha" --argjson manifest_bytes "$(stat -c %s -- "${files[release_manifest]}")" '
 any(.[]; .payloadType=="application/vnd.in-toto+json" and
  (.payload|@base64d|fromjson| (._type=="https://in-toto.io/Statement/v0.1" or ._type=="https://in-toto.io/Statement/v1") and
   .predicateType=="https://github.com/somehowchris/tika-indra-ocr-client/attestations/release-evidence/v1" and any(.subject[];.digest.sha256==$digest) and
   .predicate.source_sha==$index[0].source_sha and .predicate.release_run_id==$index[0].release_run_id and
   .predicate.release_run_attempt==$index[0].release_run_attempt and .predicate.verification.passed==true and
   .predicate.evidence_manifest.sha256==$manifest and .predicate.evidence_manifest.bytes==$manifest_bytes))
' "${files[native_evidence]}" > /dev/null || fail 'verified native evidence subject/source/manifest differs'
listed() { [[ $(awk -v path="$2" '$2==path{print $1}' "$1") = "$3" ]]; }
listed "${files[release_manifest]}" ./bin/indra-server "$server_sha" || fail 'signed release does not bind this native ELF'
listed "${files[release_manifest]}" ./materials/native/SHA256SUMS "$(sha "${files[materials]}")" || fail 'signed release does not bind these native materials'
[[ ${files[build_identity]} = "$(dirname -- "${files[materials]}")/build-identity.txt" ]] || fail 'native build identity is outside materials'
listed "${files[materials]}" ./build-identity.txt "$(sha "${files[build_identity]}")" || fail 'native material build identity differs'
[[ $(sed -n 's/^binary_sha256=//p' "${files[build_identity]}") = "$server_sha" ]] || fail 'native build identity links another ELF'
(cd -- "$(dirname -- "${files[materials]}")" && sha256sum --check --status "$(basename -- "${files[materials]}")") || fail 'native material closure checksum failed'
jq -e --arg source "$source_sha" --arg image "$image" --arg native "$server_sha" --arg materials "$(sha "${files[materials]}")" '.source_sha==$source and .assembled_image_root==$image and .role_sha256.native==$native and .native_materials_sha256==$materials' "$identity" > /dev/null || fail 'telemetry identity differs'
jq -e --slurpfile index "$index" --arg source "$source_sha" --arg image "$image" --arg native "$server_sha" --arg materials "$(sha "${files[materials]}")" '
 .schema_version==1 and .source_sha==$source and .assembled_image_root==$image and .native_sha256==$native and .native_materials_sha256==$materials and
 (.parents|keys|sort)==["bridge","gateway","native","tika"] and
 all(.parents|to_entries[]; . as $p|any($index[0].images[];.role==$p.key and .reference==$p.value.reference and .digest==$p.value.root_digest))
' "${files[assembly]}" > /dev/null || fail 'approved four-parent assembly differs'
[[ -d $models && -z $(find "$models" -type l -print -quit) ]] || fail "model tree contains a symlink"
mkdir -- "$output"
(cd -- "$models" && find . -type f -print0 | sort -z | xargs -0 -r sha256sum) > "$output/models.sha256"
[[ $(sha "$output/models.sha256") = "$(jq -er .models_inventory_sha256 "$packet")" ]] || fail "model inventory differs"
cp -- "$packet" "$output/admission.json"
for role in native assembled; do
 resolution=${files[${role}_resolution]}; container=${files[${role}_container]}
 [[ $(basename -- "$resolution") = result.json ]] || fail 'daemon resolution must name existing result.json'
 bash "$root/scripts/verify-daemon-image.sh" bind "$(dirname -- "$resolution")" "$container" "$output/$role-binding"
 expected=$native_root; [[ $role != assembled ]] || expected=$image
 jq -e --arg root "$expected" --slurpfile original "$resolution" '.passed==true and .root_digest==$root and .image_id==$root and .platform_digest==$original[0].platform_digest and .config_digest==$original[0].config_digest' "$output/$role-binding/result.json" > /dev/null || fail 'actual daemon binding differs'
done
jq -e --arg reference "$native_ref" --slurpfile native "$output/native-binding/result.json" '.passed==true and .reference==$reference and .platform_digest==$native[0].platform_digest and .config_digest==$native[0].config_digest' "${files[native_provenance]}" > /dev/null || fail 'native provenance/daemon identity differs'
jq -e --arg source "$source_sha" '.config.Labels["org.opencontainers.image.revision"]==$source' "$(dirname -- "${files[native_resolution]}")/config.json" > /dev/null || fail 'native source label differs'
jq -n --arg source "$source_sha" --arg controller "$(git -C "$root" rev-parse HEAD)" --arg image "$image" --arg native "$server_sha" --arg packet "$(sha "$packet")" --arg models "$(jq -er .models_inventory_sha256 "$packet")" '{schema_version:1,admitted:true,release_source_sha:$source,controller_source_sha:$controller,assembled_image_root:$image,native_sha256:$native,models_inventory_sha256:$models,admission_sha256:$packet,scope:"Offline binding of externally verified signatures/provenance and actual native closure; HTTP equivalence remains unexecuted"}' > "$output/result.json"
