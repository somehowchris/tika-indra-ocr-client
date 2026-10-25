#!/usr/bin/env bash
# Test actual acceptance/rejection boundaries without a daemon or cluster.
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT
mkdir "$work/proof"
id=$(printf '1%.0s' {1..64})
uid=11111111-2222-3333-4444-555555555555
config="sha256:$(printf 'a%.0s' {1..64})"
wrong="sha256:$(printf 'b%.0s' {1..64})"
root_digest="sha256:$(printf 'c%.0s' {1..64})"
platform_digest="sha256:$(printf 'd%.0s' {1..64})"
jq -n --arg config "$config" --arg root "$root_digest" --arg platform "$platform_digest" \
    '{schema_version:1,passed:true,os:"linux",architecture:"amd64",root_digest:$root,platform_digest:$platform,config_digest:$config}' > "$work/proof/result.json"
jq -n --arg id "$id" --arg uid "$uid" --arg config "$config" '{status:{id:$id,labels:{"io.kubernetes.pod.uid":$uid},imageId:$config,imageRef:"untrusted-display-reference"},info:{config:{image:{image:$config}}}}' > "$work/container.json"
jq -n --arg config "$config" '{status:{id:$config},info:{imageSpec:{os:"linux",architecture:"amd64"}}}' > "$work/image.json"
bash "$root/scripts/verify-cri-image.sh" "$work/proof" "$work/container.json" "$work/image.json" "$id" "$uid" "$work/current"
jq -e '.passed == true and .observed_image_ref == "untrusted-display-reference"' "$work/current/result.json" > /dev/null
# Older containerd retains the immutable creation ID but lacks status.imageId.
jq 'del(.status.imageId)' "$work/container.json" > "$work/older.json"
bash "$root/scripts/verify-cri-image.sh" "$work/proof" "$work/older.json" "$work/image.json" "$id" "$uid" "$work/older"
# Current crictl emits default string values even when older runtimes omit them.
jq '.status.imageId = ""' "$work/container.json" > "$work/older-default.json"
bash "$root/scripts/verify-cri-image.sh" "$work/proof" "$work/older-default.json" "$work/image.json" "$id" "$uid" "$work/older-default"
jq -e '.runtime_image_id == "" and .runtime_image_id_available == false' "$work/older-default/result.json" > /dev/null
reject_container() {
    local name=$1 expression=$2
    jq --arg wrong "$wrong" --arg root "$root_digest" --arg platform "$platform_digest" \
        "$expression" "$work/container.json" > "$work/rejected.json"
    if bash "$root/scripts/verify-cri-image.sh" "$work/proof" "$work/rejected.json" "$work/image.json" "$id" "$uid" "$work/$name"; then
        echo "incorrectly accepted $name" >&2; exit 1
    fi
}
# $wrong is a jq variable supplied by reject_container, not a shell expansion.
# shellcheck disable=SC2016
reject_container changed-creation '.info.config.image.image = $wrong'
reject_container tag-as-creation '.info.config.image.image = "tika-indra:dev"'
# shellcheck disable=SC2016
reject_container changed-runtime '.status.imageId = $wrong'
# Each descriptor level must remain distinct; comparing to a root/manifest is wrong.
# shellcheck disable=SC2016
reject_container root-as-creation '.info.config.image.image = $root'
# shellcheck disable=SC2016
reject_container manifest-as-creation '.info.config.image.image = $platform'
reject_container missing-creation 'del(.info.config.image.image)'
reject_container null-runtime '.status.imageId = null'
reject_container changed-container '.status.id = "2222222222222222222222222222222222222222222222222222222222222222"'
reject_container changed-pod '.status.labels["io.kubernetes.pod.uid"] = "99999999-2222-3333-4444-555555555555"'
jq --arg wrong "$wrong" '.status.id = $wrong' "$work/image.json" > "$work/changed-image.json"
if bash "$root/scripts/verify-cri-image.sh" "$work/proof" "$work/container.json" "$work/changed-image.json" "$id" "$uid" "$work/changed-image"; then exit 1; fi
jq '.info.imageSpec.architecture = "arm64"' "$work/image.json" > "$work/wrong-platform.json"
if bash "$root/scripts/verify-cri-image.sh" "$work/proof" "$work/container.json" "$work/wrong-platform.json" "$id" "$uid" "$work/wrong-platform"; then exit 1; fi
if bash "$root/scripts/verify-cri-image.sh" "$work/proof" "$work/container.json" "$work/image.json" "$id" "$uid" "$work/current"; then exit 1; fi
printf '%s\n' 'CRI config identity fixtures passed: current/older runtimes; wrong identity, UID, platform and overwrite rejected.'
